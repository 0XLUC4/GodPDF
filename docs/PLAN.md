# GodPDF — Plan

Pure-Rust PDF workbench.
Local-only: no account, no server, no telemetry. Desktop (Win/macOS/Linux) + Web (WASM).

## Stack

| Layer | Crate | Why |
|-------|-------|-----|
| Object model / edits | `lopdf` 0.45 | Mature pure-Rust PDF read/write |
| Rendering | `hayro` 0.8 | Most complete pure-Rust rasterizer |
| UI | `eframe`/`egui` 0.36 | One codebase → native + web |
| File dialogs | `rfd` | Native + web |

## Workspace

```
crates/
  core/    godpdf-core    lopdf ops over &[u8] -> Vec<u8>, Session (undo/redo). NO UI deps.
  render/  godpdf-render  hayro wrapper: bytes -> RGBA pixmaps, page sizes. NO UI deps.
  app/     godpdf         eframe app (bin). Depends on core + render.
assets/    logo.png, favicon.png
```

Rule: core + render never depend on UI. All edit ops are pure `fn(&[u8], ...) -> Result<Vec<u8>>`.
Undo = stack of byte snapshots (KISS, correct, cheap enough for normal PDFs).

## Features — v1 (this build)

- **View**: open many PDFs in tabs, continuous scroll, zoom (fit width / %), thumbnails sidebar, dark/light theme
- **Go to page** (Ctrl+G): type a page number; the rail keeps its small page field
- **Side by side** (View): a second viewer on the right, showing another open PDF or another place of the same one (pick the document and the page in its header); it is for reading, tools act on the first
- **Organize**: page grid, drag reorder, arrow reorder (card / sidebar arrows, Alt+Left/Right), rotate, delete, insert blank, insert from file, extract, split (every N / at pages)
- **Combine**: merge N PDFs
- **Metadata**: view/edit title, author, subject, keywords
- **Security**: open encrypted (password prompt), encrypt with password, remove password
- **Comment**: highlight, sticky note, free text (click to type on the page; click an existing box or double-click it in Read mode to edit/move it), rectangle, ink (written as real PDF annotations)
- **Text**: extract text, search with per-page counts, every match marked on the pages
- **Edit PDF text** (Tools): click a line of the page to change its words. The old words are taken out of the page's content, the new ones written where they started, at their size and in their colour, in Helvetica
- **Outline**: bookmarks panel (read)
- **History**: named undo/redo, dirty indicator, Save / Save As
- **Command palette** (Ctrl+K) listing every tool
- **Branding**: logo in window icon, welcome screen, about dialog
- **Icons**: Phosphor (regular) via egui-phosphor 0.13; fixed-size icon buttons (`widgets::icon_button`)

## v2 (later)

Forms filling, layers toggle, attachments, redaction, digital signatures, CLI + MCP server, i18n (FR/EN).

Known gaps from review (deferred):
- Edit PDF text works a line at a time, through a box, and always writes in Helvetica (WinAnsi letters only). Text inside form XObjects, or turned / slanted, is refused with a message.
- Search counts (lopdf text) and the marks on the page (hayro text) come from two readers: on unusual fonts they may differ.
- Side by side: both viewers of one document share its zoom.
- Merge/insert drop the inputs' `/AcroForm` and `/Names` (forms + named link dests in merged docs).
- `write_metadata` does not sync the XMP `/Metadata` stream or `ModDate`.
- Highlight is a dragged rectangle (no text positions yet).
- Web build compiles but has no host page / packaging (trunk + Cloudflare Pages).

## Multi-agent execution

| Phase | Agent | Owns | Parallel? |
|-------|-------|------|-----------|
| 0 | lead | plan, workspace, API contracts (stub signatures) | — |
| 1a | core-builder | `crates/core` impl + tests | yes |
| 1b | render-builder | `crates/render` impl + tests | yes |
| 1c | ui-builder | `crates/app` full UI against contracts | yes |
| 2 | lead | integrate, `cargo build/test/clippy` green | — |
| 3 | rust-reviewer | review all crates, findings | — |
| 4 | lead | apply fixes, final verification | — |

Agents may not change another agent's crate or the public contract signatures.
Contract changes go through the lead.

## Done criteria

- `cargo build --release` ok, `cargo test` ok, `cargo clippy -- -D warnings` ok
- App opens a real PDF, renders, reorders/rotates/deletes pages, merges, saves, undo works
