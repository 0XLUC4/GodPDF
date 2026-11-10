# godpdf-gpui architecture

GodPDF desktop UI on [GPUI](https://gpui.rs), through **GPUI Kit 0.9.1** (`gpui-kit` on
crates.io). The Kit bundles the `gpui-pre` 0.3.12 snapshot of Zed's gpui, its platform
layer, `gpui-component` (widgets: input, dialog, popover, list...) and the Lucide icons.
It is aliased as `gpui` in `Cargo.toml`, so:

- `gpui::*` is GPUI itself (`div`, `Render`, `Entity`, `actions!`...).
- `gpui::component::*` is gpui-component, e.g. `gpui::component::input::{Input, InputState}`.
- `gpui::assets::IconName` is the Lucide catalog (the assets are already registered in `main`).

All PDF work is in `godpdf-core` (edits, bytes in -> bytes out) and `godpdf-render`
(rasterize, text positions). This crate is UI only. The egui app (`crates/app`) is the
reference for behavior: port from it.

## Module map

| Module | Role |
|---|---|
| `main.rs` | Starts the app, opens the window (gpui-kit `Root` around `GodPdfApp`), quit-on-close. |
| `app.rs` | `GodPdfApp`, the root view: tool bar row, tab row, body, welcome screen. App actions + key bindings (`register`). Open / save / close / undo / redo / zoom handlers. |
| `doc.rs` | `DocTab` (one `Entity` per open document): `Session`, renderer, page bitmap cache, `ViewState` (zoom, scroll, viewer bounds). |
| `viewer.rs` | `impl Render for DocTab`: the continuous-scroll viewer, wheel scroll / Ctrl+wheel zoom, scrollbar. |
| `geometry.rs` | Pure math: page `Layout`, visible pages, zoom anchoring, displayed points <-> PDF space. Tested. |
| `theme.rs` | Palette of the egui app (dark / light) as a `Global`; `Palette::of(cx)`; `toggle`. |
| `toasts.rs` | `Global` toast stack: `toasts::error(cx, ..)`, `info`, `success`. |
| `files.rs` | rfd file dialogs, read, atomic write. |
| `widgets.rs` | `button`, `action_button`, `primary_button`, `separator` in the palette. |
| `tools/` | **Hook** (empty): tool bar + per-page canvas interactions. |
| `panels/` | **Hook** (empty): left sidebar, right panels. |
| `dialogs/` | **Hook** (empty): modal dialogs, command palette. |

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
4. A button runs it with `widgets::action_button(id, label, MyCommand, &palette)` or
   `window.dispatch_action(Box::new(MyCommand), cx)`.

Current bindings (same as egui): Ctrl+O open, Ctrl+S save, Ctrl+Shift+S save as, Ctrl+W
close tab, Ctrl+Tab / Ctrl+Shift+Tab switch tab, Ctrl+Z undo, Ctrl+Y / Ctrl+Shift+Z redo,
Ctrl+= / Ctrl++ zoom in, Ctrl+- zoom out, Ctrl+0 fit width, Ctrl+Q quit. Ctrl+wheel zooms
around the pointer.

## A tool

`tools/mod.rs`:

- Keep the active tool in a `Global` (e.g. `struct ToolState { tool: Tool }`), switched by
  actions (V, H, T, N, D, R, E, I in the egui app, see `crates/app/src/quickbar.rs`).
- `toolbar(window, cx)` returns the tool bar row (shown under the main tool bar).
- `page_overlay(tab, page, rect, window, cx)` is called by the viewer for every visible
  page. Return an element sized to the page: `div().absolute().size_full()`, with mouse
  handlers. Keep gesture state in the `Global` or add a field to `DocTab`.

## A panel

`panels/mod.rs`: `left(active_tab, window, cx)` / `right(..)` return the panel, laid out left
and right of the viewer in a flex row (give it a fixed width). Keep which panel is open in a
`Global`. Read the document with `tab.read(cx)`; jump to a page with
`tab.update(cx, |tab, cx| { tab.scroll_to_page(page); cx.notify() })`.
Thumbnails: render with `godpdf_render::Document::thumbnail` on
`cx.background_executor()`, as `DocTab::request` does for pages.

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
at 4096 px a side; up to 4 render at once per tab on the background executor, and pages
more than 3 away from the visible ones are evicted (their GPU textures freed).

## Not in wave 0

Password dialog (encrypted files needing a password are refused with a toast), recent
files, persistence of theme / window size, print, every tool, panel and dialog.
