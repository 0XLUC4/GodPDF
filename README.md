<p align="center"><img src="assets/logo.png" width="160" alt="GodPDF logo"></p>

<h1 align="center">GodPDF</h1>
<p align="center">Every PDF tool. Local. Pure Rust.</p>

<p align="center"><img src="docs/screenshot.png" alt="GodPDF with a document open" width="900"></p>

A PDF workbench.
No account, no cloud, no telemetry: your documents never leave your machine.

## Download

Windows: get `GodPDF-windows-x64.exe` from the [latest release](https://github.com/0XLUC4/GodPDF/releases/latest).
It is a single file, nothing to install. It is not signed, so Windows SmartScreen may ask before running it.
New versions come through the Updates button: it downloads one and restarts on it.

## Features

- View: tabs, continuous scroll, zoom, thumbnails, bookmarks, search, two documents side by side, dark/light theme
- Bookmarks: each shows its page and leads there in a click; add one for the page you read (`Ctrl+D`), rename, move, nest, point to another page, delete; one per page in a click
- Contents page: a first page listing the bookmarks with dots and page numbers, each line a link
- Large pages: plans in A3 to A0 stay sharp at any zoom
- Organize: drag-and-drop reorder, rotate one page or all, delete, insert blank or from file, extract, split, reverse, remove blank pages
- Layout: crop margins, change page size (A3, A4, A5, Letter, Legal), cut pages in half, 2 to 16 pages per sheet, booklet for printing, join into one long page
- Colours: invert for night reading, greyscale, paper tint
- Combine: merge any number of PDFs, images (JPEG, PNG) and text files; interleave two PDFs (fronts and backs of a scan)
- Open a picture or a text file as a PDF
- Stamp: page numbers (position, format, first number) and watermarks (size, opacity, angle, offset from the centre), upright on turned pages
- Pictures: put a PNG or JPEG on a page from the tool bar (`I`), at the click or in a dragged box; move and resize it like a comment
- Comment: highlight in fluo colors, sticky note, free text (Helvetica, Times or Courier; bold, italic, underline, subscript, superscript), rectangle, ink; comments can be moved, resized and erased
- Edit the text of a page, line by line
- Remove every link of a document, remove every comment, flatten comments and form fields into the page
- Attached files: add, save, remove
- Export pages as PNG or JPEG pictures
- Compress: duplicated fonts and pictures shared, objects packed
- Protect: open encrypted PDFs, add or remove a password, remove hidden data (properties, scripts, attached files)
- Tabs: right-click to close the others, reopen the last closed (`Ctrl+Shift+T`), show the file in its folder
- A file changed by another program is read again
- Print, document properties, text extraction, undo/redo history
- Command palette (`Ctrl+K`): every tool by name; chosen with nothing open, a tool asks for a file and runs on it

## Build

Requires Rust 1.94+.

```sh
cargo run --release -p godpdf
```

## Workspace

| Crate | Role |
|-------|------|
| `crates/core` (`godpdf-core`) | Editing ops on PDF bytes via `lopdf`, undo/redo session |
| `crates/render` (`godpdf-render`) | Page rasterization via `hayro` |
| `crates/app` (`godpdf`) | `egui`/`eframe` desktop app |

## License

MIT OR Apache-2.0
