//! Every command of the app by name (ported from `crates/app/src/commands.rs`): the command
//! palette and the tool menus are generated from [`COMMANDS`]. Each runs a gpui action, so a
//! menu entry, a palette row and a shortcut all go through the same handler.

use gpui::Action;

use super::*;
use crate::{app, tools};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    File,
    Edit,
    View,
    Organize,
    Combine,
    Comment,
    Protect,
    Document,
}

impl Group {
    pub fn title(self) -> &'static str {
        match self {
            Self::File => "File",
            Self::Edit => "Edit",
            Self::View => "View",
            Self::Organize => "Organize",
            Self::Combine => "Combine",
            Self::Comment => "Comment",
            Self::Protect => "Protect",
            Self::Document => "Document",
        }
    }
}

pub struct Command {
    pub label: &'static str,
    pub group: Group,
    /// One line for the palette.
    pub description: &'static str,
    pub action: fn() -> Box<dyn Action>,
    /// Does nothing without an open document (greyed in the palette).
    pub needs_doc: bool,
}

const fn command(
    label: &'static str,
    group: Group,
    description: &'static str,
    action: fn() -> Box<dyn Action>,
) -> Command {
    Command {
        label,
        group,
        description,
        action,
        needs_doc: true,
    }
}

impl Command {
    const fn without_doc(mut self) -> Self {
        self.needs_doc = false;
        self
    }

    /// Every word of `words` (lowercase) is in its family, name or description.
    pub fn matches(&self, words: &[&str]) -> bool {
        let haystack =
            format!("{} {} {}", self.group.title(), self.label, self.description).to_lowercase();
        words.iter().all(|word| haystack.contains(word))
    }

    /// Every word is in its name: listed before those the words only describe.
    pub fn named_by(&self, words: &[&str]) -> bool {
        let label = self.label.to_lowercase();
        words.iter().all(|word| label.contains(word))
    }
}

use Group::*;

pub static COMMANDS: &[Command] = &[
    command("Open PDF…", File, "", || Box::new(app::Open)).without_doc(),
    command("Save", File, "", || Box::new(app::Save)),
    command("Save As…", File, "", || Box::new(app::SaveAs)),
    command(
        "Print…",
        File,
        "Print with the dialog of the system",
        || Box::new(Print),
    ),
    command(
        "Export as images…",
        File,
        "Each page as a PNG or JPEG picture",
        || Box::new(ExportImages),
    ),
    command("Close tab", File, "", || Box::new(app::CloseTab)),
    command("Quit", File, "", || Box::new(app::Quit)).without_doc(),
    command("Undo", Edit, "", || Box::new(app::Undo)),
    command("Redo", Edit, "", || Box::new(app::Redo)),
    command("Toggle dark mode", View, "", || Box::new(app::ToggleTheme)).without_doc(),
    command("Zoom in", View, "", || Box::new(app::ZoomIn)),
    command("Zoom out", View, "", || Box::new(app::ZoomOut)),
    command("Fit width", View, "", || Box::new(app::FitWidth)),
    command("Go to page…", View, "", || Box::new(GoToPage)),
    command("Next tab", View, "", || Box::new(app::NextTab)),
    command("Previous tab", View, "", || Box::new(app::PreviousTab)),
    command(
        "Rotate left",
        Organize,
        "Turn the current page by 90°",
        || Box::new(RotateLeft),
    ),
    command(
        "Rotate right",
        Organize,
        "Turn the current page by 90°",
        || Box::new(RotateRight),
    ),
    command(
        "Rotate all pages",
        Organize,
        "Turn every page by 90°",
        || Box::new(RotateAll),
    ),
    command("Delete page", Organize, "Remove the current page", || {
        Box::new(DeletePage)
    }),
    command(
        "Insert blank page",
        Organize,
        "Add an empty page after this one",
        || Box::new(InsertBlankPage),
    ),
    command(
        "Extract pages…",
        Organize,
        "Copy pages into a new PDF",
        || Box::new(ExtractPages),
    ),
    command("Split…", Organize, "Cut a PDF into several files", || {
        Box::new(SplitDocument)
    }),
    command("Reverse page order", Organize, "Last page first", || {
        Box::new(ReversePages)
    }),
    command(
        "Crop pages…",
        Organize,
        "Hide the margins of the pages",
        || Box::new(CropPages),
    ),
    command(
        "Change page size…",
        Organize,
        "A4, Letter, A3… what the pages show is scaled to fit",
        || Box::new(PageSize),
    ),
    command(
        "Cut pages in half…",
        Organize,
        "Each page into two: scans of two pages side by side",
        || Box::new(HalvePages),
    ),
    command(
        "Pages per sheet…",
        Organize,
        "2, 4, 6, 8, 9 or 16 pages on each sheet",
        || Box::new(PagesPerSheet),
    ),
    command(
        "Make a booklet",
        Organize,
        "Print both sides, fold in the middle",
        || Box::new(Booklet),
    ),
    command(
        "Join into one long page",
        Organize,
        "Every page one under the other, on one page",
        || Box::new(JoinPages),
    ),
    command(
        "Number pages…",
        Organize,
        "Write its number on every page",
        || Box::new(NumberPages),
    ),
    command("Merge PDFs…", Combine, "Combine PDFs into one", || {
        Box::new(MergeFiles)
    })
    .without_doc(),
    command(
        "Select",
        Comment,
        "Select, move and resize comments",
        || Box::new(tools::SelectTool),
    ),
    command("Hand", Comment, "Drag to move around", || {
        Box::new(tools::HandTool)
    }),
    command("Highlight", Comment, "Mark important passages", || {
        Box::new(tools::HighlightTool)
    }),
    command("Sticky note", Comment, "Pin a sticky note", || {
        Box::new(tools::NoteTool)
    }),
    command("Free text", Comment, "Type text onto the page", || {
        Box::new(tools::TextTool)
    }),
    command("Rectangle", Comment, "Frame an area", || {
        Box::new(tools::RectangleTool)
    }),
    command("Ink", Comment, "Draw freehand", || Box::new(tools::InkTool)),
    command(
        "Picture",
        Comment,
        "Put a PNG or JPEG on the page, then move and resize it",
        || Box::new(tools::PictureTool),
    ),
    command(
        "Eraser",
        Comment,
        "Click or drag over comments to erase them",
        || Box::new(tools::EraserTool),
    ),
    command(
        "Encrypt with password…",
        Protect,
        "Protect with a password",
        || Box::new(SetPassword),
    ),
    command(
        "Remove password",
        Protect,
        "Save without a password",
        || Box::new(RemovePassword),
    ),
    command(
        "Remove hidden data",
        Protect,
        "Properties, scripts and attached files go",
        || Box::new(RemoveHiddenData),
    ),
    command(
        "Properties…",
        Document,
        "Edit title, author, keywords",
        || Box::new(Properties),
    ),
    command(
        "Add watermark…",
        Document,
        "Draw a line of text across the pages",
        || Box::new(AddWatermark),
    ),
    command(
        "Remove all links",
        Document,
        "Take every link out, keep what they were on",
        || Box::new(RemoveLinks),
    ),
    command(
        "Contents page",
        Document,
        "A first page listing the bookmarks, each line a link",
        || Box::new(ContentsPage),
    ),
    command(
        "Page colours…",
        Document,
        "Invert for night reading, greyscale, or tint the paper",
        || Box::new(PageColours),
    ),
    command(
        "Remove all comments",
        Document,
        "Highlights, notes and drawings go; links stay",
        || Box::new(RemoveComments),
    ),
    command(
        "Flatten comments and forms",
        Document,
        "Draw comments and form fields into the page for good",
        || Box::new(Flatten),
    ),
    command(
        "Attached files…",
        Document,
        "Files carried inside the PDF: add, save, remove",
        || Box::new(AttachedFiles),
    ),
    command(
        "Compress",
        Document,
        "A smaller file that shows the same",
        || Box::new(Compress),
    ),
];

/// Commands matching `query` (words, any case), those the words name first.
pub fn search(query: &str) -> Vec<&'static Command> {
    let query = query.to_lowercase();
    let words: Vec<&str> = query.split_whitespace().collect();
    let mut found: Vec<&Command> = COMMANDS
        .iter()
        .filter(|command| command.matches(&words))
        .collect();
    // Stable: declaration order is kept inside each half.
    found.sort_by_key(|command| !command.named_by(&words));
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_puts_names_before_descriptions() {
        assert_eq!(search("").len(), COMMANDS.len());
        let found = search("page");
        assert!(found.iter().all(|command| command.matches(&["page"])));
        // "Delete page" is named by the word; "Rotate left" only describes it.
        let delete = found.iter().position(|c| c.label == "Delete page").unwrap();
        let rotate = found.iter().position(|c| c.label == "Rotate left").unwrap();
        assert!(delete < rotate);
        assert!(search("zzzz").is_empty());
        assert_eq!(search("ORGANIZE booklet")[0].label, "Make a booklet");
    }

    #[test]
    fn labels_are_unique() {
        let mut labels: Vec<&str> = COMMANDS.iter().map(|command| command.label).collect();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), COMMANDS.len());
    }
}
