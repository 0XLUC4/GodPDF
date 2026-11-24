//! The dialogs made of a [`Form`]: their fields, and what confirming them does.
//! Every change of the document is one undo step through `DocTab::apply`.

use std::ops::RangeInclusive;

use godpdf_core::{
    self as core, Bates, Color, HeaderFooter, Metadata, PageNumbers, Permissions, Place, Tint,
    Watermark,
};
use gpui::{App, Entity, Window};

use super::form::{self, Form};
use super::{Passwords, active_app, convert, output, plain, ranges, state};
use crate::doc::{DocTab, guarded};
use crate::toasts;

/// Millimetres to points.
const MM: f32 = 72.0 / 25.4;
const PAGES: &str = "Pages";
const PAGES_HINT: &str = "e.g. 1-3, 5, 8-";
pub(super) const PER_PART: &str = "Pages per part";
pub(super) const STARTS_AT: &str = "New part starts at";
const LINK_TO: &str = "Link to";
pub(super) const OPTIONS: &str = "Options";
pub(super) const MARK: &str = "Watermark";
pub(super) const SCALE: &str = "Size (% of page)";
const OWNER: &str = "Owner password";
const ALLOW: [&str; 2] = ["Allowed", "Not allowed"];
const FIELD_KINDS: [&str; 5] = ["Text", "Check box", "Radio buttons", "Drop-down", "List"];

/// Colours offered for stamps.
const COLOURS: [(&str, Color); 5] = [
    (
        "Black",
        Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
        },
    ),
    (
        "Grey",
        Color {
            r: 0.5,
            g: 0.5,
            b: 0.5,
        },
    ),
    (
        "Red",
        Color {
            r: 0.85,
            g: 0.18,
            b: 0.13,
        },
    ),
    (
        "Blue",
        Color {
            r: 0.12,
            g: 0.37,
            b: 0.84,
        },
    ),
    (
        "White",
        Color {
            r: 1.0,
            g: 1.0,
            b: 1.0,
        },
    ),
];

/// How page numbers can be written: `{n}` is the page, `{total}` the last one.
const NUMBER_FORMATS: [&str; 5] = [
    "{n}",
    "Page {n}",
    "{n} / {total}",
    "Page {n} of {total}",
    "- {n} -",
];

/// Name and upright size in points.
const PAPERS: [(&str, (f32, f32)); 5] = [
    ("A4", (595.0, 842.0)),
    ("Letter", (612.0, 792.0)),
    ("A3", (842.0, 1191.0)),
    ("A5", (420.0, 595.0)),
    ("Legal", (612.0, 1008.0)),
];

const PER_SHEET: [usize; 6] = [2, 4, 6, 8, 9, 16];
const DPI: [u32; 3] = [72, 150, 300];
/// Sharpness pictures are scaled down to by Compress; `None` keeps them.
const PICTURE_DPI: [Option<u32>; 4] = [None, Some(72), Some(150), Some(300)];
/// JPEG quality of the pictures Compress writes: low, medium, high.
const JPEG_QUALITY: [u8; 3] = [50, 75, 90];
const META_LABELS: [&str; 6] = [
    "Title", "Author", "Subject", "Keywords", "Creator", "Producer",
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    GoToPage,
    NumberPages,
    Watermark,
    Crop,
    PageSize,
    PerSheet,
    Halve,
    Colours,
    Images,
    Extract,
    Split,
    Encrypt,
    Properties,
    /// A form field over `rect` (PDF space) of `page`.
    NewField {
        page: usize,
        rect: core::Rect,
    },
    Compress,
    /// The PDF/A copy is made: it shows its warnings and asks where to save it.
    PdfA,
    Word,
    /// A link over `rect` (PDF space) of `page`.
    NewLink {
        page: usize,
        rect: core::Rect,
    },
    HeaderFooter,
    Bates,
    /// The words and colour of the stamp tool's own stamp.
    CustomStamp,
}

impl Kind {
    /// Title and one line saying what it does.
    pub fn title(self) -> (&'static str, &'static str) {
        match self {
            Self::GoToPage => ("Go to page", ""),
            Self::NumberPages => ("Number pages", "Write its number on every page"),
            Self::Watermark => (
                "Add watermark",
                "A line of text or a picture across the pages",
            ),
            Self::Crop => ("Crop pages", "Hide the margins of the pages"),
            Self::PageSize => ("Change page size", "What the pages show is scaled to fit"),
            Self::PerSheet => ("Pages per sheet", "Several pages on each sheet"),
            Self::Halve => (
                "Cut pages in half",
                "Each page into two: scans of two pages side by side",
            ),
            Self::Colours => (
                "Page colours",
                "Invert for night reading, greyscale, or tint the paper",
            ),
            Self::Images => ("Export as images", "Each page as a PNG or JPEG picture"),
            Self::Extract => ("Extract pages", "Copy pages into a new PDF"),
            Self::Split => ("Split document", "Cut a PDF into several files"),
            Self::Encrypt => ("Encrypt with password", "Applied when you save (AES-256)."),
            Self::Properties => ("Document properties", ""),
            Self::NewField { .. } => ("New form field", ""),
            Self::Compress => (
                "Compress",
                "A smaller file that shows the same. Lighter pictures lose detail.",
            ),
            Self::PdfA => (
                "Save as PDF/A",
                "PDF/A-2b (best effort): an archival copy, without scripts or attached files.",
            ),
            Self::Word => (
                "Export to Word",
                "The text of each page in a .docx document. Layout and fonts are not kept.",
            ),
            Self::NewLink { .. } => ("New link", "Click it with Select or Hand to follow it"),
            Self::HeaderFooter => (
                "Header & footer",
                "{n} page, {total} last page, {date} today, {file} file name",
            ),
            Self::Bates => ("Bates numbering", "A unique number on every page"),
            Self::CustomStamp => ("Custom stamp", "The stamp tool places it with a click"),
        }
    }

    /// Primary button, and the second one when there is one.
    pub fn buttons(self) -> (&'static str, Option<&'static str>) {
        match self {
            Self::GoToPage => ("Go", None),
            Self::NumberPages => ("Number pages", None),
            Self::Watermark => ("Add watermark", None),
            Self::Crop => ("Crop", None),
            Self::PageSize | Self::PerSheet | Self::Halve | Self::Properties => ("Apply", None),
            Self::Colours => ("Change colours", None),
            Self::Images => ("Export", None),
            Self::Extract => ("Open in new tab", Some("Save as…")),
            Self::Split => ("Save to folder…", Some("Open in new tabs")),
            Self::Encrypt => ("Encrypt on save", None),
            Self::NewField { .. } => ("Add field", None),
            Self::Compress => ("Compress", None),
            Self::PdfA => ("Save as…", None),
            Self::Word => ("Export…", None),
            Self::NewLink { .. } => ("Add link", None),
            Self::HeaderFooter => ("Add header & footer", None),
            Self::Bates => ("Number pages", None),
            Self::CustomStamp => ("Use stamp", None),
        }
    }
}

/// The dialog `kind` for the document `tab`, its fields filled in.
pub fn build(
    kind: Kind,
    tab: &Entity<DocTab>,
    window: &mut Window,
    cx: &mut App,
) -> Result<Form, String> {
    let tools = cx.try_global::<crate::tools::ToolState>();
    let field_kind = tools.map_or(0, |tools| match tools.field_kind {
        core::NewField::Text => 0,
        core::NewField::CheckBox => 1,
        core::NewField::Radio { .. } => 2,
        core::NewField::Choice { list: false, .. } => 3,
        core::NewField::Choice { list: true, .. } => 4,
    });
    let stamp = match tools.map(|tools| &tools.stamp) {
        Some(core::StampLabel::Custom { text, color }) => (
            text.clone(),
            COLOURS.iter().position(|(_, c)| c == color).unwrap_or(0),
        ),
        _ => (String::new(), 2),
    };
    let doc = tab.read(cx);
    let count = doc.page_count();
    // The pages selected in the Organize grid, or all of them.
    let all = ranges::format(&crate::organize::offered_pages(doc));
    let extract = if doc.grid.on {
        let mut pages = crate::organize::target_pages(doc);
        pages.sort_unstable();
        ranges::format(&pages)
    } else {
        ranges::format(&(0..count).collect::<Vec<_>>())
    };
    let bytes_meta = (kind == Kind::Properties).then(|| {
        guarded(|| core::read_metadata(doc.session.bytes()))
            .map_err(|err| format!("Cannot read properties: {err}"))
    });
    let mut text = |label, value: &str, hint: &str| {
        form::text(label, value.to_owned(), hint.to_owned(), window, cx)
    };
    let fields = match kind {
        Kind::GoToPage => vec![text("Page", "", &format!("1 to {count}"))],
        Kind::NumberPages => vec![
            form::choice("Position", Place::ALL.map(Place::label), 0),
            form::choice("Format", NUMBER_FORMATS.map(number_sample), 0),
            text("First number", "1", ""),
            text("Size (pt)", "10", ""),
            form::choice("Colour", COLOURS.map(|(name, _)| name), 0),
            text(PAGES, &all, PAGES_HINT),
        ],
        Kind::Watermark => vec![
            form::choice(MARK, ["Text", "Picture"], 0),
            text("Text", "", "e.g. DRAFT"),
            text(SCALE, "50", ""),
            text("Size (pt)", "64", ""),
            text("Opacity (%)", "30", ""),
            text("Angle (°)", "45", ""),
            form::choice("Colour", COLOURS.map(|(name, _)| name), 1),
            text("Right of centre (pt)", "0", ""),
            text("Above centre (pt)", "0", ""),
            text(PAGES, &all, PAGES_HINT),
        ],
        Kind::Crop => vec![
            text("Left (mm)", "10", ""),
            text("Top (mm)", "10", ""),
            text("Right (mm)", "10", ""),
            text("Bottom (mm)", "10", ""),
            text(PAGES, &all, PAGES_HINT),
        ],
        Kind::PageSize => vec![
            form::choice("Size", PAPERS.map(|(name, _)| name), 0),
            form::choice("Orientation", ["Portrait", "Landscape"], 0),
            text(PAGES, &all, PAGES_HINT),
        ],
        Kind::PerSheet => vec![form::choice(
            "Pages on a sheet",
            PER_SHEET.map(|count| count.to_string()),
            0,
        )],
        Kind::Halve => vec![
            form::choice("Cut", ["Left | right", "Top / bottom"], 0),
            text(PAGES, &all, PAGES_HINT),
        ],
        Kind::Colours => vec![
            form::choice("Colours", ["Invert", "Greyscale", "Paper colour"], 0),
            text(PAGES, &all, PAGES_HINT),
        ],
        Kind::Images => vec![
            form::choice("Format", ["PNG", "JPEG"], 0),
            form::choice("Sharpness", DPI.map(|dpi| format!("{dpi} dpi")), 1),
            text(PAGES, &all, PAGES_HINT),
        ],
        Kind::Extract => vec![text(PAGES, &extract, PAGES_HINT)],
        Kind::Split => vec![
            form::choice("Split", ["Every few pages", "At pages"], 0),
            text(PER_PART, "1", ""),
            text(STARTS_AT, "", "e.g. 4, 9"),
        ],
        Kind::Encrypt => vec![
            form::password("Open password", "", window, cx),
            form::password("Confirm", "", window, cx),
            form::password(OWNER, "optional: lifts the limits", window, cx),
            form::choice("Printing", ALLOW, 0),
            form::choice("Copying text", ALLOW, 0),
            form::choice("Editing", ALLOW, 0),
            form::choice("Comments & forms", ALLOW, 0),
        ],
        Kind::NewField { .. } => vec![
            form::choice("Kind", FIELD_KINDS, field_kind),
            text("Name", "Field", ""),
            text(OPTIONS, "", "e.g. Yes, No, Maybe"),
        ],
        Kind::HeaderFooter => vec![
            text("Header left", "", ""),
            text("Header centre", "", ""),
            text("Header right", "", "e.g. {date}"),
            text("Footer left", "", "e.g. {file}"),
            text("Footer centre", "Page {n} of {total}", ""),
            text("Footer right", "", ""),
            text("First number", "1", ""),
            text("Size (pt)", "10", ""),
            form::choice("Colour", COLOURS.map(|(name, _)| name), 0),
            text(PAGES, &all, PAGES_HINT),
        ],
        Kind::Bates => vec![
            text("Prefix", "", "e.g. ACME-"),
            text("Start at", "1", ""),
            text("Digits", "6", ""),
            form::choice("Position", Place::ALL.map(Place::label), 2),
            text("Size (pt)", "10", ""),
            form::choice("Colour", COLOURS.map(|(name, _)| name), 0),
            text(PAGES, &all, PAGES_HINT),
        ],
        Kind::CustomStamp => vec![
            text("Text", &stamp.0, "e.g. PAID"),
            form::choice("Colour", COLOURS.map(|(name, _)| name), stamp.1),
        ],
        Kind::Compress => vec![
            form::choice(
                "Pictures",
                PICTURE_DPI.map(|dpi| {
                    dpi.map_or("Keep as they are".to_owned(), |dpi| format!("{dpi} dpi"))
                }),
                2,
            ),
            form::choice("JPEG quality", ["Low", "Medium", "High"], 1),
        ],
        Kind::PdfA => Vec::new(),
        Kind::Word => vec![form::choice("Pictures", ["Include", "Leave out"], 0)],
        Kind::NewLink { .. } => vec![text(LINK_TO, "", "Page number or https://…")],
        Kind::Properties => {
            let meta = bytes_meta.unwrap_or_else(|| Ok(Metadata::default()))?;
            let values = [
                meta.title,
                meta.author,
                meta.subject,
                meta.keywords,
                meta.creator,
                meta.producer,
            ];
            META_LABELS
                .into_iter()
                .zip(values)
                .map(|(label, value)| text(label, &value.unwrap_or_default(), ""))
                .collect()
        }
    };
    Ok(Form {
        kind,
        tab: tab.downgrade(),
        fields,
        error: (kind == Kind::PdfA)
            .then(|| convert::pdfa_warnings(cx))
            .flatten(),
    })
}

/// Do what the dialog says. `Err` keeps it open showing why.
pub fn confirm(form: &Form, secondary: bool, cx: &mut App) -> Result<(), String> {
    let tab = form
        .tab
        .upgrade()
        .ok_or("The document was closed meanwhile")?;
    let count = tab.read(cx).page_count();
    let pages = || ranges::parse(&form.text(PAGES, cx), count);
    let num = |label, range| number(&form.text(label, cx), label, range);
    let colour = || COLOURS[form.choice("Colour").min(COLOURS.len() - 1)].1;
    match form.kind {
        Kind::GoToPage => {
            let page = num("Page", 1.0..=count as f32)? as usize;
            tab.update(cx, |tab, cx| {
                tab.scroll_to_page(page - 1);
                cx.notify();
            });
            Ok(())
        }
        Kind::NumberPages => {
            let numbers = PageNumbers {
                place: Place::ALL[form.choice("Position").min(Place::ALL.len() - 1)],
                template: NUMBER_FORMATS[form.choice("Format")].to_owned(),
                first: num("First number", 0.0..=99_999.0)? as usize,
                font_size: num("Size (pt)", 6.0..=72.0)?,
                color: colour(),
            };
            let pages = pages()?;
            apply(&tab, "Number pages", "Pages numbered", cx, |pdf| {
                core::number_pages(pdf, &pages, &numbers)
            })
        }
        Kind::Watermark if form.choice(MARK) == 1 => {
            let opacity = num("Opacity (%)", 5.0..=100.0)? / 100.0;
            let scale = num(SCALE, 5.0..=100.0)? / 100.0;
            let pages = pages()?;
            picture_watermark(tab, pages, opacity, scale, cx);
            Ok(())
        }
        Kind::Watermark => {
            let text = form.text("Text", cx);
            if text.trim().is_empty() {
                return Err("Type the text of the watermark".to_owned());
            }
            let mark = Watermark {
                text,
                font_size: num("Size (pt)", 8.0..=300.0)?,
                opacity: num("Opacity (%)", 5.0..=100.0)? / 100.0,
                angle: num("Angle (°)", -90.0..=90.0)?,
                color: colour(),
                offset: (
                    num("Right of centre (pt)", -2000.0..=2000.0)?,
                    num("Above centre (pt)", -2000.0..=2000.0)?,
                ),
            };
            let pages = pages()?;
            apply(&tab, "Add watermark", "Watermark added", cx, |pdf| {
                core::watermark(pdf, &pages, &mark)
            })
        }
        Kind::Crop => {
            let mut margins = [0.0; 4];
            for (margin, label) in
                margins
                    .iter_mut()
                    .zip(["Left (mm)", "Top (mm)", "Right (mm)", "Bottom (mm)"])
            {
                *margin = num(label, 0.0..=500.0)? * MM;
            }
            let pages = pages()?;
            apply(&tab, "Crop pages", "Pages cropped", cx, |pdf| {
                core::crop_pages(pdf, &pages, margins)
            })
        }
        Kind::PageSize => {
            let (_, (width, height)) = PAPERS[form.choice("Size")];
            let (width, height) = if form.choice("Orientation") == 1 {
                (height, width)
            } else {
                (width, height)
            };
            let pages = pages()?;
            apply(&tab, "Change page size", "Pages resized", cx, |pdf| {
                core::resize_pages(pdf, &pages, width, height)
            })
        }
        Kind::PerSheet => {
            let per_sheet = PER_SHEET[form.choice("Pages on a sheet")];
            apply(
                &tab,
                "Pages per sheet",
                "Pages put together on sheets",
                cx,
                |pdf| core::pages_per_sheet(pdf, per_sheet),
            )
        }
        Kind::Halve => {
            let across = form.choice("Cut") == 0;
            let pages = pages()?;
            apply(&tab, "Cut pages in half", "Pages cut in half", cx, |pdf| {
                core::halve_pages(pdf, &pages, across)
            })
        }
        Kind::Colours => {
            let tint = match form.choice("Colours") {
                0 => Tint::Invert,
                1 => Tint::Greyscale,
                _ => Tint::Paper(Color {
                    r: 1.0,
                    g: 0.96,
                    b: 0.84,
                }),
            };
            let pages = pages()?;
            apply(&tab, "Page colours", "Colours changed", cx, |pdf| {
                core::tint_pages(pdf, &pages, tint)
            })
        }
        Kind::Images => {
            let pages = pages()?;
            let jpeg = form.choice("Format") == 1;
            let dpi = DPI[form.choice("Sharpness")];
            let doc = tab.read(cx);
            let (bytes, base) = (doc.session.bytes().to_vec(), stem(doc.title()).to_owned());
            output::export_images(bytes, pages, dpi, jpeg, base, cx);
            Ok(())
        }
        Kind::Extract => {
            let pages = pages()?;
            let doc = tab.read(cx);
            let bytes = guarded(|| core::extract_pages(doc.session.bytes(), &pages))
                .map_err(|err| plain(err.to_string()))?;
            let range = form.text(PAGES, cx);
            let mut name = format!("{} (pages {}).pdf", stem(doc.title()), range.trim());
            if name.chars().count() > 80 {
                name = format!("{} (selection).pdf", stem(doc.title()));
            }
            if secondary {
                crate::files::save_new_pdf(cx, &name, bytes, |_, cx| {
                    toasts::success(cx, "Pages exported");
                });
            } else {
                open_tab(name, bytes, cx);
            }
            Ok(())
        }
        Kind::Split => {
            let every = form.choice("Split") == 0;
            let starts = if every {
                Vec::new()
            } else {
                ranges::parse(&form.text(STARTS_AT, cx), count)?
            };
            let n = if every {
                num(PER_PART, 1.0..=10_000.0)? as usize
            } else {
                0
            };
            let doc = tab.read(cx);
            let (bytes, base) = (doc.session.bytes().to_vec(), doc.title().to_owned());
            output::split(bytes, every, n, starts, base, !secondary, cx);
            Ok(())
        }
        Kind::Encrypt => {
            let user = form.text("Open password", cx);
            let owner = form.text(OWNER, cx);
            let permissions = Permissions {
                print: form.choice("Printing") == 0,
                copy: form.choice("Copying text") == 0,
                edit: form.choice("Editing") == 0,
                annotate: form.choice("Comments & forms") == 0,
            };
            if user.is_empty() {
                return Err("Enter a password".to_owned());
            }
            if user != form.text("Confirm", cx) {
                return Err("Passwords do not match".to_owned());
            }
            state(cx).passwords.insert(
                tab.entity_id(),
                Passwords {
                    user,
                    owner,
                    permissions,
                },
            );
            toasts::success(cx, "Password set: the file is encrypted when you save");
            Ok(())
        }
        Kind::NewField { page, rect } => {
            let options: Vec<String> = form
                .text(OPTIONS, cx)
                .split(',')
                .map(|option| option.trim().to_owned())
                .filter(|option| !option.is_empty())
                .collect();
            let kind = match form.choice("Kind") {
                1 => core::NewField::CheckBox,
                2 => core::NewField::Radio { options },
                3 => core::NewField::Choice {
                    options,
                    list: false,
                },
                4 => core::NewField::Choice {
                    options,
                    list: true,
                },
                _ => core::NewField::Text,
            };
            let name = form.text("Name", cx);
            let done = "Field added: type its answer with Fill in form";
            apply(&tab, "Add form field", done, cx, |pdf| {
                core::add_form_field(pdf, page, rect, kind, &name)
            })
        }
        Kind::Compress => {
            let images = PICTURE_DPI[form.choice("Pictures")].map(|dpi| core::ImageOptions {
                dpi,
                quality: JPEG_QUALITY[form.choice("JPEG quality")],
            });
            super::compress(images, cx);
            Ok(())
        }
        Kind::PdfA => convert::save_pdfa(cx),
        Kind::Word => {
            let pictures = form.choice("Pictures") == 0;
            let doc = tab.read(cx);
            let (bytes, title) = (doc.session.bytes().to_vec(), doc.title().to_owned());
            convert::export_word(bytes, pictures, &title, cx);
            Ok(())
        }
        Kind::NewLink { page, rect } => {
            let target = link_target(&form.text(LINK_TO, cx), count)?;
            apply(&tab, "Add link", "Link added", cx, |pdf| {
                core::add_link(pdf, page, rect, &target)
            })
        }
        Kind::HeaderFooter => {
            let slot = |label| form.text(label, cx);
            let marks = HeaderFooter {
                header: ["Header left", "Header centre", "Header right"].map(slot),
                footer: ["Footer left", "Footer centre", "Footer right"].map(slot),
                first: num("First number", 0.0..=99_999.0)? as usize,
                date: crate::tools::today(),
                file: tab.read(cx).title().to_owned(),
                font_size: num("Size (pt)", 6.0..=72.0)?,
                color: colour(),
                ..HeaderFooter::default()
            };
            let pages = pages()?;
            apply(
                &tab,
                "Header & footer",
                "Header and footer added",
                cx,
                |pdf| core::header_footer(pdf, &pages, &marks),
            )
        }
        Kind::Bates => {
            let place = Place::ALL[form.choice("Position").min(Place::ALL.len() - 1)];
            let mut marks = HeaderFooter {
                bates: Bates {
                    prefix: form.text("Prefix", cx).trim().to_owned(),
                    start: num("Start at", 0.0..=1e9)? as u64,
                    digits: num("Digits", 1.0..=12.0)? as usize,
                },
                font_size: num("Size (pt)", 6.0..=72.0)?,
                color: colour(),
                ..HeaderFooter::default()
            };
            let slot = match place {
                Place::TopLeft | Place::BottomLeft => 0,
                Place::TopCenter | Place::BottomCenter => 1,
                Place::TopRight | Place::BottomRight => 2,
            };
            let line = match place {
                Place::TopLeft | Place::TopCenter | Place::TopRight => &mut marks.header,
                _ => &mut marks.footer,
            };
            line[slot] = "{bates}".to_owned();
            let pages = pages()?;
            apply(&tab, "Bates numbering", "Bates numbers added", cx, |pdf| {
                core::header_footer(pdf, &pages, &marks)
            })
        }
        Kind::CustomStamp => {
            let text = form.text("Text", cx).trim().to_owned();
            if text.is_empty() {
                return Err("Type the words of the stamp".to_owned());
            }
            let color = colour();
            crate::tools::use_stamp(core::StampLabel::Custom { text, color }, cx);
            Ok(())
        }
        Kind::Properties => {
            let [title, author, subject, keywords, creator, producer] = META_LABELS.map(|label| {
                Some(form.text(label, cx).trim().to_owned()).filter(|value| !value.is_empty())
            });
            let meta = Metadata {
                title,
                author,
                subject,
                keywords,
                creator,
                producer,
            };
            apply(&tab, "Edit properties", "Properties updated", cx, |pdf| {
                core::write_metadata(pdf, &meta)
            })
        }
    }
}

/// Ask for a PNG or JPEG, then draw it in the middle of `pages` of `tab` (one undo step).
fn picture_watermark(
    tab: Entity<DocTab>,
    pages: Vec<usize>,
    opacity: f32,
    scale: f32,
    cx: &mut App,
) {
    crate::files::pick_file(cx, ("Picture", &["png", "jpg", "jpeg"]), move |path, cx| {
        let picture = std::fs::read(&path)
            .map_err(|err| err.to_string())
            .and_then(|bytes| crate::tools::decode_picture(&bytes));
        let mark = match picture {
            Ok(picture) => core::PictureWatermark {
                picture,
                opacity,
                scale,
            },
            Err(err) => return toasts::error(cx, format!("Cannot use this picture: {err}")),
        };
        let done = apply(&tab, "Add watermark", "Watermark added", cx, |pdf| {
            core::picture_watermark(pdf, &pages, &mark)
        });
        if let Err(err) = done {
            toasts::error(cx, err);
        }
    });
}

/// One undo step on `tab`; says `done` when it worked.
fn apply(
    tab: &Entity<DocTab>,
    label: &str,
    done: &str,
    cx: &mut App,
    edit: impl FnOnce(&[u8]) -> core::Result<Vec<u8>>,
) -> Result<(), String> {
    tab.update(cx, |tab, cx| {
        let result = tab.apply(label, edit);
        cx.notify();
        result
    })
    .map_err(plain)?;
    toasts::success(cx, done.to_owned());
    Ok(())
}

/// A new tab holding `bytes` (not saved anywhere yet).
pub fn open_tab(name: String, bytes: Vec<u8>, cx: &mut App) {
    match active_app(cx) {
        Some(app) => app.update(cx, |app, cx| app.open_bytes(name, None, bytes, cx)),
        None => toasts::error(cx, "Cannot open a new tab"),
    }
}

/// A number typed in the field `label`, within `range` (a comma works as decimal point).
pub fn number(text: &str, label: &str, range: RangeInclusive<f32>) -> Result<f32, String> {
    let text = text.trim();
    let value: f32 = text
        .replace(',', ".")
        .parse()
        .map_err(|_| format!("{label}: “{text}” is not a number"))?;
    if !range.contains(&value) {
        return Err(format!(
            "{label}: from {} to {}",
            range.start(),
            range.end()
        ));
    }
    Ok(value)
}

/// Where a new link leads, typed as a page number (1 to `count`) or a web or mail address;
/// an address starting with `www.` is taken as `https://`.
fn link_target(text: &str, count: usize) -> Result<core::LinkTarget, String> {
    let text = text.trim();
    if let Ok(page) = text.parse::<usize>() {
        return if (1..=count).contains(&page) {
            Ok(core::LinkTarget::Page(page - 1))
        } else {
            Err(format!("{LINK_TO}: a page from 1 to {count}"))
        };
    }
    let uri = if text.to_ascii_lowercase().starts_with("www.") {
        format!("https://{text}")
    } else {
        text.to_owned()
    };
    if core::is_web_uri(&uri) && !uri.contains(char::is_whitespace) {
        Ok(core::LinkTarget::Uri(uri))
    } else {
        Err("Type a page number, or an address starting with https://, http:// or mailto:".into())
    }
}

/// A number format as it reads on the first of twelve pages.
fn number_sample(template: &str) -> String {
    template.replace("{n}", "1").replace("{total}", "12")
}

pub fn stem(name: &str) -> &str {
    name.strip_suffix(".pdf")
        .or_else(|| name.strip_suffix(".PDF"))
        .unwrap_or(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_read_within_their_range() {
        assert_eq!(number(" 12 ", "Size", 6.0..=72.0), Ok(12.0));
        assert_eq!(number("2,5", "Size", 0.0..=10.0), Ok(2.5));
        assert!(number("abc", "Size", 0.0..=10.0).is_err());
        assert!(number("", "Size", 0.0..=10.0).is_err());
        assert!(number("11", "Size", 0.0..=10.0).is_err());
        assert!(number("-1", "Size", 0.0..=10.0).is_err());
    }

    #[test]
    fn link_targets_are_pages_or_web_addresses() {
        use godpdf_core::LinkTarget::{Page, Uri};
        assert_eq!(link_target(" 3 ", 5), Ok(Page(2)));
        assert!(link_target("0", 5).is_err() && link_target("6", 5).is_err());
        assert_eq!(
            link_target("www.example.com", 5),
            Ok(Uri("https://www.example.com".into()))
        );
        assert_eq!(
            link_target("mailto:a@b.c", 5),
            Ok(Uri("mailto:a@b.c".into()))
        );
        assert!(link_target("javascript:alert(1)", 5).is_err());
        assert!(link_target("https://a b", 5).is_err());
        assert!(link_target("", 5).is_err());
    }

    #[test]
    fn samples_and_stems() {
        assert_eq!(number_sample("Page {n} of {total}"), "Page 1 of 12");
        assert_eq!(stem("report.pdf"), "report");
        assert_eq!(stem("scan.PDF"), "scan");
        assert_eq!(stem("notes"), "notes");
    }
}
