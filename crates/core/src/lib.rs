//! PDF editing operations for GodPDF.
//!
//! Every edit is a pure function `(&[u8], ..) -> Result<Vec<u8>>`: bytes in, new bytes out.
//! Page indices are **0-based** everywhere. Geometry is in PDF points (1/72 inch),
//! origin bottom-left, unrotated page space.

mod annotate;
mod attach;
mod bookmarks;
mod clean;
mod compare;
mod create;
mod error;
mod fonts;
mod forms;
mod helvetica;
mod info;
mod layout;
mod organize;
mod pdf;
mod retype;
mod security;
mod session;
mod stamp;
mod text;
mod textlayer;

pub use annotate::{
    Annotation, Color, FREE_TEXT_ASCENT, FREE_TEXT_LINE_HEIGHT, FREE_TEXT_PADDING, Picture,
    PlacedAnnotation, Rect, add_annotation, add_annotations, annotations, free_text_rect,
    free_text_width, remove_annotation, replace_annotation, wrap_free_text, wrap_styled,
};
pub use attach::{Attachment, add_attachment, attachments, remove_attachment};
pub use bookmarks::{contents_page, write_outline};
pub use clean::{
    Tint, compress, flatten_annotations, remove_comments, remove_hidden_data, tint_pages,
};
pub use compare::{PageDiff, compare_text};
pub use create::{jpeg_document, rgb_document, text_document};
pub use error::{Error, Result};
pub use fonts::{CharStyle, Font, Script, style_at};
pub use forms::{
    FieldKind, FieldValue, FormField, NewField, add_form_field, fill_form, form_fields,
};
pub use helvetica::line_width as helvetica_line_width;
pub use info::{
    Metadata, OutlineItem, PageGeometry, outline, page_count, page_geometry, read_metadata,
    write_metadata,
};
pub use layout::{booklet, crop_pages, halve_pages, join_pages, pages_per_sheet, resize_pages};
pub use organize::{
    blank_document, delete_pages, extract_pages, insert_blank_page, insert_pdf, interleave, merge,
    reorder_pages, rotate_pages, split_at, split_every,
};
pub use retype::{LineLook, line_look, replace_styled, replace_text};
pub use security::{decrypt, encrypt, is_encrypted};
pub use session::Session;
pub use stamp::{PageNumbers, Place, Watermark, number_pages, remove_links, watermark};
pub use text::{SearchHit, extract_all_text, extract_text, search};
pub use textlayer::{Word, add_text_layer};
