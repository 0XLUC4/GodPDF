//! Every user-facing command: label, icon, group, shortcut. Menus, the tools panel,
//! welcome cards and the command palette are all generated from this one list.

use egui::{Color32, Key, KeyboardShortcut, Modifiers};

use crate::widgets::icon;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    Open,
    Merge,
    Save,
    SaveAs,
    Print,
    Close,
    ReopenTab,
    ExportPages,
    Quit,
    Undo,
    Redo,
    ToggleTheme,
    ZoomIn,
    ZoomOut,
    FitWidth,
    FitPage,
    GoToPage,
    SplitView,
    ToggleThumbnails,
    Organize,
    RotateLeft,
    RotateRight,
    RotateAll,
    DeletePages,
    MoveEarlier,
    MoveLater,
    InsertBlank,
    InsertFile,
    Extract,
    Split,
    NumberPages,
    Highlight,
    Note,
    FreeText,
    Rectangle,
    Ink,
    Encrypt,
    RemovePassword,
    Properties,
    ExtractText,
    EditText,
    Watermark,
    RemoveLinks,
    Find,
    Palette,
    About,
    Updates,
    ReversePages,
    CropPages,
    PagesPerSheet,
    Booklet,
    PageSize,
    HalvePages,
    JoinPages,
    RemoveBlank,
    Interleave,
    AddBookmark,
    ContentsPage,
    PageColours,
    RemoveComments,
    Flatten,
    Attachments,
    Compress,
    RemoveHiddenData,
    ExportImages,
}

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
    Help,
}

const GREEN: Color32 = Color32::from_rgb(0x3F, 0xB9, 0x50);
const PURPLE: Color32 = Color32::from_rgb(0xA3, 0x71, 0xF7);
const AMBER: Color32 = Color32::from_rgb(0xE3, 0xA0, 0x08);
const BLUE: Color32 = Color32::from_rgb(0x4C, 0x8D, 0xF6);
const TEAL: Color32 = Color32::from_rgb(0x1F, 0xB5, 0xA5);
const PINK: Color32 = Color32::from_rgb(0xE8, 0x5A, 0xAD);
const RED: Color32 = Color32::from_rgb(0xF0, 0x52, 0x52);

impl Group {
    /// The colour of the icons of its tools: one per family, to find a tool by eye. Each
    /// reads on the dark theme as on the light one.
    pub fn tint(self) -> Color32 {
        match self {
            Self::Organize => GREEN,
            Self::Combine => PURPLE,
            Self::Comment => AMBER,
            Self::Protect | Self::Edit | Self::Help => BLUE,
            Self::File | Self::View => TEAL,
            Self::Document => PINK,
        }
    }

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
            Self::Help => "Help",
        }
    }

    /// Groups shown in the Tools panel / Tools menu / welcome cards.
    pub const TOOLS: [Group; 5] = [
        Group::Organize,
        Group::Combine,
        Group::Comment,
        Group::Protect,
        Group::Document,
    ];
}

/// Comment tools: drag on a page to place an annotation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommentTool {
    Highlight,
    Note,
    FreeText,
    Rectangle,
    Ink,
}

impl CommentTool {
    pub fn hint(self) -> &'static str {
        match self {
            Self::Highlight => "Drag over the area to highlight",
            Self::Note => "Click where the sticky note goes",
            Self::FreeText => "Click on the page to type",
            Self::Rectangle => "Drag to draw a rectangle",
            Self::Ink => "Draw freehand on the page",
        }
    }
}

const CMD: Modifiers = Modifiers::COMMAND;
const CMD_SHIFT: Modifiers = Modifiers::COMMAND.plus(Modifiers::SHIFT);

impl Cmd {
    pub const ALL: [Cmd; 65] = [
        Cmd::Open,
        Cmd::Merge,
        Cmd::Save,
        Cmd::SaveAs,
        Cmd::Print,
        Cmd::Close,
        Cmd::ReopenTab,
        Cmd::ExportPages,
        Cmd::ExportImages,
        Cmd::Quit,
        Cmd::Undo,
        Cmd::Redo,
        Cmd::ToggleTheme,
        Cmd::ZoomIn,
        Cmd::ZoomOut,
        Cmd::FitWidth,
        Cmd::FitPage,
        Cmd::GoToPage,
        Cmd::SplitView,
        Cmd::ToggleThumbnails,
        Cmd::Organize,
        Cmd::RotateLeft,
        Cmd::RotateRight,
        Cmd::RotateAll,
        Cmd::DeletePages,
        Cmd::MoveEarlier,
        Cmd::MoveLater,
        Cmd::InsertBlank,
        Cmd::InsertFile,
        Cmd::Extract,
        Cmd::Split,
        Cmd::ReversePages,
        Cmd::CropPages,
        Cmd::PageSize,
        Cmd::HalvePages,
        Cmd::PagesPerSheet,
        Cmd::Booklet,
        Cmd::JoinPages,
        Cmd::RemoveBlank,
        Cmd::NumberPages,
        Cmd::Highlight,
        Cmd::Note,
        Cmd::FreeText,
        Cmd::Rectangle,
        Cmd::Ink,
        Cmd::Encrypt,
        Cmd::RemovePassword,
        Cmd::Properties,
        Cmd::ExtractText,
        Cmd::EditText,
        Cmd::Watermark,
        Cmd::RemoveLinks,
        Cmd::AddBookmark,
        Cmd::ContentsPage,
        Cmd::PageColours,
        Cmd::RemoveComments,
        Cmd::Flatten,
        Cmd::Attachments,
        Cmd::Compress,
        Cmd::RemoveHiddenData,
        Cmd::Interleave,
        Cmd::Find,
        Cmd::Palette,
        Cmd::About,
        Cmd::Updates,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "Open PDF…",
            Self::Merge => "Merge PDFs…",
            Self::Save => "Save",
            Self::SaveAs => "Save As…",
            Self::Print => "Print…",
            Self::Close => "Close tab",
            Self::ReopenTab => "Reopen closed tab",
            Self::ExportPages => "Export pages…",
            Self::Quit => "Quit",
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::ToggleTheme => "Toggle dark mode",
            Self::ZoomIn => "Zoom in",
            Self::ZoomOut => "Zoom out",
            Self::FitWidth => "Fit width",
            Self::FitPage => "Fit page",
            Self::GoToPage => "Go to page…",
            Self::SplitView => "Side by side",
            Self::ToggleThumbnails => "Toggle sidebar",
            Self::Organize => "Organize pages",
            Self::RotateLeft => "Rotate left",
            Self::RotateRight => "Rotate right",
            Self::RotateAll => "Rotate all pages",
            Self::DeletePages => "Delete pages",
            Self::MoveEarlier => "Move earlier",
            Self::MoveLater => "Move later",
            Self::InsertBlank => "Insert blank page",
            Self::InsertFile => "Insert from file…",
            Self::Extract => "Extract pages…",
            Self::Split => "Split…",
            Self::NumberPages => "Number pages…",
            Self::Highlight => "Highlight",
            Self::Note => "Sticky note",
            Self::FreeText => "Free text",
            Self::Rectangle => "Rectangle",
            Self::Ink => "Ink",
            Self::Encrypt => "Encrypt with password…",
            Self::RemovePassword => "Remove password",
            Self::Properties => "Properties…",
            Self::ExtractText => "Extract text",
            Self::EditText => "Edit PDF text",
            Self::Watermark => "Add watermark…",
            Self::RemoveLinks => "Remove all links",
            Self::Find => "Search text",
            Self::Palette => "Command palette",
            Self::About => "About GodPDF",
            Self::Updates => "Updates",
            Self::ReversePages => "Reverse page order",
            Self::CropPages => "Crop pages…",
            Self::PagesPerSheet => "Pages per sheet…",
            Self::Booklet => "Make a booklet",
            Self::PageSize => "Change page size…",
            Self::HalvePages => "Cut pages in half…",
            Self::JoinPages => "Join into one long page",
            Self::RemoveBlank => "Remove blank pages",
            Self::Interleave => "Interleave with…",
            Self::AddBookmark => "Bookmark this page",
            Self::ContentsPage => "Contents page",
            Self::PageColours => "Page colours…",
            Self::RemoveComments => "Remove all comments",
            Self::Flatten => "Flatten comments and forms",
            Self::Attachments => "Attached files…",
            Self::Compress => "Compress",
            Self::RemoveHiddenData => "Remove hidden data",
            Self::ExportImages => "Export as images…",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Open => icon::FOLDER_OPEN,
            Self::Merge => icon::STACK,
            Self::Save => icon::FLOPPY_DISK,
            Self::SaveAs => icon::FLOPPY_DISK_BACK,
            Self::Print => icon::PRINTER,
            Self::Close => icon::X,
            Self::ReopenTab => icon::CLOCK_COUNTER_CLOCKWISE,
            Self::Quit => icon::SIGN_OUT,
            Self::ExportPages | Self::Extract => icon::EXPORT,
            Self::Undo => icon::ARROW_U_UP_LEFT,
            Self::Redo => icon::ARROW_U_UP_RIGHT,
            Self::ToggleTheme => icon::CIRCLE_HALF,
            Self::ZoomIn => icon::MAGNIFYING_GLASS_PLUS,
            Self::ZoomOut => icon::MAGNIFYING_GLASS_MINUS,
            Self::FitWidth => icon::ARROWS_OUT_LINE_HORIZONTAL,
            Self::FitPage => icon::CORNERS_OUT,
            Self::GoToPage => icon::HASH,
            Self::SplitView => icon::COLUMNS,
            Self::ToggleThumbnails => icon::SIDEBAR_SIMPLE,
            Self::Organize => icon::SQUARES_FOUR,
            Self::RotateLeft => icon::ARROW_COUNTER_CLOCKWISE,
            Self::RotateRight => icon::ARROW_CLOCKWISE,
            Self::RotateAll => icon::ARROWS_CLOCKWISE,
            Self::DeletePages => icon::TRASH,
            Self::MoveEarlier => icon::CARET_LEFT,
            Self::MoveLater => icon::CARET_RIGHT,
            Self::InsertBlank => icon::FILE_PLUS,
            Self::InsertFile => icon::FILE_ARROW_DOWN,
            Self::Split => icon::SCISSORS,
            Self::NumberPages => icon::LIST_NUMBERS,
            Self::Highlight => icon::HIGHLIGHTER,
            Self::Note => icon::NOTE,
            Self::FreeText => icon::TEXT_T,
            Self::Rectangle => icon::RECTANGLE,
            Self::Ink => icon::PENCIL_SIMPLE,
            Self::Encrypt => icon::LOCK,
            Self::RemovePassword => icon::LOCK_OPEN,
            Self::Properties => icon::INFO,
            Self::ExtractText => icon::TEXT_ALIGN_LEFT,
            Self::EditText => icon::TEXT_AA,
            Self::Watermark => icon::STAMP,
            Self::RemoveLinks => icon::LINK_BREAK,
            Self::Find => icon::MAGNIFYING_GLASS,
            Self::Palette => icon::COMMAND,
            Self::About => icon::QUESTION,
            Self::Updates => icon::ARROW_CIRCLE_UP,
            Self::ReversePages => icon::SORT_DESCENDING,
            Self::CropPages => icon::CROP,
            Self::PagesPerSheet => icon::SQUARES_FOUR,
            Self::Booklet => icon::BOOK_OPEN,
            Self::PageSize => icon::RESIZE,
            Self::HalvePages => icon::SQUARE_SPLIT_HORIZONTAL,
            Self::JoinPages => icon::ROWS,
            Self::RemoveBlank => icon::FILE_X,
            Self::Interleave => icon::SHUFFLE,
            Self::AddBookmark => icon::BOOKMARK_SIMPLE,
            Self::ContentsPage => icon::LIST_BULLETS,
            Self::PageColours => icon::PALETTE,
            Self::RemoveComments => icon::ERASER,
            Self::Flatten => icon::STACK_SIMPLE,
            Self::Attachments => icon::PAPERCLIP,
            Self::Compress => icon::ARROWS_IN,
            Self::RemoveHiddenData => icon::BROOM,
            Self::ExportImages => icon::FILE_IMAGE,
        }
    }

    /// The colour of its icon in the lists of tools: the one of its family, save for what
    /// takes pages out (red) and what writes them elsewhere (teal).
    pub fn tint(self) -> Color32 {
        match self {
            Self::DeletePages => RED,
            Self::Extract | Self::ExportPages | Self::ExtractText | Self::ExportImages => TEAL,
            Self::RemoveBlank | Self::RemoveComments | Self::RemoveHiddenData => RED,
            Self::Find | Self::Properties => BLUE,
            other => other.group().tint(),
        }
    }

    /// Compact label for the top ribbon.
    pub fn short_label(self) -> &'static str {
        match self {
            Self::Organize => "Organize",
            Self::DeletePages => "Delete",
            Self::InsertBlank => "Blank page",
            Self::InsertFile => "Insert file",
            Self::Extract => "Extract",
            Self::Split => "Split",
            Self::NumberPages => "Number pages",
            Self::Watermark => "Watermark",
            Self::Merge => "Merge",
            Self::Encrypt => "Set password",
            Self::ExtractText => "Text",
            Self::PagesPerSheet => "Per sheet",
            Self::HalvePages => "Cut in half",
            Self::JoinPages => "One page",
            Self::ReversePages => "Reverse",
            Self::RemoveBlank => "Blank pages",
            Self::PageColours => "Colours",
            Self::RemoveHiddenData => "Hidden data",
            other => other.label().trim_end_matches('…'),
        }
    }

    /// One-line description for welcome cards and palette.
    pub fn description(self) -> &'static str {
        match self {
            Self::Organize => "Reorder, rotate and delete pages in a grid",
            Self::RotateLeft | Self::RotateRight => "Turn the current page by 90°",
            Self::RotateAll => "Turn every page by 90°",
            Self::Updates => "New versions of GodPDF and what they bring",
            Self::DeletePages => "Remove pages for good",
            Self::MoveEarlier => "Move the selected pages one place earlier",
            Self::MoveLater => "Move the selected pages one place later",
            Self::InsertBlank => "Add an empty page",
            Self::InsertFile => "Insert another PDF's pages",
            Self::Extract => "Copy pages into a new PDF",
            Self::Split => "Cut a PDF into several files",
            Self::Merge => "Combine PDFs, images and text files into one",
            Self::NumberPages => "Write its number on every page",
            Self::Watermark => "Draw a line of text across the pages",
            Self::RemoveLinks => "Take every link out, keep what they were on",
            Self::ReopenTab => "Open again the document closed last",
            Self::Highlight => "Mark important passages",
            Self::Note => "Pin a sticky note",
            Self::FreeText => "Type text onto the page",
            Self::Rectangle => "Frame an area",
            Self::Ink => "Draw freehand",
            Self::Encrypt => "Protect with a password",
            Self::RemovePassword => "Save without a password",
            Self::Properties => "Edit title, author, keywords",
            Self::ExtractText => "Copy all text out",
            Self::EditText => "Click a line of the page to change its words",
            Self::Find => "Find words with per-page counts",
            Self::SplitView => "Two PDFs, or two places of one, next to each other",
            Self::ReversePages => "Last page first",
            Self::CropPages => "Hide the margins of the pages",
            Self::PagesPerSheet => "2, 4, 6, 8, 9 or 16 pages on each sheet",
            Self::Booklet => "Print both sides, fold in the middle",
            Self::PageSize => "A4, Letter, A3… what the pages show is scaled to fit",
            Self::HalvePages => "Each page into two: scans of two pages side by side",
            Self::JoinPages => "Every page one under the other, on one page",
            Self::RemoveBlank => "Find the empty pages and take them out",
            Self::Interleave => "Take the pages of two PDFs in turn: fronts and backs of a scan",
            Self::AddBookmark => "A bookmark for the page you are reading",
            Self::ContentsPage => "A first page listing the bookmarks, each line a link",
            Self::PageColours => "Invert for night reading, greyscale, or tint the paper",
            Self::RemoveComments => "Highlights, notes and drawings go; links stay",
            Self::Flatten => "Draw comments and form fields into the page for good",
            Self::Attachments => "Files carried inside the PDF: add, save, remove",
            Self::Compress => "A smaller file that shows the same",
            Self::RemoveHiddenData => "Properties, scripts and attached files go",
            Self::ExportImages => "Each page as a PNG or JPEG picture",
            _ => "",
        }
    }

    pub fn group(self) -> Group {
        match self {
            Self::Open
            | Self::Save
            | Self::SaveAs
            | Self::Print
            | Self::Close
            | Self::ReopenTab
            | Self::ExportPages
            | Self::ExportImages
            | Self::Quit => Group::File,
            Self::Undo | Self::Redo | Self::Palette => Group::Edit,
            Self::ToggleTheme
            | Self::ZoomIn
            | Self::ZoomOut
            | Self::FitWidth
            | Self::FitPage
            | Self::GoToPage
            | Self::SplitView
            | Self::ToggleThumbnails => Group::View,
            Self::Organize
            | Self::RotateLeft
            | Self::RotateRight
            | Self::RotateAll
            | Self::DeletePages
            | Self::MoveEarlier
            | Self::MoveLater
            | Self::InsertBlank
            | Self::InsertFile
            | Self::Extract
            | Self::Split
            | Self::NumberPages
            | Self::ReversePages
            | Self::CropPages
            | Self::PagesPerSheet
            | Self::Booklet
            | Self::PageSize
            | Self::HalvePages
            | Self::JoinPages
            | Self::RemoveBlank => Group::Organize,
            Self::Merge | Self::Interleave => Group::Combine,
            Self::Highlight | Self::Note | Self::FreeText | Self::Rectangle | Self::Ink => {
                Group::Comment
            }
            Self::Encrypt | Self::RemovePassword | Self::RemoveHiddenData => Group::Protect,
            Self::Properties
            | Self::ExtractText
            | Self::EditText
            | Self::Watermark
            | Self::RemoveLinks
            | Self::Find
            | Self::AddBookmark
            | Self::ContentsPage
            | Self::PageColours
            | Self::RemoveComments
            | Self::Flatten
            | Self::Attachments
            | Self::Compress => Group::Document,
            Self::About | Self::Updates => Group::Help,
        }
    }

    pub fn comment_tool(self) -> Option<CommentTool> {
        match self {
            Self::Highlight => Some(CommentTool::Highlight),
            Self::Note => Some(CommentTool::Note),
            Self::FreeText => Some(CommentTool::FreeText),
            Self::Rectangle => Some(CommentTool::Rectangle),
            Self::Ink => Some(CommentTool::Ink),
            _ => None,
        }
    }

    /// Needs an open document to run.
    pub fn needs_doc(self) -> bool {
        !matches!(
            self,
            Self::Open
                | Self::Merge
                | Self::Quit
                | Self::ReopenTab
                | Self::ToggleTheme
                | Self::Palette
                | Self::About
                | Self::Updates
                | Self::ToggleThumbnails
        )
    }

    /// A tool: chosen with no document open, it asks for one and runs on it.
    pub fn asks_for_doc(self) -> bool {
        self.needs_doc() && Group::TOOLS.contains(&self.group())
    }

    pub fn shortcut(self) -> Option<KeyboardShortcut> {
        let (mods, key) = match self {
            Self::Open => (CMD, Key::O),
            Self::Save => (CMD, Key::S),
            Self::SaveAs => (CMD_SHIFT, Key::S),
            Self::Print => (CMD, Key::P),
            Self::Close => (CMD, Key::W),
            Self::ReopenTab => (CMD_SHIFT, Key::T),
            Self::Undo => (CMD, Key::Z),
            Self::Redo => (CMD, Key::Y),
            Self::ZoomIn => (CMD, Key::Plus),
            Self::ZoomOut => (CMD, Key::Minus),
            Self::Find => (CMD, Key::F),
            Self::GoToPage => (CMD, Key::G),
            Self::AddBookmark => (CMD, Key::D),
            Self::Palette => (CMD, Key::K),
            Self::FitWidth => (CMD, Key::Num0),
            Self::MoveEarlier => (Modifiers::ALT, Key::ArrowLeft),
            Self::MoveLater => (Modifiers::ALT, Key::ArrowRight),
            _ => return None,
        };
        Some(KeyboardShortcut::new(mods, key))
    }

    /// Shortcuts to check each frame. More modifiers first: `consume_shortcut` matches
    /// Ctrl+Shift+S against a plain Ctrl+S pattern otherwise.
    pub fn key_bindings() -> Vec<(KeyboardShortcut, Cmd)> {
        let mut bindings: Vec<(KeyboardShortcut, Cmd)> = Self::ALL
            .iter()
            .filter_map(|cmd| cmd.shortcut().map(|sc| (sc, *cmd)))
            .collect();
        bindings.push((KeyboardShortcut::new(CMD_SHIFT, Key::Z), Cmd::Redo));
        bindings.push((KeyboardShortcut::new(CMD, Key::Equals), Cmd::ZoomIn));
        bindings.sort_by_key(|(sc, _)| std::cmp::Reverse(modifier_count(sc.modifiers)));
        bindings
    }
}

fn modifier_count(mods: Modifiers) -> u8 {
    u8::from(mods.shift) + u8::from(mods.alt) + u8::from(mods.command || mods.ctrl)
}

/// Commands of a group, in declaration order.
pub fn in_group(group: Group) -> impl Iterator<Item = Cmd> {
    Cmd::ALL.into_iter().filter(move |cmd| cmd.group() == group)
}
