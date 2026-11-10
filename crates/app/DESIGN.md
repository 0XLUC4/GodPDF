# godpdf design

The design follows the previous app, see commit 026fd48. Dark and light themes;
reference picture: `docs/screenshot.png` (dark, document open, All tools). Sizes are
GPUI px at scale 1.

## Base (done)

| What | Source | GPUI |
|---|---|---|
| UI font | `Ubuntu-Light.ttf` (epaint_default_fonts 0.35), proportional | `fonts::UI` = `"Ubuntu"` + `fonts::UI_WEIGHT` (Light), set on the root div and in gpui-component's theme (`fonts::apply_to_components`) |
| Mono font | `Hack-Regular.ttf` | `fonts::MONO` = `"Hack"` |
| Icons | Phosphor regular 0.13 | `fonts::ICONS` = `"Phosphor"`; glyph constants in `icons.rs` under their Phosphor names (`icons::X`) |
| Default text size | Body / button 13, heading 18, small 9 | `fonts::BODY` = 13 on the root |
| Bold | none (strong text only recolours) | Ubuntu has only Light: bold resolves to Light too |
| Files | `~/.cargo/registry/.../epaint_default_fonts-0.35.0/fonts`, Phosphor 0.13 `res` | `assets/fonts/` (+ licences), `include_bytes!`, loaded by `fonts::load` in `main` |

GPUI on Windows only falls back to *system* fonts, so a Phosphor glyph inside a text string
does not show: draw icons as their own element, `widgets::icon(glyph, size, color)` (or
`widgets::glyph(glyph)` to inherit size and colour), and `widgets::with_icon(glyph, label,
size, color)` for a glyph and a label (glyph, gap of half the size, label).

### Palette (`theme.rs`)

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

Visuals: widget radius 7 (`theme::RADIUS`), window radius 12, menu
radius 9; window shadow offset (0, 10) blur 28 black alpha 110 (dark) / 28 (light); popup
shadow (0, 6) blur 16 alpha 3/4 of that. Hovered fg stroke 1.5. Spacing: item 8 × 6, button
padding 10 × 5, menu margin 8, window margin 16, interact height 26.

Tool colours (`commands.rs`, `theme::{GREEN, ...}`): Organize green #3FB950, Combine purple
#A371F7, Comment amber #E3A008, Protect / Edit / Help blue #4C8DF6, File / View teal #1FB5A5,
Document pink #E85AAD; delete / remove red #F05252; extract / export teal; Find, Properties
blue. Update dot blue #2563EB.

### Shared widgets (`widgets.rs`)

| Widget | GPUI | Size |
|---|---|---|
| `icon_button` | `widgets::icon_button(id, glyph, tip, palette)` | 28 × 28, glyph 16, no frame until hovered (hover fill, radius 7) |
| `small_icon_button` | `widgets::small_icon_button` | 22 × 22, glyph 13 |
| `rail_button` | `widgets::rail_button(id, glyph, tip, selected, enabled, palette)` | 32 × 32, glyph 18, `selected` fill when on |
| `tool_row` | `widgets::tool_row(id, glyph, tint, label, description, palette)` | full width, 40 high (52 with description), glyph 20 in `tint` centered 20 px in, label 14 at 42 px, description 13 dim; hover fill radius 8 |
| tooltip | `widgets::tooltip(element, text)` | gpui-component tooltip |
| frameless button | `widgets::button` | padding 10 × 5, height 26, text 13 |
| `icon_widget` disabled | `widgets::icon_widget(.., enabled = false, ..)` | glyph in `palette.disabled()`, no hover |

## Window layout

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
Panels draw a 1 px `border` separator on their inner edge: header bottom, tools panel
right, rail left, right panel left, banner bottom. The tab row has none.

## Regions

Each region is one file; parallel work stays inside it. TODO comments in each file list what
is left.

### Tab row: `src/shell/tabrow.rs`

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
  update is out, tooltip "GodPDF X.Y.Z is available"), `QUESTION` "About GodPDF", `MOON` "Dark
  mode" / `SUN` "Light mode" (28 × 28 icon buttons).
- Updates popover (`services::updates_button`, gpui-base `Popover`; Esc or a click outside
  closes it; the Updates command opens it): 6 under the button, right-aligned, 300 wide, fill
  `panel`, 1 px `border`, radius 12, popup shadow, padding 16. Update out: title 15 "GodPDF
  X.Y.Z is available", "You have X.Y.Z." dim, the notes' `- ` lines as bullets (max 260 high,
  scrolls), a full-width 34 high blue (`DOWNLOAD_BLUE`) "Update and restart" (spinner +
  "Downloading…" meanwhile, errors as toasts) and "Unsaved changes are asked about before the
  app restarts." 11 dim. Otherwise "You're up to date" / "Checking…" / "Could not check for
  updates" with "GodPDF X.Y.Z" (or the error) dim, and a refresh button on the right
  (Lucide `refresh-cw` SVG, spinning while checking: Phosphor glyphs are text and cannot rotate).

### Header: `src/shell/header.rs`

- Frame: fill `bg`, margin 10 × 0, height 46 (`HEADER_HEIGHT`), separator line under it.
- Menu: frameless button, `SIDEBAR_SIMPLE` + "Menu" at 14, min height 32. Popup (min width
  200): checkbox "Tools panel" (disabled without a document; turning it on leaves Read for
  All tools), separator, then the menus File / Edit / View / Tools / Help with `with_icon`
  entries and shortcut text.
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
- Menu entries: `header.rs::look` gives each command its icon and shortcut text (by label).
- Left to do: hide the shortcut under 200 px, View > Toggle sidebar.

### Left tools panel: `src/shell/leftpanel.rs`

Rows are `widgets::tool_row`.

- Shown when `show_tools` and mode is not Read. Width 280 (resizable 220..420), fill
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

### Organize grid and Side by side: `src/organize/`

- Grid (`grid.rs`) instead of the viewer while `DocTab::grid.on` (header Organize, the
  "Organize" row, "Organize pages"). Tool bar: panel fill, margin 16 × 10, framed buttons
  28 high (glyph + label, tooltip = description), a separator, Select all / Clear selection,
  "k of n selected" dim. Cards 168 × 218, 14 apart, 12 in from the left, 16 from the top,
  radius 10, `surface` (hover `hover`), 1 px `border` or 2 px `accent` when selected; page
  fitted in 144 × 180 at 12 from the top on white; number 13 at 14 above the bottom
  (`accent_text` when selected); earlier / later small arrows 8 in from the bottom corners
  (on hover or when selected). Dropping on a card's left / right half puts the dragged
  page (or its selection) before / after it, a 3 px accent line in the gap.
- Side by side (`split.rs`): the right half, a `border` line on its left; a row (10 in): the
  document picker (outline button, 80..240 wide), page box 38 × 26, "/ n" dim, the small
  `X` "Close the second view" 6 from the right; then a read-only viewer.
- Drag its right edge to resize (`resize.rs`, 220..420).

### Right rail: `src/shell/rail.rs` (+ the panel it opens: `panels::right` in `src/panels/mod.rs`)

- Rail: width 52, fill `bg`, margin 0 × 10, separator on its left. Top, centered, spacing 6:
  rail buttons Comments `CHAT_TEXT`, Pages `FILES`, Bookmarks `BOOKMARK_SIMPLE`, Search
  `MAGNIFYING_GLASS`, Form `LIST_CHECKS`, Signature `SIGNATURE`; tooltip = title; click
  toggles the right panel on that view (Search focuses its field).
- Bottom, top to bottom, spacing 4: page box (DragValue 38 × 26, tooltip "Page number (drag
  or click to type)"), "/ N" 12 dim, `CARET_UP` "Previous page", `CARET_DOWN` "Next page"
  (disabled at the ends), 8 px, zoom "150%" 12 dim, `ARROWS_OUT_LINE_HORIZONTAL` "Fit width
  (Ctrl+0)", `FRAME_CORNERS` "Fit page", `MAGNIFYING_GLASS_PLUS` "Zoom in (Ctrl++)",
  `MAGNIFYING_GLASS_MINUS` "Zoom out (Ctrl+-)".
- Right panel: width 320 (resizable 240..520), fill `bg`, margin 14 × 6, separator on
  its left; 10 px, title 17 (+ comment count 15 dim for Comments) and `X` "Close panel",
  8 px, the view. Open on Pages by default.
- Views: Comments (per page: "Page N"
  + count 13 dim, rows margin 8 radius 8 with a 28 px `surface` badge and glyph 15, kind 13
  dim, text; current row `surface`); Pages (rows 196, fill 2 px in radius 8, current accent
  at 18 %, page fitted in (row - 32) × 156 centered 84 px down, 2 px accent outline, number
  13, Move up / down `CARET_UP` / `CARET_DOWN` small buttons on hover 10 px from the right);
  Bookmarks (framed "Add" `BOOKMARK_SIMPLE` + framed `DOTS_THREE_VERTICAL` menu, 6 px, rows
  24 radius 4, title 14 at 6, number 12.5 dim, indent 18); Search (field + `MAGNIFYING_GLASS`
  button, status label, 4 px, hits as 26 px selectable rows). Menus use Phosphor glyphs
  (`panels::menu_entry`).
- Page box: drag right / up for later pages (1 per point), click to type (Enter or blur).
- Bookmark branches have an open / close caret in their indent slot (open at first when they
  hold the bookmark of the page read); the panel is resized by its left edge (`resize.rs`).

### Banner: `src/shell/banner.rs`

Over the canvas only (between the side panels). Fill `surface`,
margin 16 × 6, separator under it. Glyph 16 (`LOCK` encrypted on save / `INFO` unsaved),
text 13, right: pill button (fill `bg`, 1 px `text_dim`, radius 14: "Remove password" /
"Save") and small `X` "Dismiss" (per document and kind).

### Welcome: `src/shell/welcome.rs`

From the top left, padding 24, scrolls: "RECENT" (small, `accent_text`), 12 under it a
wrapping row of recent files, 20 apart, no card: the first page preview 208 × 294 on `canvas`,
radius 4, 1 px `border` (hover `accent`), shadow (0, 2) blur 8 alpha 20 (rendered in the
background, cached per path + modified time; `LOCK` for encrypted files, `FILE_PDF` for
unreadable ones, empty while rendering); 8 under it name 13 and folder 11 dim, both with
ellipsis; tooltip the path, pointer cursor. Click opens; `X` small button on hover (preview's
top-right, 6 in) or right-click "Remove from recent" removes it. A missing file: 50 % opacity,
`FILE_X`, "File not found", only removable. No recent file: logo 56 radius 12 and "Open a PDF
(Ctrl+O) to see it here" 15 dim, centered. No buttons: Open, New and Merge are in Menu > File,
the palette and the tab row's "+ Open".
Recent files: `Shell::recent` (newest first, 8), kept in `services::settings`; also Menu >
File > Open recent. Closed tabs: `Shell::closed`, Ctrl+Shift+T reopens.

### Quick bar (canvas): `src/tools/quickbar.rs`

Card 16 px from the canvas' top-left: fill `panel`, 1 px `border`,
radius 10, margin 4, shadow (0, 4) blur 16 black alpha 28; tool toggles 28 × 28 (glyph 16)
2 px apart, `selected` fill when on; divider 7 high; undo `ARROW_U_UP_LEFT` / redo
`ARROW_U_UP_RIGHT`. Style card 6 px to its right; its colours are dots 18 (1 px `border`)
in a 26 ring, 2 apart: 2 px `text` stroke with a 2 px gap on the current one, `border` on
hover. Circles get a radius half a pixel under half their side (`circle_radius`): layout
snaps sides to device pixels, and a radius over half the snapped side makes lumpy edges.
Rectangle / ellipse cards add a Fill column ("Stroke" / "Fill" headings, 9 px `text_dim`)
opening with No fill: a white dot with a red slash. Tool glyphs: Select `CURSOR`, Hand `HAND`,
Text `TEXT_T`, Note `NOTE`, Highlight `HIGHLIGHTER`, Ink `PENCIL_SIMPLE`, Rectangle `SQUARE`,
Picture `IMAGE`, Sign `SIGNATURE`, Field `TEXTBOX`, Eraser `ERASER`.
GPUI already uses these glyphs (plus its own shapes); left to do: 28 px buttons (GPUI: 32),
card radius / margin / shadow.

### Not covered yet

Dialogs (`dialogs.rs`: window radius 12, margin 16, shadow above), toasts (`toasts.rs`:
radius 10, margin 14 × 10, max width 380, glyph 16 in the accent), command palette, popup
menus (radius 9, margin 8): they keep gpui-component's look in the app palette and font.
