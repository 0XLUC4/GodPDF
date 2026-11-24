//! Certificate signatures: sign with the key of a .pfx/.p12 file, and check the signatures
//! of a document.
//!
//! Signing appends an incremental update (signature field + /Sig dictionary) to the bytes
//! as they are, so earlier signatures stay valid, then puts a detached CMS SignedData over
//! the /ByteRange in the reserved /Contents. Keys: RSA (PKCS#1 v1.5) and ECDSA P-256, SHA-256.
//! Checking proves the signed bytes are intact and signed by the embedded certificate; it does
//! **not** check the certificate against a trusted authority.

use cms::cert::{CertificateChoices, IssuerAndSerialNumber};
use cms::content_info::{CmsVersion, ContentInfo};
use cms::signed_data::{
    CertificateSet, EncapsulatedContentInfo, SignedAttributes, SignedData, SignerIdentifier,
    SignerInfo, SignerInfos,
};
use der::asn1::{ObjectIdentifier, OctetString, SetOfVec, UtcTime};
use der::{Any, DateTime, Decode, Encode, Tag};
use lopdf::{Dictionary, Document, IncrementalDocument, Object, ObjectId, StringFormat};
use rsa::pkcs8::{DecodePrivateKey, DecodePublicKey};
use rsa::signature::{SignatureEncoding, Signer as _, Verifier as _};
use sha2::{Digest, Sha256};
use x509_cert::Certificate;
use x509_cert::attr::Attribute;
use x509_cert::spki::AlgorithmIdentifierOwned;
use x509_cert::time::Time;

use crate::{Error, Result};

/// Bytes reserved for the CMS in /Contents (written as twice as many hex digits).
const CONTENTS_SIZE: usize = 16 * 1024;
/// Stands for each /ByteRange number until the real ones are known: 10 digits each.
const RANGE_PLACEHOLDER: i64 = 9_999_999_999;
const RANGE_TEXT: &[u8] = b"[0 9999999999 9999999999 9999999999]";

const OID_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.1");
const OID_SIGNED_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.2");
const OID_CONTENT_TYPE: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.3");
const OID_MESSAGE_DIGEST: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.4");
const OID_SIGNING_TIME: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.5");
const OID_SIGNING_CERT_V2: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.2.47");
const OID_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.1");
const OID_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1");
const OID_RSA_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.11");
const OID_EC_KEY: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.2.1");
const OID_ECDSA_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.2");
const OID_COMMON_NAME: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.4.3");

enum Key {
    Rsa(rsa::pkcs1v15::SigningKey<Sha256>),
    P256(p256::ecdsa::SigningKey),
}

/// A private key and its certificate chain (signer first), read from a .pfx/.p12 file.
pub struct Signer {
    key: Key,
    chain: Vec<Certificate>,
}

impl std::fmt::Debug for Signer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Signer")
            .field("name", &self.name())
            .finish_non_exhaustive()
    }
}

impl Signer {
    /// Read the first key and its certificates from PKCS#12 bytes.
    /// [`Error::BadPassword`] on a wrong password.
    pub fn from_pkcs12(p12: &[u8], password: &str) -> Result<Self> {
        use p12_keystore::error::Error as P12Error;
        let store = p12_keystore::KeyStore::from_pkcs12(
            p12,
            password,
            p12_keystore::Pkcs12ImportPolicy::Strict,
        )
        .map_err(|err| match err {
            P12Error::MacError(_) | P12Error::UnpadError => Error::BadPassword,
            other => invalid(format!("cannot read the certificate file: {other}")),
        })?;
        let (_, chain) = store.private_key_chain().ok_or_else(|| {
            invalid("the certificate file holds no private key with its certificate")
        })?;
        let certs: Vec<&[u8]> = chain.certs().iter().map(|cert| cert.as_der()).collect();
        Self::from_der(chain.key().as_der(), &certs)
    }

    /// A PKCS#8 private key and DER certificates, the signer's first.
    fn from_der(key: &[u8], certs: &[&[u8]]) -> Result<Self> {
        let key = if let Ok(rsa) = rsa::PrivateKey::from_pkcs8_der(key) {
            Key::Rsa(rsa::pkcs1v15::SigningKey::new(rsa))
        } else if let Ok(ec) = p256::ecdsa::SigningKey::from_pkcs8_der(key) {
            Key::P256(ec)
        } else {
            return Err(invalid(
                "unsupported key type: only RSA and ECDSA P-256 keys can sign",
            ));
        };
        let chain = certs
            .iter()
            .map(|der| Certificate::from_der(der))
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|err| invalid(format!("unreadable certificate: {err}")))?;
        if chain.is_empty() {
            return Err(invalid("the certificate file holds no certificate"));
        }
        Ok(Self { key, chain })
    }

    /// Common name of the certificate (its whole subject without one).
    pub fn name(&self) -> String {
        common_name(&self.chain[0])
    }

    fn sign_bytes(&self, message: &[u8]) -> Vec<u8> {
        match &self.key {
            Key::Rsa(key) => key.sign(message).to_vec(),
            Key::P256(key) => {
                let signature: p256::ecdsa::DerSignature = key.sign(message);
                signature.to_vec()
            }
        }
    }

    fn signature_algorithm(&self) -> AlgorithmIdentifierOwned {
        match self.key {
            Key::Rsa(_) => AlgorithmIdentifierOwned {
                oid: OID_RSA_SHA256,
                parameters: Some(Any::null()),
            },
            Key::P256(_) => AlgorithmIdentifierOwned {
                oid: OID_ECDSA_SHA256,
                parameters: None,
            },
        }
    }
}

/// What the signer adds to the signature.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SignOptions {
    pub reason: String,
    pub location: String,
}

/// Sign `pdf` as it is: an invisible signature field is appended as an incremental update.
/// Any later change of the returned bytes (other than another incremental update) breaks it.
pub fn sign(pdf: &[u8], signer: &Signer, options: &SignOptions) -> Result<Vec<u8>> {
    let mut out = append_signature_field(pdf, signer, options)?;
    // Where the placeholders landed, in the part just appended.
    let tail = pdf.len();
    let range_at = tail
        + find(&out[tail..], RANGE_TEXT)
            .ok_or_else(|| Error::Pdf("signature placeholder not found".into()))?;
    let start = range_at
        + find(&out[range_at..], b"<")
            .ok_or_else(|| Error::Pdf("signature placeholder not found".into()))?;
    let end = start + 2 * CONTENTS_SIZE + 2;
    let placeholder_ok = out.get(start + 1..end - 1).is_some_and(|hex| {
        hex.iter().all(|&digit| digit == b'0') && out.get(end - 1) == Some(&b'>')
    });
    if !placeholder_ok {
        return Err(Error::Pdf("signature placeholder not found".into()));
    }
    let after = out.len() - end;
    let ranges = format!("[0 {start} {end} {after}");
    let mut patched = ranges.into_bytes();
    patched.resize(RANGE_TEXT.len() - 1, b' ');
    patched.push(b']');
    out[range_at..range_at + RANGE_TEXT.len()].copy_from_slice(&patched);

    let mut hasher = Sha256::new();
    hasher.update(&out[..start]);
    hasher.update(&out[end..]);
    let cms = signed_data(signer, &hasher.finalize())?;
    if cms.len() > CONTENTS_SIZE {
        return Err(invalid(format!(
            "the signature ({} bytes) does not fit in the {CONTENTS_SIZE} bytes reserved for it: \
             the certificate chain is too long",
            cms.len()
        )));
    }
    let hex: String = cms.iter().map(|byte| format!("{byte:02X}")).collect();
    out[start + 1..start + 1 + hex.len()].copy_from_slice(hex.as_bytes());
    Ok(out)
}

/// `pdf` plus an update holding the /Sig dictionary (placeholders for /ByteRange and
/// /Contents), its field on the first page and the AcroForm listing it.
fn append_signature_field(pdf: &[u8], signer: &Signer, options: &SignOptions) -> Result<Vec<u8>> {
    let prev = Document::load_mem(pdf).map_err(|err| Error::Parse(err.to_string()))?;
    let mut doc = IncrementalDocument::create_from(pdf.to_vec(), prev);
    let prev = doc.get_prev_documents();
    if prev.is_encrypted() || prev.trailer.has(b"Encrypt") {
        return Err(invalid(
            "remove the password of the document before signing it",
        ));
    }
    if prev.xref_start == 0 {
        return Err(invalid(
            "the cross-reference table of this file is damaged: save it once, then sign the copy",
        ));
    }
    let page_id = *prev
        .get_pages()
        .values()
        .next()
        .ok_or_else(|| invalid("the document has no pages"))?;
    let catalog_id = prev.trailer.get(b"Root")?.as_reference()?;
    let field_name = format!("Signature{}", existing_signatures(prev).len() + 1);

    let now = std::time::SystemTime::now();
    let mut sig = Dictionary::new();
    sig.set("Type", Object::Name(b"Sig".to_vec()));
    sig.set("Filter", Object::Name(b"Adobe.PPKLite".to_vec()));
    sig.set("SubFilter", Object::Name(b"adbe.pkcs7.detached".to_vec()));
    sig.set("Name", lopdf::text_string(&signer.name()));
    sig.set("M", Object::string_literal(pdf_date(now)?));
    if !options.reason.trim().is_empty() {
        sig.set("Reason", lopdf::text_string(options.reason.trim()));
    }
    if !options.location.trim().is_empty() {
        sig.set("Location", lopdf::text_string(options.location.trim()));
    }
    // /ByteRange right before /Contents: the placeholder of /Contents is the next `<`.
    sig.set(
        "ByteRange",
        Object::Array(vec![
            Object::Integer(0),
            Object::Integer(RANGE_PLACEHOLDER),
            Object::Integer(RANGE_PLACEHOLDER),
            Object::Integer(RANGE_PLACEHOLDER),
        ]),
    );
    sig.set(
        "Contents",
        Object::String(vec![0; CONTENTS_SIZE], StringFormat::Hexadecimal),
    );
    let new = &mut doc.new_document;
    let sig_id = new.add_object(sig);
    let mut widget = Dictionary::new();
    widget.set("Type", Object::Name(b"Annot".to_vec()));
    widget.set("Subtype", Object::Name(b"Widget".to_vec()));
    widget.set("FT", Object::Name(b"Sig".to_vec()));
    widget.set("T", lopdf::text_string(&field_name));
    widget.set("V", Object::Reference(sig_id));
    // Print + Locked; an empty rectangle: the signature is invisible.
    widget.set("F", Object::Integer(132));
    widget.set(
        "Rect",
        Object::Array(vec![0.into(), 0.into(), 0.into(), 0.into()]),
    );
    widget.set("P", Object::Reference(page_id));
    let widget_id = new.add_object(widget);

    push_to_array(&mut doc, page_id, b"Annots", widget_id)?;
    let acro_form = doc
        .get_prev_documents()
        .get_dictionary(catalog_id)?
        .get(b"AcroForm")
        .ok()
        .cloned();
    let form_id = match acro_form {
        Some(Object::Reference(id)) => id,
        // A form inside the catalog, or none: it becomes an object of its own.
        found => {
            let form = match found {
                Some(Object::Dictionary(form)) => form,
                _ => Dictionary::new(),
            };
            let form_id = doc.new_document.add_object(form);
            doc.opt_clone_object_to_new_document(catalog_id)?;
            doc.new_document
                .get_dictionary_mut(catalog_id)?
                .set("AcroForm", Object::Reference(form_id));
            form_id
        }
    };
    push_to_array(&mut doc, form_id, b"Fields", widget_id)?;
    // Signatures exist, and the file must only be appended to.
    doc.new_document
        .get_dictionary_mut(form_id)?
        .set("SigFlags", Object::Integer(3));

    let mut out = Vec::with_capacity(pdf.len() + 4 * CONTENTS_SIZE);
    doc.save_to(&mut out)
        .map_err(|err| Error::Pdf(err.to_string()))?;
    Ok(out)
}

/// Append `item` to the array `key` of the dictionary `owner`, in the new revision.
fn push_to_array(
    doc: &mut IncrementalDocument,
    owner: ObjectId,
    key: &[u8],
    item: ObjectId,
) -> Result<()> {
    if !doc.new_document.has_object(owner) {
        doc.opt_clone_object_to_new_document(owner)?;
    }
    let entry = doc
        .new_document
        .get_dictionary(owner)?
        .get(key)
        .ok()
        .cloned();
    match entry {
        // An array of its own: update that object.
        Some(Object::Reference(array_id)) => {
            if !doc.new_document.has_object(array_id) {
                doc.opt_clone_object_to_new_document(array_id)?;
            }
            doc.new_document
                .get_object_mut(array_id)?
                .as_array_mut()
                .map_err(|_| Error::Parse("expected an array".into()))?
                .push(Object::Reference(item));
        }
        entry => {
            let mut items = match entry {
                Some(Object::Array(items)) => items,
                _ => Vec::new(),
            };
            items.push(Object::Reference(item));
            doc.new_document
                .get_dictionary_mut(owner)?
                .set(key, Object::Array(items));
        }
    }
    Ok(())
}

/// DER ContentInfo of a detached SignedData over a document of SHA-256 `digest`.
fn signed_data(signer: &Signer, digest: &[u8]) -> Result<Vec<u8>> {
    let cert = &signer.chain[0];
    let cert_hash = Sha256::digest(cert.to_der().map_err(der_error)?);
    // SigningCertificateV2 { certs: [ESSCertIDv2 { certHash }] } (hash algorithm: SHA-256 default).
    let hash = OctetString::new(cert_hash.to_vec()).map_err(der_error)?;
    let ess_cert_id = sequence(&hash)?;
    let signing_cert = sequence(&sequence(&ess_cert_id)?)?;
    let now = DateTime::from_system_time(std::time::SystemTime::now()).map_err(der_error)?;
    let time = Time::UtcTime(UtcTime::from_date_time(now).map_err(der_error)?);
    let attrs: SignedAttributes = SetOfVec::try_from(vec![
        attribute(OID_CONTENT_TYPE, Any::encode_from(&OID_DATA))?,
        attribute(OID_SIGNING_TIME, Any::encode_from(&time))?,
        attribute(
            OID_MESSAGE_DIGEST,
            Any::encode_from(&OctetString::new(digest.to_vec()).map_err(der_error)?),
        )?,
        attribute(OID_SIGNING_CERT_V2, Ok(signing_cert))?,
    ])
    .map_err(der_error)?;
    let signature = signer.sign_bytes(&attrs.to_der().map_err(der_error)?);
    let sha256 = AlgorithmIdentifierOwned {
        oid: OID_SHA256,
        parameters: None,
    };
    let signer_info = SignerInfo {
        version: CmsVersion::V1,
        sid: SignerIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
            issuer: cert.tbs_certificate().issuer().clone(),
            serial_number: cert.tbs_certificate().serial_number().clone(),
        }),
        digest_alg: sha256.clone(),
        signed_attrs: Some(attrs),
        signature_algorithm: signer.signature_algorithm(),
        signature: OctetString::new(signature).map_err(der_error)?,
        unsigned_attrs: None,
    };
    let certificates = signer
        .chain
        .iter()
        .cloned()
        .map(CertificateChoices::Certificate)
        .collect::<Vec<_>>();
    let signed = SignedData {
        version: CmsVersion::V1,
        digest_algorithms: SetOfVec::try_from(vec![sha256]).map_err(der_error)?,
        encap_content_info: EncapsulatedContentInfo {
            econtent_type: OID_DATA,
            econtent: None,
        },
        certificates: Some(CertificateSet(
            SetOfVec::try_from(certificates).map_err(der_error)?,
        )),
        crls: None,
        signer_infos: SignerInfos(SetOfVec::try_from(vec![signer_info]).map_err(der_error)?),
    };
    ContentInfo {
        content_type: OID_SIGNED_DATA,
        content: Any::encode_from(&signed).map_err(der_error)?,
    }
    .to_der()
    .map_err(der_error)
}

fn attribute(oid: ObjectIdentifier, value: der::Result<Any>) -> Result<Attribute> {
    Ok(Attribute {
        oid,
        values: SetOfVec::try_from(vec![value.map_err(der_error)?]).map_err(der_error)?,
    })
}

/// A SEQUENCE of the one element `content`.
fn sequence(content: &impl Encode) -> Result<Any> {
    Any::new(Tag::Sequence, content.to_der().map_err(der_error)?).map_err(der_error)
}

/// How a signature checks out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignatureStatus {
    /// Intact, and it covers the whole file.
    Valid,
    /// The signed revision is intact, but the file was changed (appended to) after signing.
    ModifiedAfter,
    /// Signed bytes changed, or the signature does not match its certificate: why.
    Invalid(String),
    /// Uses something not supported here (another digest, a timestamp only, ...): what.
    Unchecked(String),
}

/// A signature of the document. The certificate is not checked against a trusted authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureInfo {
    /// Name of its form field.
    pub field: String,
    /// Common name of the signing certificate, else the /Name of the signature.
    pub signer: String,
    /// When it was signed (UTC): the signed time, else the claimed /M date.
    pub signed_at: Option<String>,
    pub reason: Option<String>,
    pub location: Option<String>,
    pub status: SignatureStatus,
}

/// Every signature of `pdf`, oldest first, each checked against the bytes of `pdf`.
pub fn signatures(pdf: &[u8]) -> Result<Vec<SignatureInfo>> {
    let doc = crate::pdf::load(pdf)?;
    let mut found: Vec<(usize, SignatureInfo)> = existing_signatures(&doc)
        .into_iter()
        .filter_map(|(field, sig)| {
            let sig = doc.get_dictionary(sig).ok()?;
            Some(check(&doc, pdf, field, sig))
        })
        .collect();
    found.sort_by_key(|(end, _)| *end);
    Ok(found.into_iter().map(|(_, info)| info).collect())
}

/// Name and /Sig dictionary of every signed signature field.
fn existing_signatures(doc: &Document) -> Vec<(String, ObjectId)> {
    doc.objects
        .values()
        .filter_map(|object| {
            let dict = object.as_dict().ok()?;
            if dict.get(b"FT").and_then(Object::as_name).ok()? != b"Sig" {
                return None;
            }
            let sig = dict.get(b"V").and_then(Object::as_reference).ok()?;
            let name = crate::info::text_entry(doc, dict, b"T").unwrap_or_default();
            Some((name, sig))
        })
        .collect()
}

/// The signature `sig` of field `field`, and where its signed bytes end (to sort them).
fn check(doc: &Document, pdf: &[u8], field: String, sig: &Dictionary) -> (usize, SignatureInfo) {
    let text = |key: &[u8]| crate::info::text_entry(doc, sig, key).filter(|t| !t.is_empty());
    let mut info = SignatureInfo {
        field,
        signer: text(b"Name").unwrap_or_else(|| "Unknown signer".into()),
        signed_at: sig
            .get(b"M")
            .and_then(Object::as_str)
            .ok()
            .and_then(parse_pdf_date),
        reason: text(b"Reason"),
        location: text(b"Location"),
        status: SignatureStatus::Valid,
    };
    let ranges: Vec<usize> = sig
        .get(b"ByteRange")
        .and_then(Object::as_array)
        .map(|ranges| {
            ranges
                .iter()
                .filter_map(|n| n.as_i64().ok().and_then(|n| usize::try_from(n).ok()))
                .collect()
        })
        .unwrap_or_default();
    let contents = sig
        .get(b"Contents")
        .and_then(Object::as_str)
        .unwrap_or_default();
    let [0, first_len, second_at, second_len] = ranges[..] else {
        info.status = SignatureStatus::Invalid("its byte range is missing or malformed".into());
        return (0, info);
    };
    let end = second_at.saturating_add(second_len);
    if first_len > second_at || end > pdf.len() {
        info.status = SignatureStatus::Invalid("its byte range lies outside the file".into());
        return (end, info);
    }
    let mut hasher = Sha256::new();
    hasher.update(&pdf[..first_len]);
    hasher.update(&pdf[second_at..end]);
    let signed = Signed {
        digest: hasher.finalize().to_vec(),
        first: &pdf[..first_len],
        second: &pdf[second_at..end],
    };
    match verify_cms(contents, &signed) {
        Ok(cms) => {
            if let Some(name) = cms.signer {
                info.signer = name;
            }
            if cms.signed_at.is_some() {
                info.signed_at = cms.signed_at;
            }
            // Bytes after the signed part, beyond line ends, are a later revision.
            let covers_all = pdf[end..].iter().all(u8::is_ascii_whitespace);
            if !covers_all {
                info.status = SignatureStatus::ModifiedAfter;
            }
        }
        Err(status) => info.status = status,
    }
    (end, info)
}

/// The bytes a signature covers.
struct Signed<'a> {
    digest: Vec<u8>,
    first: &'a [u8],
    second: &'a [u8],
}

struct CmsCheck {
    signer: Option<String>,
    signed_at: Option<String>,
}

/// Check the CMS `contents` (zero padded) against `signed`.
fn verify_cms(contents: &[u8], signed: &Signed) -> std::result::Result<CmsCheck, SignatureStatus> {
    let unreadable =
        |err: der::Error| SignatureStatus::Invalid(format!("unreadable signature: {err}"));
    let (info, _padding) = ContentInfo::from_der_partial(contents).map_err(unreadable)?;
    if info.content_type != OID_SIGNED_DATA {
        return Err(SignatureStatus::Unchecked("not a CMS signature".into()));
    }
    let data =
        SignedData::from_der(&info.content.to_der().map_err(unreadable)?).map_err(unreadable)?;
    let signer_info = data
        .signer_infos
        .0
        .iter()
        .next()
        .ok_or_else(|| SignatureStatus::Invalid("the signature has no signer".into()))?;
    let certs: Vec<&Certificate> = data
        .certificates
        .iter()
        .flat_map(|set| set.0.iter())
        .filter_map(|choice| match choice {
            CertificateChoices::Certificate(cert) => Some(cert),
            CertificateChoices::Other(_) => None,
        })
        .collect();
    let cert = certs
        .iter()
        .find(|cert| match &signer_info.sid {
            SignerIdentifier::IssuerAndSerialNumber(id) => {
                cert.tbs_certificate().issuer() == &id.issuer
                    && cert.tbs_certificate().serial_number() == &id.serial_number
            }
            SignerIdentifier::SubjectKeyIdentifier(_) => true,
        })
        .copied()
        .ok_or_else(|| SignatureStatus::Invalid("the signing certificate is missing".into()))?;
    let mut check = CmsCheck {
        signer: Some(common_name(cert)),
        signed_at: None,
    };
    if signer_info.digest_alg.oid != OID_SHA256 {
        return Err(SignatureStatus::Unchecked(format!(
            "digest algorithm {} is not supported (SHA-256 only)",
            signer_info.digest_alg.oid
        )));
    }
    let message = match &signer_info.signed_attrs {
        Some(attrs) => {
            let value = |oid| {
                attrs
                    .iter()
                    .find(|attr| attr.oid == oid)
                    .and_then(|attr| attr.values.iter().next())
            };
            let digest = value(OID_MESSAGE_DIGEST)
                .and_then(|any| any.decode_as::<OctetString>().ok())
                .ok_or_else(|| SignatureStatus::Invalid("the signed digest is missing".into()))?;
            if digest.as_bytes() != signed.digest {
                return Err(SignatureStatus::Invalid(
                    "the document was changed after signing".into(),
                ));
            }
            check.signed_at = value(OID_SIGNING_TIME)
                .and_then(|any| any.to_der().ok())
                .and_then(|der| Time::from_der(&der).ok())
                .map(|time| date_text(time.to_date_time()));
            attrs.to_der().map_err(unreadable)?
        }
        // Without signed attributes the signature covers the document bytes directly.
        None => [signed.first, signed.second].concat(),
    };
    verify_signature(
        cert,
        &signer_info.signature_algorithm,
        &message,
        signer_info.signature.as_bytes(),
    )?;
    Ok(check)
}

/// Check `signature` of `message` with the public key of `cert`.
fn verify_signature(
    cert: &Certificate,
    algorithm: &AlgorithmIdentifierOwned,
    message: &[u8],
    signature: &[u8],
) -> std::result::Result<(), SignatureStatus> {
    let spki = cert.tbs_certificate().subject_public_key_info();
    let spki_der = spki
        .to_der()
        .map_err(|err| SignatureStatus::Invalid(format!("unreadable certificate key: {err}")))?;
    let mismatch =
        || SignatureStatus::Invalid("the signature does not match its certificate".into());
    let (key_type, algorithm) = (spki.algorithm.oid, algorithm.oid);
    if key_type == OID_RSA && [OID_RSA, OID_RSA_SHA256].contains(&algorithm) {
        let key = rsa::PublicKey::from_public_key_der(&spki_der).map_err(|_| mismatch())?;
        let signature = rsa::pkcs1v15::Signature::try_from(signature).map_err(|_| mismatch())?;
        rsa::pkcs1v15::VerifyingKey::<Sha256>::new(key)
            .verify(message, &signature)
            .map_err(|_| mismatch())
    } else if key_type == OID_EC_KEY && [OID_ECDSA_SHA256, OID_EC_KEY].contains(&algorithm) {
        let key = p256::ecdsa::VerifyingKey::from_public_key_der(&spki_der).map_err(|_| {
            SignatureStatus::Unchecked("only P-256 elliptic curve keys are supported".into())
        })?;
        let signature = p256::ecdsa::DerSignature::from_bytes(signature).map_err(|_| mismatch())?;
        key.verify(message, &signature).map_err(|_| mismatch())
    } else {
        Err(SignatureStatus::Unchecked(format!(
            "signature algorithm {algorithm} is not supported"
        )))
    }
}

/// Common name of the subject of `cert`, else the whole subject.
fn common_name(cert: &Certificate) -> String {
    let subject = cert.tbs_certificate().subject();
    subject
        .iter()
        .find(|atv| atv.oid == OID_COMMON_NAME)
        .map(|atv| String::from_utf8_lossy(atv.value.value()).into_owned())
        .unwrap_or_else(|| subject.to_string())
}

/// `D:YYYYMMDDHHmmSS+00'00'` for `time` (UTC).
fn pdf_date(time: std::time::SystemTime) -> Result<String> {
    let date = DateTime::from_system_time(time).map_err(der_error)?;
    Ok(format!(
        "D:{:04}{:02}{:02}{:02}{:02}{:02}+00'00'",
        date.year(),
        date.month(),
        date.day(),
        date.hour(),
        date.minutes(),
        date.seconds()
    ))
}

/// `YYYY-MM-DD HH:MM` and its offset from a PDF date (`D:YYYYMMDDHHmmSS+HH'mm'`).
fn parse_pdf_date(raw: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(raw).ok()?.trim_start_matches("D:");
    let digits = |range: std::ops::Range<usize>| {
        text.get(range)
            .filter(|d| d.bytes().all(|b| b.is_ascii_digit()))
    };
    let (year, month, day) = (digits(0..4)?, digits(4..6)?, digits(6..8)?);
    let (hour, minute) = (
        digits(8..10).unwrap_or("00"),
        digits(10..12).unwrap_or("00"),
    );
    let zone = match text.get(14..).unwrap_or("").replace('\'', "") {
        zone if zone.is_empty() || zone.starts_with('Z') || zone.starts_with("+0000") => {
            "UTC".to_owned()
        }
        zone if zone.len() >= 5 => format!("UTC{}:{}", &zone[..3], &zone[3..5]),
        zone => format!("UTC{zone}"),
    };
    Some(format!("{year}-{month}-{day} {hour}:{minute} {zone}"))
}

fn date_text(date: DateTime) -> String {
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02} UTC",
        date.year(),
        date.month(),
        date.day(),
        date.hour(),
        date.minutes()
    )
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn invalid(message: impl Into<String>) -> Error {
    Error::InvalidArgument(message.into())
}

fn der_error(err: der::Error) -> Error {
    Error::Pdf(format!("cannot encode the signature: {err}"))
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;
    use std::time::Duration;

    use p12_keystore::{KeyStore, KeyStoreEntry, PrivateKey, PrivateKeyChain};
    use rsa::pkcs8::EncodePrivateKey;
    use x509_cert::builder::{Builder, CertificateBuilder, Profile};
    use x509_cert::ext::Extension;
    use x509_cert::name::Name;
    use x509_cert::serial_number::SerialNumber;
    use x509_cert::spki::{SubjectPublicKeyInfoOwned, SubjectPublicKeyInfoRef};
    use x509_cert::time::Validity;

    use super::*;

    const PASSWORD: &str = "secret";

    /// Self-signed, no extensions.
    struct SelfSigned(Name);

    impl Profile for SelfSigned {
        fn get_issuer(&self, subject: &Name) -> Name {
            subject.clone()
        }
        fn get_subject(&self) -> Name {
            self.0.clone()
        }
        fn build_extensions(
            &self,
            _: SubjectPublicKeyInfoRef<'_>,
            _: SubjectPublicKeyInfoRef<'_>,
            _: &x509_cert::TbsCertificate,
        ) -> x509_cert::builder::Result<Vec<Extension>> {
            Ok(Vec::new())
        }
    }

    fn builder(name: &str, spki: SubjectPublicKeyInfoOwned) -> CertificateBuilder<SelfSigned> {
        CertificateBuilder::new(
            SelfSigned(Name::from_str(&format!("CN={name},O=GodPDF tests")).unwrap()),
            SerialNumber::from(7u32),
            Validity::from_now(Duration::from_secs(3600)).unwrap(),
            spki,
        )
        .unwrap()
    }

    /// A .p12 holding `key` (PKCS#8) and `cert`, protected by [`PASSWORD`].
    fn pkcs12(key: &[u8], cert: &Certificate) -> Vec<u8> {
        let cert = p12_keystore::Certificate::from_der(&cert.to_der().unwrap()).unwrap();
        let chain = PrivateKeyChain::new(vec![1u8], PrivateKey::from_der(key).unwrap(), [cert]);
        let mut store = KeyStore::new();
        store.add_entry("signer", KeyStoreEntry::PrivateKeyChain(chain));
        store.writer(PASSWORD).write().unwrap()
    }

    fn rsa_p12() -> Vec<u8> {
        let key = rsa::PrivateKey::new(&mut getrandom::SysRng, 2048).unwrap();
        let signing = rsa::pkcs1v15::SigningKey::<Sha256>::new(key.clone());
        let spki = SubjectPublicKeyInfoOwned::from_key(&key.to_public_key()).unwrap();
        let cert = builder("Test RSA", spki)
            .build::<_, rsa::pkcs1v15::Signature>(&signing)
            .unwrap();
        pkcs12(key.to_pkcs8_der().unwrap().as_bytes(), &cert)
    }

    fn ecdsa_p12() -> Vec<u8> {
        use p256::elliptic_curve::Generate;
        let key = p256::ecdsa::SigningKey::generate();
        let spki = SubjectPublicKeyInfoOwned::from_key(key.verifying_key()).unwrap();
        let cert = builder("Test ECDSA", spki)
            .build::<_, p256::ecdsa::DerSignature>(&key)
            .unwrap();
        pkcs12(key.to_pkcs8_der().unwrap().as_bytes(), &cert)
    }

    fn options() -> SignOptions {
        SignOptions {
            reason: "Approval".into(),
            location: "Lyon".into(),
        }
    }

    fn statuses(pdf: &[u8]) -> Vec<SignatureStatus> {
        signatures(pdf)
            .unwrap()
            .into_iter()
            .map(|info| info.status)
            .collect()
    }

    /// Signed, tampered with, then signed again on top.
    fn round_trip(p12: &[u8], name: &str) {
        let signer = Signer::from_pkcs12(p12, PASSWORD).unwrap();
        assert_eq!(signer.name(), name);
        let blank = crate::blank_document(595.0, 842.0).unwrap();
        let signed = sign(&blank, &signer, &options()).unwrap();
        assert!(signed.starts_with(&blank), "an incremental update");

        let found = signatures(&signed).unwrap();
        assert_eq!(found.len(), 1);
        let info = &found[0];
        assert_eq!(info.status, SignatureStatus::Valid);
        assert_eq!(info.signer, name);
        assert_eq!(info.field, "Signature1");
        assert_eq!(info.reason.as_deref(), Some("Approval"));
        assert_eq!(info.location.as_deref(), Some("Lyon"));
        assert!(
            info.signed_at
                .as_deref()
                .is_some_and(|at| at.ends_with("UTC"))
        );

        // One signed byte changed: the reason.
        let mut tampered = signed.clone();
        let at = find(&tampered, b"(Approval)").unwrap();
        tampered[at + 1] = b'X';
        assert!(matches!(
            statuses(&tampered)[..],
            [SignatureStatus::Invalid(_)]
        ));

        // A later incremental update (a second signature): the first one is still intact.
        let twice = sign(&signed, &signer, &SignOptions::default()).unwrap();
        assert_eq!(
            statuses(&twice),
            [SignatureStatus::ModifiedAfter, SignatureStatus::Valid]
        );
        assert_eq!(signatures(&twice).unwrap()[1].field, "Signature2");
    }

    #[test]
    fn rsa_signatures_check_out() {
        round_trip(&rsa_p12(), "Test RSA");
    }

    #[test]
    fn ecdsa_signatures_check_out() {
        round_trip(&ecdsa_p12(), "Test ECDSA");
    }

    #[test]
    fn appended_changes_are_reported() {
        let signer = Signer::from_pkcs12(&ecdsa_p12(), PASSWORD).unwrap();
        let blank = crate::blank_document(595.0, 842.0).unwrap();
        let signed = sign(&blank, &signer, &options()).unwrap();
        let prev = Document::load_mem(&signed).unwrap();
        let mut update = IncrementalDocument::create_from(signed, prev);
        update
            .new_document
            .add_object(Object::string_literal("added later"));
        let mut changed = Vec::new();
        update.save_to(&mut changed).unwrap();
        assert_eq!(statuses(&changed), [SignatureStatus::ModifiedAfter]);
    }

    #[test]
    fn wrong_password_and_bad_files_are_refused() {
        let p12 = ecdsa_p12();
        assert!(matches!(
            Signer::from_pkcs12(&p12, "wrong"),
            Err(Error::BadPassword)
        ));
        assert!(Signer::from_pkcs12(b"not a p12", PASSWORD).is_err());
        let unsigned = crate::blank_document(595.0, 842.0).unwrap();
        assert!(signatures(&unsigned).unwrap().is_empty());
    }

    #[test]
    fn pdf_dates_read_well() {
        assert_eq!(
            parse_pdf_date(b"D:20261124153000+01'00'").as_deref(),
            Some("2026-11-24 15:30 UTC+01:00")
        );
        assert_eq!(
            parse_pdf_date(b"D:20261124153000Z").as_deref(),
            Some("2026-11-24 15:30 UTC")
        );
        assert_eq!(parse_pdf_date(b"garbage"), None);
    }
}
