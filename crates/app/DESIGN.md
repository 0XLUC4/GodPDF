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
| border | #242424 | #E6E6E6 | hairline separators, field / card / window strokes (never on buttons) |
| text | #F2F2F2 | #111111 | `override_text_color` |
| text_dim | #9A9A9A | #666666 | inactive tabs, hints, noninteractive fg; disabled = text_dim at 50 % |
| accent / accent_text | #F2F2F2 | #111111 | active widget stroke, primary buttons, links, selection stroke |
| on_accent | #111111 | #FFFFFF | text on accent |
| accent_hover | #D6D6D6 | #333333 | hovered / pressed primary button |
| button / button_hover | #222222 / #2C2C2C | #F1F1F1 / #E8E8E8 | secondary button fill, hovered |
| selected | #333333 | #E0E0E0 | `selection.bg_fill`, active / pressed widget fill, selected rail button |
| canvas | #0A0A0A | #F2F2F2 | viewer behind pages |
| error | #F87171 | #D92D20 | error text and the error bar |

Visuals: widget radius 6 (`theme::RADIUS`), window (dialog) radius 12, menu / popup radius 8;
window shadow offset (0, 10) blur 28 black alpha 110 (dark) / 28 (light); popup shadow (0, 6)
blur 16 alpha 3/4 of that. Spacing scale 4 / 8 / 12 / 16 / 24; button padding 12 × 6,
button / field height 28 (`theme::INTERACT_HEIGHT`), larger actions 32.

Polish rules: whole-pixel sizes, paddings and gaps (`with_icon` rounds its gap); icons 16
(14 in 22 px buttons, 18 / 20 in rail and tool rows); circles get exactly half their side
as radius (`circle_radius` in quickbar; gpui clamps it to the painted size, a smaller one leaves
flat, jagged sides); separators are 1 px
`border` hairlines; secondary text `text_dim`; scrollbar thumbs 6 wide, `text_dim` at 35 %.

Tool colours (`commands.rs`, `theme::{GREEN, ...}`): Organize green #3FB950, Combine purple
#A371F7, Comment amber #E3A008, Protect / Edit / Help blue #4C8DF6, File / View teal #1FB5A5,
Document pink #E85AAD; delete / remove red #F05252; extract / export teal; Find, Properties
blue. Update dot blue #2563EB.

### Shared widgets (`widgets.rs`)

| Widget | GPUI | Size |
|---|---|---|
| `icon_button` | `widgets::icon_button(id, glyph, tip, palette)` | ghost 28 × 28, glyph 16: transparent, `hover` fill on hover, `selected` pressed, radius 6 |
| `small_icon_button` | `widgets::small_icon_button` | 22 × 22, glyph 14 |
| `rail_button` | `widgets::rail_button(id, glyph, tip, selected, enabled, palette)` | 32 × 32, glyph 18, `selected` fill when on |
| `tool_row` | `widgets::tool_row(id, glyph, tint, label, description, palette)` | full width, 40 high (52 with description), glyph 20 in `tint` centered 20 px in, label 14 at 42 px, description 13 dim; hover fill radius 8 |
| tooltip | `widgets::tooltip(element, text)` | gpui-component tooltip |
| ghost text button | `widgets::button` | transparent, height 28, padding 12, radius 6, text 13; `hover` on hover, `selected` pressed |
| secondary button | `widgets::framed_button(id, label, enabled, palette)` | `button` fill, **no border**, `button_hover` on hover, `selected` pressed; disabled = dim label, no hover |
| primary button | `widgets::accent_button(id, label, enabled, palette)` | solid `accent`, `on_accent` text, no border, `accent_hover` on hover / pressed; disabled = 50 % |

Button rules: one primary per dialog / card, first in the row; others secondary; toolbar and
chrome icons are ghost. No 1 px outline on any button (gpui-component buttons: `ghost()` or
`secondary()`, never `outline()`); only fields, cards and previews have a border. The custom
div buttons are not focusable; gpui-component buttons and fields show their focus ring
(`ring` = `accent_text`).
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
  closes it; the Updates command opens it): 6 under the button, right-aligned, 256 wide, fill
  `panel`, 1 px `border`, radius 10, popup shadow, padding 12, text 12, title 14. Update out: title 15 "GodPDF
  X.Y.Z is available", "You have X.Y.Z." dim, the notes' `- ` lines as bullets (max 200 high,
  scrolls), a full-width 32 high blue (`DOWNLOAD_BLUE`) "Update and restart" (spinner +
  "Downloading…" meanwhile, errors in the error bar) and "Unsaved changes are asked about before the
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
- Right, right to left with spacing 6: `PRINTER` "Print… (Ctrl+P)", `FLOPPY_DISK_BACK` "Save As…
  (Ctrl+Shift+S)", `FLOPPY_DISK` "Save (Ctrl+S)", `INFO` "Document properties" (disabled
  without a document), 6 px, the search pill, then a spinner "Working…" while a job runs.
  Under 900 px wide the pill is a `MAGNIFYING_GLASS` icon button; the mode tabs are clipped
  before the right buttons.
- Search pill: width `available - 40` clamped 140..300, height 32, radius 16, fill `bg`, 1 px
  `border` (`text_dim` when hovered), `MAGNIFYING_GLASS` 16 dim 10 px in, "Find
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
  "Organize" row, "Organize pages"). Tool bar: panel fill, margin 16 × 10, secondary buttons
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
  Bookmarks (secondary "Add" `BOOKMARK_SIMPLE` + secondary `DOTS_THREE_VERTICAL` menu, 6 px, rows
  24 radius 4, title 14 at 6, number 12.5 dim, indent 18); Search (field + `MAGNIFYING_GLASS`
  button, status label, 4 px, hits as 26 px selectable rows). Menus use Phosphor glyphs
  (`panels::menu_entry`).
- Page box: drag right / up for later pages (1 per point), click to type (Enter or blur).
- Bookmark branches have an open / close caret in their indent slot (open at first when they
  hold the bookmark of the page read); the panel is resized by its left edge (`resize.rs`).

### Banner: `src/shell/banner.rs`

Over the canvas only (between the side panels). Fill `bg`, at least 36 high, padding 16 left
/ 8 right, hairline under it. Glyph 16 dim (`LOCK` encrypted on save),
text 13, right: a secondary button ("Remove password") and small `X` "Dismiss" (per
document). No unsaved banner: it would push the page down on the first edit; the tab's `*`
and Save say it.

### Notifications: `src/toasts.rs`

No toast stack. `toasts::success` / `info` are no-ops: the result shows on the page, and
slow work shows the header's "Working…" spinner (`jobs::background`). `toasts::error` must
never be lost: one error bar (latest wins) in the banner slot, above the banner (above the
welcome screen without a document): full width, at least 36 high, `error` at 10 % over the
fill, hairline `error` at 25 % under it, `WARNING_CIRCLE` 16 in `error`, text 13 `text`,
small `X` "Dismiss"; it goes after 6 s. Warnings that matter (file changed by another
program while edited here) go through `error`.

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

Bar 12 px from the viewer's top-left (cards never pass the viewer's height less 12 above and
below: they scroll): fill `panel`, 1 px `border`, radius 12, padding 6, shadow (0, 4) blur 16
black alpha 28; tool toggles 32 × 32 (glyph 18, radius 6) 4 px apart, hover fill `hover`, the
active one tinted blue (`theme::BLUE` glyph on blue at 16 %); groups Select Hand | Text Note
Highlight Ink Rectangle Picture Stamp | Redact Eraser | undo `ARROW_U_UP_LEFT` / redo
`ARROW_U_UP_RIGHT`, split by 1 px dividers 20 wide. Tooltips carry the shortcut.
Style card 8 px to its right, top-aligned, same card; columns one button wide, 4 apart, each
under a 12 px `text_dim` glyph 14 high whose tooltip names it (Type `SHAPES`, Colour `DROP` /
Stroke `PEN`, Fill `PAINT_BUCKET`, Size `TEXT_AA` / Width `RULER`). Cards carry no titles nor
hints: icon controls with tooltips, numbers (sizes, counts, mm fields) stay visible. Eraser:
`STACK` comments / `ERASER` page content toggles, then the brush stepper (+, size, -) for the
page. Stamp: the labels (rows 32 high, 4 columns wide), `CALENDAR_CHECK` date toggle. Redact
(only once areas are marked): count, `CHECK` apply (red), `X` clear. Picture: one glyph per
field (`ARROWS_HORIZONTAL` W, `ARROWS_VERTICAL` H, `ARROW_LINE_RIGHT` X, `ARROW_LINE_DOWN` Y; mm
in the tooltip), `LINK_SIMPLE` keep proportions, `ARROWS_OUT_SIMPLE` original size. Note,
Picture, Sign and Field show no card. Colours
are dots 18 in a 26 ring, 2 apart: 2 px `text_dim` stroke with a 2 px gap on the current one,
`border` on hover. Only light colours (and No fill) get a 1 px `border` outline. Circles get
exactly half their side as radius (`circle_radius`). The Fill column opens with No fill: a white dot with a red slash.
Each colour column ends with the custom swatch: `PLUS` 12 on a `surface` dot, or the current
colour when it is none of the presets (ringed as current). It opens the picker card
(`tools/picker.rs`) 8 px right of the style card (in its place when the viewer is too narrow
for both): padding 10, gap 10, title + `X`; saturation / value square 192 × 128 and hue slider
192 × 12 (six gradient segments of 32), radius 6, knobs 14 with a 2 px white ring; preview
chip 40 × 26 and a hex field (`#RRGGBB`, `RRGGBB`, `#RGB`); "Recent" (9 dim) and the last 6
custom colours (settings `recent_colors`). A colour is applied on release, Enter or a click
(one undo step). Tool glyphs: Select `CURSOR`,
Hand `HAND`, Text `TEXT_T`, Note `NOTE`, Highlight `HIGHLIGHTER`, Ink `PENCIL_SIMPLE`,
Rectangle `SQUARE`, Picture `IMAGE`, Sign `SIGNATURE`, Field `TEXTBOX`, Eraser `ERASER`.

### Not covered yet

Dialogs (`dialogs/mod.rs::card`: radius 12, padding 16, popup shadow, 8 between items,
title 18, button row 8 more above), command palette, popup
menus (radius 8, margin 8): they keep gpui-component's look in the app palette and font.
