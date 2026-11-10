# godpdf-gpui design: match the egui app

The egui app (`crates/app`) is the reference look. Goal: the GPUI app looks the same,
pixel-close, in dark and light. Reference picture: `docs/screenshot.png` (dark, document open,
All tools). **egui points = GPUI px** at scale 1: copy numbers from the egui code as they are.

## Base (done)

| What | egui | GPUI |
|---|---|---|
| UI font | `Ubuntu-Light.ttf` (epaint_default_fonts 0.35), proportional | `fonts::UI` = `"Ubuntu"` + `fonts::UI_WEIGHT` (Light), set on the root div and in gpui-component's theme (`fonts::apply_to_components`) |
| Mono font | `Hack-Regular.ttf` | `fonts::MONO` = `"Hack"` |
| Icons | Phosphor regular (egui-phosphor 0.13), fallback of the proportional font | `fonts::ICONS` = `"Phosphor"`; glyph constants in `icons.rs` with the **same names** as `egui_phosphor::regular` (`icon::X` in egui = `icons::X` here) |
| Default text size | `TextStyle::Body` / `Button` 13, Heading 18, Small 9 | `fonts::BODY` = 13 on the root |
| Bold | none (`.strong()` only recolours) | Ubuntu has only Light: bold resolves to Light too |
| Files | `~/.cargo/registry/.../epaint_default_fonts-0.35.0/fonts`, `egui-phosphor-0.13.0/res` | `assets/fonts/` (+ licences), `include_bytes!`, loaded by `fonts::load` in `main` |

GPUI on Windows only falls back to *system* fonts, so a Phosphor glyph inside a text string
does not show: draw icons as their own element, `widgets::icon(glyph, size, color)` (or
`widgets::glyph(glyph)` to inherit size and colour), and `widgets::with_icon(glyph, label,
size, color)` for egui's `with_icon(glyph, label)` (glyph, gap of half the size, label).

### Palette (`theme.rs` = `crates/app/src/theme.rs`)

| Token | Dark | Light | Used for |
|---|---|---|---|
| bg | #0E0E0E | #FFFFFF | header, side panels, rail, active tab, `extreme_bg_color` |
| panel | #141414 | #FFFFFF | `panel_fill`, `window_fill`, quick bar card, noninteractive widgets |
| surface | #1C1C1C | #F5F5F5 | tab row, banner, inactive widget fill (`faint_bg`, `code_bg`) |
| hover | #262626 | #EFEFEF | hovered widget fill, `open` widget fill |
| border | #2E2E2E | #E4E4E4 | panel separator lines, widget / window strokes |
| text | #F2F2F2 | #111111 | `override_text_color` |
| text_dim | #9A9A9A | #666666 | inactive tabs, hints, noninteractive fg; disabled = text_dim at 50 % |
| accent / accent_text | #F2F2F2 | #111111 | active widget stroke, primary buttons, links, selection stroke |
| on_accent | #111111 | #FFFFFF | text on accent |
| selected | #333333 | #E6E6E6 | `selection.bg_fill`, active / pressed widget fill, selected rail button |
| canvas | #0A0A0A | #F2F2F2 | viewer behind pages |

Visuals (`theme::configure`): widget radius 7 (`theme::RADIUS`), window radius 12, menu
radius 9; window shadow offset (0, 10) blur 28 black alpha 110 (dark) / 28 (light); popup
shadow (0, 6) blur 16 alpha 3/4 of that. Hovered fg stroke 1.5. Spacing: item 8 × 6, button
padding 10 × 5, menu margin 8, window margin 16, interact height 26.

Tool colours (`commands.rs`, `theme::{GREEN, ...}`): Organize green #3FB950, Combine purple
#A371F7, Comment amber #E3A008, Protect / Edit / Help blue #4C8DF6, File / View teal #1FB5A5,
Document pink #E85AAD; delete / remove red #F05252; extract / export teal; Find, Properties
blue. Update dot blue #2563EB.

### Shared widgets (`widgets.rs` = `crates/app/src/widgets.rs`)

| egui | GPUI | Size |
|---|---|---|
| `icon_button` | `widgets::icon_button(id, glyph, tip, palette)` | 28 × 28, glyph 16, no frame until hovered (hover fill, radius 7) |
| `small_icon_button` | `widgets::small_icon_button` | 22 × 22, glyph 13 |
| `rail_button` | `widgets::rail_button(id, glyph, tip, selected, enabled, palette)` | 32 × 32, glyph 18, `selected` fill when on |
| `tool_row` | `widgets::tool_row(id, glyph, tint, label, description, palette)` | full width, 40 high (52 with description), glyph 20 in `tint` centered 20 px in, label 14 at 42 px, description 13 dim; hover fill radius 8 |
| `on_hover_text` | `widgets::tooltip(element, text)` | gpui-component tooltip |
| frameless `Button` | `widgets::button` | padding 10 × 5, height 26, text 13 |
| `icon_widget` disabled | `widgets::icon_widget(.., enabled = false, ..)` | glyph in `palette.disabled()`, no hover |

## Window layout (`crates/app/src/app.rs::ui`)

```
+--------------------------------------------------------------------+
| tab row (surface, 4 + 38)                                          |
| header (bg, 46, line under)                                        |
+-----------+--------------------------------------+---------+------+
| tools     | banner (surface, optional)           | right   | rail |
| panel     +--------------------------------------+ panel   |  52  |
| 280, bg   | canvas (viewer + floating quick bar) | 320, bg |  bg  |
+-----------+--------------------------------------+---------+------+
```

Without a document (or with the home tab in front) the body is the welcome screen on `bg`.
egui panels draw a 1 px `border` separator on their inner edge: header bottom, tools panel
right, rail left, right panel left, banner bottom. The tab row has none.

## Regions

Each region is one file; parallel work stays inside it. TODO comments in each file list what
is left.

### Tab row: `src/shell/tabrow.rs`

egui: `GodApp::tab_row`, `tab_shape`, `centered_glyph`, `doc_tab` (`app/shell.rs`).

- Frame: fill `surface`, margin left 8 / right 8 / top 4 / bottom 0, row 38 high
  (`TAB_HEIGHT`), item spacing 2, 2 px before the home tab.
- Tab shape: active = `bg`, top corners 8, from 4 px below the row top to its bottom (merges
  into the header); inactive hovered = `hover`, radius 8, 5 px in from top and bottom.
- Home: 40 × 38 tab shape, `HOUSE` 17 in `text`, 2 px below center; tooltip "Home"; active
  when no document shows.
- Document tab: width 12 + 16 + 8 + name + (14 if dirty) + 8 + 22 + 8; `FILE_PDF` 16 centered
  20 px in; name 14 at 36 px (`text` active, `text_dim` not), ellipsis at 180 (shrinks with
  many tabs, min 40); unsaved: `*` 15 after the name, tooltip "<name> (unsaved changes)";
  close `X` small icon button (22 / 13) 8 px from the right, tooltip "Close"; middle click
  closes. Context menu: Close (`X`), Close the others (`X_CIRCLE`), Reopen closed tab
  (`CLOCK_COUNTER_CLOCKWISE`), separator, Show in folder (`FOLDER_OPEN`), Copy path (`COPY`).
- 6 px gap, then "+ Open": frameless button, `PLUS` + "Open" at 14, tooltip "Open PDF (Ctrl+O)".
- Right, left to right: `ARROW_CIRCLE_UP` "Updates" (blue dot r 4 at top-right −6/+6 when an
  update is out, tooltip "Update available: <tag>"), `QUESTION` "About GodPDF", `MOON` "Dark
  mode" / `SUN` "Light mode" (28 × 28 icon buttons).
- Left to do: Updates / About actions (their egui dialogs are not ported) and the update dot.

### Header: `src/shell/header.rs`

egui: `GodApp::header_row`, `mode_tab`, `search_field`.

- Frame: fill `bg`, margin 10 × 0, height 46 (`HEADER_HEIGHT`), separator line under it.
- Menu: frameless button, `SIDEBAR_SIMPLE` + "Menu" at 14, min height 32. Popup (min width
  200): checkbox "Tools panel" (disabled without a document; turning it on leaves Read for
  All tools), separator, then the menus File / Edit / View / Tools / Help with `with_icon`
  entries and shortcut text (`GodApp::menus` in `app.rs`).
- 14 px, then the mode tabs (spacing 2): All tools, Read, Organize, Comment, Protect. Each
  text 15 + 11 px each side, full height; `text` active, `text_dim` inactive, disabled (no
  document, all but All tools) `text_dim` at 50 %; hover fill (not active) 8 px in from top
  and bottom, radius 6; active underline 2 px at the bottom, 11 px in from each side, radius
  1, `text`. Pointer cursor when enabled.
- Right, right to left with spacing 6: `PRINTER` "Print… (Ctrl+P)", `EXPORT` "Save a copy…
  (Ctrl+Shift+S)", `FLOPPY_DISK` "Save (Ctrl+S)", `INFO` "Document properties" (disabled
  without a document), 6 px, the search pill, then a spinner "Working…" while a job runs.
- Search pill: width `available - 40` clamped 140..300, height 32, radius 16, fill `bg`, 1 px
  `border` (`text_dim` when hovered), `MAGNIFYING_GLASS` 15 dim centered 18 px in, "Find
  tools and commands" 14 dim at 34 px (ellipsis), shortcut "Ctrl+K" 12 dim 14 px from the
  right (only when wider than 200). Text cursor; tooltip "Command palette (Ctrl+K)"; click
  opens the palette.
- Menu entries: `header.rs::look` gives each command its egui icon and shortcut text (by label).
- Left to do: spinner (no job list), hide the shortcut under 200 px, Preferences, About,
  View > Side by side / Toggle sidebar / Organize pages.

### Left tools panel: `src/shell/leftpanel.rs`

egui: `doc_panels` (Panel::left "tools-panel") + `tools::panel`, rows `widgets::tool_row`.

- Shown when `show_tools` and mode is not Read. Width 280 (egui resizable 220..420), fill
  `bg`, margin 12 × 6, separator on its right.
- 10 px, title 17 (`Mode::panel_title`: "All tools", "Organize pages", "Add comments",
  "Protect a PDF") + `X` icon button "Close panel" on the right, 8 px, then the scrolling
  list, rows 2 px apart.
- All tools: the 19 rows of `ALL_TOOLS` (glyph, tint, label in the file), first 11 shown,
  then 6 px and a frameless "View more" / "View less" (`text`). Tooltip = the command's
  description (none when empty). A mode row switches the header mode.
- Organize / Comment / Protect: `commands::in_group(group)` rows with `Cmd::icon`,
  `Cmd::tint`, `Cmd::short_label`; Organize adds "Combine files" (`STACK`, purple); Comment
  ends with 10 px and "Comments are listed in the panel on the right." 13 dim.
- Not ported commands (Insert from file, Organize grid, Move
  earlier / later, Remove blank pages, Sign, Form field) are listed as in egui; a click
  shows the toast "Not available yet".
- Left to do: drag to resize.

### Right rail: `src/shell/rail.rs` (+ the panel it opens: `panels::right` in `src/panels/mod.rs`)

egui: `doc_panels` (Panel::right "rail") + `GodApp::rail`, `right_panel`.

- Rail: width 52, fill `bg`, margin 0 × 10, separator on its left. Top, centered, spacing 6:
  rail buttons Comments `CHAT_TEXT`, Pages `FILES`, Bookmarks `BOOKMARK_SIMPLE`, Search
  `MAGNIFYING_GLASS`, Form `LIST_CHECKS`, Signature `SIGNATURE`; tooltip = title; click
  toggles the right panel on that view (Search focuses its field).
- Bottom, top to bottom, spacing 4: page box (DragValue 38 × 26, tooltip "Page number (drag
  or click to type)"), "/ N" 12 dim, `CARET_UP` "Previous page", `CARET_DOWN` "Next page"
  (disabled at the ends), 8 px, zoom "150%" 12 dim, `ARROWS_OUT_LINE_HORIZONTAL` "Fit width
  (Ctrl+0)", `FRAME_CORNERS` "Fit page", `MAGNIFYING_GLASS_PLUS` "Zoom in (Ctrl++)",
  `MAGNIFYING_GLASS_MINUS` "Zoom out (Ctrl+-)".
- Right panel: width 320 (egui resizable 240..520), fill `bg`, margin 14 × 6, separator on
  its left; 10 px, title 17 (+ comment count 15 dim for Comments) and `X` "Close panel",
  8 px, the view. Open on Pages by default (egui `show_sidebar` defaults to on).
- Views as egui `sidebar.rs` / `comments.rs` / `bookmarks.rs`: Comments (per page: "Page N"
  + count 13 dim, rows margin 8 radius 8 with a 28 px `surface` badge and glyph 15, kind 13
  dim, text; current row `surface`); Pages (rows 196, fill 2 px in radius 8, current accent
  at 18 %, page fitted in (row - 32) × 156 centered 84 px down, 2 px accent outline, number
  13, Move up / down `CARET_UP` / `CARET_DOWN` small buttons on hover 10 px from the right);
  Bookmarks (framed "Add" `BOOKMARK_SIMPLE` + framed `DOTS_THREE_VERTICAL` menu, 6 px, rows
  24 radius 4, title 14 at 6, number 12.5 dim, indent 18); Search (field + `MAGNIFYING_GLASS`
  button, status label, 4 px, hits as 26 px selectable rows). Menus use Phosphor glyphs
  (`panels::menu_entry`).
- Page box: drag right / up for later pages (1 per point), click to type (Enter or blur).
- Left to do: Form / Signature views, bookmark tree collapse arrows (GPUI shows the whole
  tree flat, indented), drag to resize the panel.

### Banner: `src/shell/banner.rs`

egui: `GodApp::banner`. Over the canvas only (between the side panels). Fill `surface`,
margin 16 × 6, separator under it. Glyph 16 (`LOCK` encrypted on save / `INFO` unsaved),
text 13, right: pill button (fill `bg`, 1 px `text_dim`, radius 14: "Remove password" /
"Save") and small `X` "Dismiss" (per document and kind).

### Welcome: `src/shell/welcome.rs`

egui: `welcome::show`, `GodApp::welcome_central`. Centered column (max 980): logo 112 × 112
radius 24, 10 px, "GodPDF" 40, tagline "Every PDF tool. Local. Pure Rust." 17 dim, 24 px,
three buttons 170 × 44 radius 10, text 16, 8 apart: "Open PDF" `FOLDER_OPEN` (fill `accent`,
text `on_accent`), "New PDF" `FILE_PLUS` and "Merge PDFs" `STACK` (1 px `accent` stroke).
Then, with recent files: 26 px, "RECENT" (small, strong, `accent_text`) and one frameless row
per file (`FILE_PDF` + name, tooltip the path).
Recent files: `Shell::recent` (newest first, 8), kept for the run only (no settings storage
yet); also Menu > File > Open recent. Closed tabs: `Shell::closed`, Ctrl+Shift+T reopens.

### Quick bar (canvas): `src/tools/quickbar.rs`

egui: `quickbar::show`. Card 16 px from the canvas' top-left: fill `panel`, 1 px `border`,
radius 10, margin 4, shadow (0, 4) blur 16 black alpha 28; tool toggles 28 × 28 (glyph 16)
2 px apart, `selected` fill when on; divider 7 high; undo `ARROW_U_UP_LEFT` / redo
`ARROW_U_UP_RIGHT`. Style card 6 px to its right. Tool glyphs: Select `CURSOR`, Hand `HAND`,
Text `TEXT_T`, Note `NOTE`, Highlight `HIGHLIGHTER`, Ink `PENCIL_SIMPLE`, Rectangle `SQUARE`,
Picture `IMAGE`, Sign `SIGNATURE`, Field `TEXTBOX`, Eraser `ERASER`.
GPUI already uses these glyphs (plus its own shapes); left to do: 28 px buttons (GPUI: 32),
card radius / margin / shadow.

### Not covered yet

Dialogs (`dialogs.rs`: window radius 12, margin 16, shadow above), toasts (`toasts.rs`:
radius 10, margin 14 × 10, max width 380, glyph 16 in the accent), command palette, popup
menus (radius 9, margin 8): they keep gpui-component's look in the egui palette and font.
