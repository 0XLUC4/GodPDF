use std::collections::BTreeMap;
use std::sync::Arc;

use lopdf::encryption::crypt_filters::{Aes256CryptFilter, CryptFilter};
use lopdf::{
    Document, EncryptionState, EncryptionVersion, LoadOptions, Permissions as PdfPermissions,
};

use crate::pdf::{load, save, save_raw};
use crate::{Error, Result};

/// Name of the single crypt filter used for strings and streams.
const CRYPT_FILTER: &[u8] = b"StdCF";

/// True when the trailer has an /Encrypt dictionary.
///
/// Also true for owner-password-only files (empty user password). Those open without a
/// password, so `decrypt(pdf, "")` succeeds and every other function (page_count, edits, ...)
/// works on them directly, but edits return an **unencrypted** result. Callers that want to
/// keep the protection must re-encrypt with [`crate::encrypt`] after editing.
pub fn is_encrypted(pdf: &[u8]) -> bool {
    if let Ok(meta) = Document::load_metadata_mem(pdf) {
        return meta.encrypted;
    }
    Document::load_mem(pdf).is_ok_and(|doc| doc.is_encrypted() || doc.was_encrypted())
}

/// Return an unencrypted copy. `Error::BadPassword` on wrong password.
///
/// An unencrypted input is returned re-serialized, unchanged in content.
pub fn decrypt(pdf: &[u8], password: &str) -> Result<Vec<u8>> {
    let mut doc = Document::load_mem_with_options(pdf, LoadOptions::with_password(password))
        .map_err(|err| match err {
            lopdf::Error::InvalidPassword => Error::BadPassword,
            other => Error::Parse(other.to_string()),
        })?;
    if doc.is_encrypted() {
        return Err(Error::BadPassword);
    }
    save(&mut doc)
}

/// What a reader may do with an encrypted document without its owner password.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Permissions {
    pub print: bool,
    /// Copy text and pictures (screen readers always may).
    pub copy: bool,
    /// Change the pages: their content, order and number.
    pub edit: bool,
    /// Add comments and fill in forms.
    pub annotate: bool,
}

impl Default for Permissions {
    fn default() -> Self {
        Self {
            print: true,
            copy: true,
            edit: true,
            annotate: true,
        }
    }
}

impl Permissions {
    fn to_pdf(self) -> PdfPermissions {
        let groups = [
            (
                self.print,
                PdfPermissions::PRINTABLE | PdfPermissions::PRINTABLE_IN_HIGH_QUALITY,
            ),
            (self.copy, PdfPermissions::COPYABLE),
            (
                self.edit,
                PdfPermissions::MODIFIABLE | PdfPermissions::ASSEMBLABLE,
            ),
            (
                self.annotate,
                PdfPermissions::ANNOTABLE | PdfPermissions::FILLABLE,
            ),
        ];
        groups.into_iter().filter(|(allowed, _)| *allowed).fold(
            PdfPermissions::COPYABLE_FOR_ACCESSIBILITY,
            |all, (_, group)| all | group,
        )
    }

    fn from_pdf(flags: PdfPermissions) -> Self {
        Self {
            print: flags.contains(PdfPermissions::PRINTABLE),
            copy: flags.contains(PdfPermissions::COPYABLE),
            edit: flags.contains(PdfPermissions::MODIFIABLE),
            annotate: flags.contains(PdfPermissions::ANNOTABLE),
        }
    }
}

/// The permissions of an encrypted document, opened with `password`; all of them for a
/// document that is not encrypted.
pub fn permissions(pdf: &[u8], password: &str) -> Result<Permissions> {
    let doc = Document::load_mem_with_options(pdf, LoadOptions::with_password(password)).map_err(
        |err| match err {
            lopdf::Error::InvalidPassword => Error::BadPassword,
            other => Error::Parse(other.to_string()),
        },
    )?;
    if doc.is_encrypted() {
        return Err(Error::BadPassword);
    }
    Ok(doc
        .encryption_state
        .as_ref()
        .map_or_else(Permissions::default, |state| {
            Permissions::from_pdf(state.permissions())
        }))
}

/// Encrypt with AES-256 (fallback AES-128 if unsupported). Empty `owner` = same as `user`.
///
/// Uses the standard security handler V5/R6 (AES-256), all permissions granted.
pub fn encrypt(pdf: &[u8], user: &str, owner: &str) -> Result<Vec<u8>> {
    let owner = if owner.is_empty() { user } else { owner };
    encrypt_with(pdf, user, owner, Permissions::default())
}

/// Encrypt with AES-256 granting only `permissions` to whoever opens it with `user`.
/// An empty `owner` gets a random one: nobody can lift the restrictions then.
pub fn encrypt_with(
    pdf: &[u8],
    user: &str,
    owner: &str,
    permissions: Permissions,
) -> Result<Vec<u8>> {
    let mut doc = load(pdf)?;
    let random_owner: String;
    let owner = if owner.is_empty() {
        let mut bytes = [0u8; 24];
        random(&mut bytes)?;
        random_owner = bytes.iter().map(|b| format!("{b:02x}")).collect();
        random_owner.as_str()
    } else {
        owner
    };
    let mut file_key = [0u8; 32];
    random(&mut file_key)?;
    let filter: Arc<dyn CryptFilter> = Arc::new(Aes256CryptFilter);
    let state = EncryptionState::try_from(EncryptionVersion::V5 {
        encrypt_metadata: true,
        crypt_filters: BTreeMap::from([(CRYPT_FILTER.to_vec(), filter)]),
        file_encryption_key: &file_key,
        stream_filter: CRYPT_FILTER.to_vec(),
        string_filter: CRYPT_FILTER.to_vec(),
        owner_password: owner,
        user_password: user,
        permissions: permissions.to_pdf(),
    })?;
    // Compress before encrypting: filters must apply to plaintext.
    doc.prune_objects();
    doc.compress();
    doc.encrypt(&state)?;
    doc.version = "2.0".into();
    save_raw(&mut doc)
}

fn random(bytes: &mut [u8]) -> Result<()> {
    getrandom::fill(bytes).map_err(|err| Error::Pdf(format!("no randomness available: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permissions_are_read_back() {
        let pdf = crate::blank_document(200.0, 200.0).unwrap();
        let limited = Permissions {
            print: true,
            copy: false,
            edit: false,
            annotate: true,
        };
        let locked = encrypt_with(&pdf, "user", "owner", limited).unwrap();
        assert_eq!(permissions(&locked, "user").unwrap(), limited);
        assert!(matches!(
            permissions(&locked, "nope"),
            Err(Error::BadPassword)
        ));
        let open = encrypt(&pdf, "user", "").unwrap();
        assert_eq!(permissions(&open, "user").unwrap(), Permissions::default());
        assert_eq!(permissions(&pdf, "").unwrap(), Permissions::default());
    }

    #[test]
    fn an_empty_owner_password_is_random() {
        let pdf = crate::blank_document(200.0, 200.0).unwrap();
        let nothing = Permissions {
            print: false,
            copy: false,
            edit: false,
            annotate: false,
        };
        let locked = encrypt_with(&pdf, "user", "", nothing).unwrap();
        assert_eq!(permissions(&locked, "user").unwrap(), nothing);
        assert!(decrypt(&locked, "user").is_ok());
        assert!(matches!(decrypt(&locked, ""), Err(Error::BadPassword)));
    }
}
