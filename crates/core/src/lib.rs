//! PDF editing operations for GodPDF.
//!
//! Every edit is a pure function `(&[u8], ..) -> Result<Vec<u8>>`: bytes in, new bytes out.
//! Page indices are **0-based** everywhere. Geometry is in PDF points (1/72 inch),
//! origin bottom-left, unrotated page space.

mod annotate;
mod error;
mod fonts;
mod helvetica;
mod info;
mod organize;
mod pdf;
mod retype;
mod security;
mod session;
mod text;

pub use annotate::{
    Annotation, Color, FREE_TEXT_ASCENT, FREE_TEXT_LINE_HEIGHT, FREE_TEXT_PADDING,
    PlacedAnnotation, Rect, add_annotation, add_annotations, annotations, free_text_rect,
    free_text_width, remove_annotation, replace_annotation, wrap_free_text, wrap_styled,
};
pub use error::{Error, Result};
pub use fonts::{CharStyle, Font, Script, style_at};
pub use helvetica::line_width as helvetica_line_width;
pub use info::{
    Metadata, OutlineItem, PageGeometry, outline, page_count, page_geometry, read_metadata,
    write_metadata,
};
pub use organize::{
    blank_document, delete_pages, extract_pages, insert_blank_page, insert_pdf, merge,
    reorder_pages, rotate_pages, split_at, split_every,
};
pub use retype::{LineLook, line_look, replace_text};
pub use security::{decrypt, encrypt, is_encrypted};
pub use session::Session;
pub use text::{SearchHit, extract_all_text, extract_text, search};
