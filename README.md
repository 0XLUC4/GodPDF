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
- Combine: merge any number of PDFs
- Comment: highlight in fluo colors, sticky note, free text, rectangle, ink; comments can be moved, resized and erased
- Edit the text of a page, line by line
- Protect: open encrypted PDFs, add or remove a password
- Print, document properties, text extraction, undo/redo history, command palette (`Ctrl+K`)

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
