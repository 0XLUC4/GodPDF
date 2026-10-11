use std::collections::BTreeMap;
use std::sync::Arc;

use lopdf::encryption::crypt_filters::{Aes256CryptFilter, CryptFilter};
use lopdf::{Document, EncryptionState, EncryptionVersion, LoadOptions, Permissions};

use crate::pdf::{load, save, save_raw};
use crate::{Error, Result};

/// Name of the single crypt filter used for strings and streams.
const CRYPT_FILTER: &[u8] = b"StdCF";

/// True when the trailer has an /Encrypt dictionary.
///
/// Also true for owner-password-only files (empty user password). Those open without a
/// password, so `decrypt(pdf, "")` succeeds and every other function (page_count, edits, ...)
/// works on them directly — but edits return an **unencrypted** result. Callers that want to
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

/// Encrypt with AES-256 (fallback AES-128 if unsupported). Empty `owner` = same as `user`.
///
/// Uses the standard security handler V5/R6 (AES-256), all permissions granted.
pub fn encrypt(pdf: &[u8], user: &str, owner: &str) -> Result<Vec<u8>> {
    let mut doc = load(pdf)?;
    let owner = if owner.is_empty() { user } else { owner };
    let mut file_key = [0u8; 32];
    getrandom::fill(&mut file_key)
        .map_err(|err| Error::Pdf(format!("no randomness available: {err}")))?;
    let filter: Arc<dyn CryptFilter> = Arc::new(Aes256CryptFilter);
    let state = EncryptionState::try_from(EncryptionVersion::V5 {
        encrypt_metadata: true,
        crypt_filters: BTreeMap::from([(CRYPT_FILTER.to_vec(), filter)]),
        file_encryption_key: &file_key,
        stream_filter: CRYPT_FILTER.to_vec(),
        string_filter: CRYPT_FILTER.to_vec(),
        owner_password: owner,
        user_password: user,
        permissions: Permissions::all(),
    })?;
    // Compress before encrypting: filters must apply to plaintext.
    doc.prune_objects();
    doc.compress();
    doc.encrypt(&state)?;
    doc.version = "2.0".into();
    save_raw(&mut doc)
}
