mod common;

use common::{make_pdf, page_texts, texts};
use godpdf_core::{Error, decrypt, encrypt, is_encrypted, page_count, read_metadata, rotate_pages};

#[test]
fn encrypt_decrypt_round_trip() {
    let pdf = make_pdf(3, Some("Secret title"));
    assert!(!is_encrypted(&pdf));
    let locked = encrypt(&pdf, "user", "owner").unwrap();
    assert!(is_encrypted(&locked));
    for password in ["user", "owner"] {
        let open = decrypt(&locked, password).unwrap();
        assert!(!is_encrypted(&open));
        assert_eq!(page_count(&open).unwrap(), 3);
        assert_eq!(page_texts(&open), texts(&[1, 2, 3]));
        assert_eq!(
            read_metadata(&open).unwrap().title.as_deref(),
            Some("Secret title")
        );
    }
}

#[test]
fn wrong_password_is_rejected() {
    let locked = encrypt(&make_pdf(1, None), "user", "").unwrap();
    assert!(matches!(decrypt(&locked, "nope"), Err(Error::BadPassword)));
    assert!(decrypt(&locked, "user").is_ok());
}

#[test]
fn edits_on_locked_document_report_encrypted() {
    let locked = encrypt(&make_pdf(2, None), "user", "owner").unwrap();
    assert!(matches!(page_count(&locked), Err(Error::Encrypted)));
    assert!(matches!(
        rotate_pages(&locked, &[0], 90),
        Err(Error::Encrypted)
    ));
}

#[test]
fn decrypt_plain_document_is_noop() {
    let pdf = make_pdf(2, None);
    assert_eq!(page_count(&decrypt(&pdf, "anything").unwrap()).unwrap(), 2);
}

#[test]
fn owner_password_only_document() {
    // Empty user password: readable without a password, but still reported as encrypted.
    let locked = encrypt(&make_pdf(2, None), "", "owner").unwrap();
    assert!(is_encrypted(&locked));
    assert_eq!(page_count(&decrypt(&locked, "").unwrap()).unwrap(), 2);
    assert_eq!(page_count(&locked).unwrap(), 2);
    // Documented: edits open it transparently and drop the encryption.
    let rotated = rotate_pages(&locked, &[0], 90).unwrap();
    assert!(!is_encrypted(&rotated));
    assert_eq!(page_texts(&rotated), texts(&[1, 2]));
}
