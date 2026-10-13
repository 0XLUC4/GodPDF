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
- Large pages: plans in A3 to A0 stay sharp at any zoom
- Organize: drag-and-drop reorder, rotate one page or all, delete, insert blank or from file, extract, split
- Combine: merge any number of PDFs, images (JPEG, PNG) and text files
- Open a picture or a text file as a PDF
- Stamp: page numbers (position, format, first number) and watermarks (size, opacity, angle, offset from the centre), upright on turned pages
- Comment: highlight in fluo colors, sticky note, free text (Helvetica, Times or Courier; bold, italic, underline, subscript, superscript), rectangle, ink; comments can be moved, resized and erased
- Edit the text of a page, line by line
- Remove every link of a document
- Protect: open encrypted PDFs, add or remove a password
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
