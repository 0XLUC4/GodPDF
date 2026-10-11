/// Errors from PDF operations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Input is not a readable PDF.
    #[error("invalid PDF: {0}")]
    Parse(String),
    /// Document is encrypted; call [`crate::decrypt`] first.
    #[error("document is encrypted")]
    Encrypted,
    /// Wrong password for an encrypted document.
    #[error("wrong password")]
    BadPassword,
    /// A page index was outside `0..count`.
    #[error("page {page} out of range (document has {count} pages)")]
    PageOutOfRange { page: usize, count: usize },
    /// Caller passed an invalid argument (empty selection, n == 0, ...).
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    /// Anything else lopdf reports.
    #[error("{0}")]
    Pdf(String),
}

pub type Result<T> = std::result::Result<T, Error>;
