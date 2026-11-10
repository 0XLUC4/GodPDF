# godpdf architecture

GodPDF desktop UI on [GPUI](https://gpui.rs), through **GPUI Kit 0.9.1** (`gpui-kit` on
crates.io). The Kit bundles the `gpui-pre` 0.3.12 snapshot of Zed's gpui, its platform
layer, `gpui-component` (widgets: input, dialog, popover, list...) and the Lucide icons.
It is aliased as `gpui` in `Cargo.toml`, so:

- `gpui::*` is GPUI itself (`div`, `Render`, `Entity`, `actions!`...).
- `gpui::component::*` is gpui-component, e.g. `gpui::component::input::{Input, InputState}`.
- `gpui::assets::IconName` is the Lucide catalog (the assets are already registered in `main`).

All PDF work is in `godpdf-core` (edits, bytes in -> bytes out) and `godpdf-render`
(rasterize, text positions). This crate is UI only.

## Module map

| Module | Role |
|---|---|
| `main.rs` | Starts the app, opens the window (gpui-kit `Root` around `GodPdfApp`), quit-on-close. |
| `app.rs` | `GodPdfApp`, the root view (UI font on the root). App actions + key bindings (`register`). Open / save / close / undo / redo / zoom handlers. |
| `shell/` | Window chrome, one file per region: `tabrow.rs`, `header.rs`, `leftpanel.rs` (tools panel), `rail.rs` (right icon rail), `banner.rs`, `welcome.rs`; `Shell` state (mode, tools panel, home tab). Measurements: DESIGN.md. |
| `fonts.rs` / `icons.rs` | Bundled fonts (Ubuntu Light, Hack, Phosphor), loaded in `main`; Phosphor glyph constants under their Phosphor names. |
| `doc.rs` | `DocTab` (one `Entity` per open document): `Session`, renderer, page bitmap cache, `ViewState` (zoom, scroll, viewer bounds). |
| `viewer.rs` | `impl Render for DocTab`: the continuous-scroll viewer, wheel scroll / Ctrl+wheel zoom, scrollbar. |
| `geometry.rs` | Pure math: page `Layout`, visible pages, zoom anchoring, displayed points <-> PDF space. Tested. |
| `theme.rs` | Palette and visuals (dark / light) as a `Global`; `Palette::of(cx)`; `toggle`; tool colours. |
| `signature.rs` | The user's signature (strokes one unit wide, tested), kept in `<config>/GodPDF/signature.txt`; the pad dialog is `dialogs/signpad.rs`. |
| `toasts.rs` | `toasts::error(cx, ..)`: the error bar under the header (latest wins, 6 s); `note` the same bar, neutral, for an action that changed nothing; `info` / `success` are no-ops. |
| `files.rs` | rfd file dialogs, read, atomic write. |
| `widgets.rs` | Shared widgets: `icon`, `with_icon`, `icon_button`, `small_icon_button`, `rail_button`, `tool_row`, `tooltip`, `button`, `framed_button`, `accent_button`, `dispatch`. |
| `tools/` | Canvas tools: floating quick bar + style / picture cards (`quickbar.rs`), per-page select / move / resize / draw / erase / pan (`canvas.rs`), text box and sticky note typing (`editor.rs`, styled text drawn by `richtext.rs`: per-character bold / italic / underline / sub / superscript, width grip on the right edge), pure geometry (`geom.rs`, tested), form fields clicked on the page and the Field tool (`fields.rs`), the Sign tool (`sign.rs`). |
| `panels/` | Right panel opened from the rail (`shell/rail.rs`): comments by page (`comments.rs`), page thumbnails with drag reorder, move arrows and a right-click page menu (`thumbs.rs`), bookmarks tree (`bookmarks.rs`), form answers (`form.rs`), the saved signature (`sign.rs`). |
| `find/` | Browser-style find bar (Ctrl+F, rail magnifier, palette "Search text"): card at the viewer's top right, searches as you type (debounced 120 ms, background, page by page from the current page, stale searches cancelled by a token), marks every match on the pages (current one orange), Enter / F3 and Shift+Enter / Shift+F3 step and scroll to the word, Esc closes. `FindState` per tab in `DocTab::panels.find`; pure logic in `matching.rs` (tested). Hooks: `page_overlay`, `viewer_overlay`. |
| `text/` | Page text (from `textsel.rs`, `lineedit.rs`, `ocr.rs`, `compare.rs`): select with the Select tool and copy (Ctrl+C), "Edit PDF text" mode (click a line, type in place, Enter / click away writes, Esc), Compare window, Recognize text (OCR: Windows.Media.Ocr, ocrs elsewhere). Hooks: `page_underlay` / `page_overlay` (viewer, under / over the tools), `overlay` (app). |
| `organize/` | Organize page grid with selection, drag reorder and page commands (`grid.rs`, orders in `order.rs`, tested), move earlier / later (Alt+Left / Alt+Right), insert from file, remove blank pages, interleave dialog (`mod.rs`), "Side by side" second viewer (`split.rs`). |
| `import.rs` | Pictures (JPEG, PNG) and text files made into PDFs: Open, drag-drop, Combine and Insert accept them. |
| `services/` | Settings kept between sessions as JSON (`settings.rs`, `%APPDATA%\GodPDF\data\settings.json` like eframe: theme, panels, recent files, window size), GitHub updates + download-and-restart (`updates.rs`), the header's "Working…" counter (`jobs::background` instead of `background_executor().spawn` for core work), files changed by another program (`watch.rs`, every 2 s), the Updates popover of the tab row and the About / Preferences / password dialogs (`mod.rs`). `build.rs` embeds `assets/icon.ico` and the version (winresource). |
| `dialogs/` | Command list (`commands.rs`, feeds the palette and the tool bar menus), command palette Ctrl+K (`palette.rs`), form dialogs (`form.rs` rows + `kinds.rs` what each does), merge (`merge.rs`), background export / split (`output.rs`), print (`print.rs`), page ranges (`ranges.rs`). |

Each hook module has a `register(cx)` (called from `app::register`) for its actions and key
bindings, and returns `Option<AnyElement>` from its render hooks; `None` = nothing shown.
Waves work in their own module and touch `app.rs` only to add a line if needed.

## Commands (actions)

Every command is a gpui action, so a shortcut, a button and the command palette all run the
same handler:

1. Declare it: `actions!(godpdf, [MyCommand]);` in the module that owns it.
2. Bind keys in that module's `register`: `cx.bind_keys([KeyBinding::new("secondary-k", MyCommand, None)])`.
   `secondary` is Ctrl on Windows/Linux and Cmd on macOS.
3. Handle it. App-wide handlers sit on the root div in `GodPdfApp::render`
   (`.on_action(cx.listener(..))`); a module can instead handle it globally with
   `cx.on_action(|_: &MyCommand, cx| ..)` in `register` (no view access, use globals).
4. A button runs it with `.on_click(widgets::dispatch(MyCommand))` or
   `window.dispatch_action(Box::new(MyCommand), cx)`.

Current bindings: Ctrl+O open, Ctrl+S save, Ctrl+Shift+S save as, Ctrl+W
close tab, Ctrl+Tab / Ctrl+Shift+Tab switch tab, Ctrl+Z undo, Ctrl+Y / Ctrl+Shift+Z redo,
Ctrl+= / Ctrl++ zoom in, Ctrl+- zoom out, Ctrl+0 fit width, Ctrl+Q quit, Ctrl+F find,
Ctrl+D add bookmark, F3 / Shift+F3 next / previous search hit (Enter / Shift+Enter in the field). Ctrl+wheel zooms
around the pointer.

## A tool

`tools/`:

- `ToolState` (a `Global`): active tool, per-tool styles, picture card fields, and the
  document the viewer shows (set while it renders, so key actions reach it).
- `DocTab::canvas` (`tools::Canvas`): selection, gesture in progress, text being typed and
  the annotations of each page, parsed once per `DocTab::generation()`.
- Hooks: `page_overlay(tab, page, rect, window, cx)` (called by the viewer for every visible
  page) and `viewer_overlay(tab, window, cx)` (the quick bar, drawn over the viewer's
  top-left corner). `toolbar` stays `None`.
- Move / resize: nothing is written while dragging. The annotation is drawn anew at the
  pointer (text, shapes, lines, ink, markup, stamps as vectors; pictures from their own
  pixels, decoded once in the background into `Canvas::picture`) over the page rendered
  without it (`DocTab::lift` / `lifted`, rendered in the background when the pointer hovers
  it or it is selected; until then the page as it is shows under the stand-in). Page pixels
  are never moved, except for notes. The drop is one `replace_annotation` (one undo step)
  and the ghost stays until the page is rendered again (`DocTab::page_settled`).
- Right click on the selection: Bring to front / forward, Send backward / to back
  (`reorder_annotation`; Ctrl+Shift+] / Ctrl+] / Ctrl+[ / Ctrl+Shift+[), Delete.
- Eraser: a brush circle follows the pointer (`Canvas::pointer`) and stands in for it: the system pointer is hidden while over the page (`tools::pointer`, decided each frame, Windows only). "Erase elements" removes
  the comments its 14 px circle touches (the one it would take is tinted red); "Erase page
  content" brushes the page (round dabs `ToolState::brush` points wide, each 8 rectangles, `geom::brush_dabs`, the brushed area a soft grey) and
  on release `erase_content` removes the text under the path and paints it white (one undo
  step "Erase").
- Page text: with Select, Highlight, Underline and StrikeOut the canvas picks text where no
  annotation is hit (`text::select_from` / `select_to`, `Gesture::Text`; I-beam over text):
  drag by character in reading order, double click a word, triple click a line, Ctrl+A the
  page, Ctrl+C copies. A markup drag over text writes one annotation with one quad per line;
  off text it draws the old rectangle.
- Keys: V H T N D R I E, O L A (ellipse, line, arrow), U K (underline, strikeout), S stamp, X redact pick a tool, Delete / Backspace delete the selection, Esc drops it,
  all in the context `GodPdf && !Input` so they never fire while typing. In a text box
  (context `TextBox`): Ctrl+B / I / U, Ctrl+Enter writes it, Esc cancels.

## A panel

`panels/mod.rs`: `right(active_tab, window, cx)` returns the open panel, laid out between the
viewer and the rail (`shell/mod.rs`). Keep which panel is open in a
`Global`. Read the document with `tab.read(cx)`; jump to a page with
`tab.update(cx, |tab, cx| { tab.scroll_to_page(page); cx.notify() })`.
Thumbnails: render with `godpdf_render::Document::thumbnail` on
`cx.background_executor()`, as `DocTab::request` does for pages.

Done in `panels/`: `PanelState` global (open panel, tab shown) and
`DocTab::panels` (`TabPanels`: thumbnail cache, find state, outline + bookmark rename).
Thumbnails use a `uniform_list` (only visible rows render; far ones are evicted). The viewer
calls `find::page_overlay` per page for the find marks.

## A dialog

`dialogs/mod.rs`: an action opens it (state in a `Global`); `overlay(window, cx)` draws it
over the window (`div().absolute().inset_0()` + a centered card), `None` when closed.
gpui-component dialogs also work since the window root is a gpui-kit `Root`. Text fields:
`gpui::component::input::{Input, InputState}` (an `Entity<InputState>` created once, kept in
the dialog state). Native yes/no questions: `window.prompt(PromptLevel::.., ..)` as
`GodPdfApp::request_close` does.

## Edits go through `Session`

```rust
tab.update(cx, |tab, cx| {
    let result = tab.apply("Rotate page", |pdf| godpdf_core::rotate_pages(pdf, &[page], 90));
    if let Err(err) = result { toasts::error(cx, err) }
    cx.notify();
});
```

`DocTab::apply(label, edit)` runs the core edit on the current bytes (panics in core are
turned into errors by `doc::guarded`), pushes an undo step labeled `label`, then `reload`s:
the renderer is re-opened, the generation bumped, so every page re-renders (old bitmaps
stay shown meanwhile). `amend` folds a change into the last undo step (one undo for a
multi-part gesture). Undo / redo (`DocTab::undo`, `redo`) also reload. Never write
`session` bytes another way.

## Coordinate math

- **view**: logical pixels in the viewer, origin top-left (`DocTab::view.bounds` holds the
  viewer bounds in the window; event positions are window coordinates, subtract
  `bounds.origin`).
- **content** = view + `view.scroll`. `DocTab::layout()` gives each page's `PageRect` in
  content space: pages stacked with `PAD` margin and `GAP` between, centered horizontally.
- **displayed page points**: origin top-left of the page, after /Rotate (what
  `godpdf_render` reports and renders). One point is `zoom` logical pixels:
  `point = (content - rect.origin) / zoom`, `Layout::hit` does it.
- **PDF user space**: unrotated, origin bottom-left, what `godpdf_core` takes.
  `geometry::to_pdf(geom, displayed_size, point)` / `from_pdf` / `to_pdf_rect` convert, with
  `geom = tab.page_geom(page)?` and `displayed_size = tab.page_sizes[page]`.

Bitmaps render at `zoom * window.scale_factor()` pixels per point (sharp on HiDPI), capped
at 4096 px a side; a page too large for that (A3..A0 zoomed in) also gets the part on screen
rendered sharp (`DocTab::page_detail`, `render_region`) and drawn over it; up to 4 render at once per tab on the background executor, and pages
more than 3 away from the visible ones are evicted (their GPU textures freed).

## Dialogs

`dialogs/mod.rs`: page tools, protect, properties, print, merge, split / extract, export as
pictures. A dialog action records the dialog as pending in the `Dialogs` global; `overlay`
(it has the window) builds its fields and focuses the first one, remembering the focus to
give back on close. Enter / Esc in a field are `DialogConfirm` / `DialogCancel` (context
`GodPdfDialog > Input`, which outranks the field's own keys). Edits go through
`DocTab::apply`. A password set in "Encrypt with password" is applied by `app::save_tab`
through `dialogs::bytes_to_save`; the session stays unencrypted. Shortcuts: Ctrl+K palette,
Ctrl+P print, Ctrl+G go to page.

## Look

Fonts, Phosphor icons (`widgets::icon(icons::X, 16.0, color)`, never
Lucide `IconName` in new code), palette and sizes. Every region's measurements are in
DESIGN.md.

