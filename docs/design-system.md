# swaypplet design system

**Every surface swaypplet draws takes its colour, type, space, shape and
motion from one set of tokens, and its widgets from one set of components.**
The tokens are generated from four inputs (mode, accent, neutral, contrast);
nothing else in the stylesheet or the code picks a colour or a size. This
file is the source of truth: `data/tokens.css`, `src/tokens.rs`, `src/ui/`
and the lint tests implement it, and a change to any of them starts here.

The research behind it (Material 3, Apple HIG and Liquid Glass, Fluent 2,
libadwaita, Radix, Carbon, Primer, Linear) and the audit of the stylesheet
before it are summarised in the zoo, `docs/design-system-zoo.html`.

## 1. Principles

1. **The glass is the surface; nothing on it is glass.** One glass layer per
   surface. Tiles, rows, fields and menus inside a card are fills on it.
2. **Accent means "on", "selected" or "the one primary action".** Nothing
   else is accent-coloured. At most one primary action per card.
3. **Size and weight before colour.** Hierarchy is carried by the type
   scale first; colour is kept free for state and status.
4. **Three text levels.** `--fg` for names and values, `--fg-muted` for
   state, `--fg-faint` for metadata. Nothing in between.
5. **Status colour is only for status.** Green means healthy, not
   decoration.
6. **Group with fills and space, not lines.** Dividers only inside lists.
7. **States are overlays of the content's own colour**, so they read the
   same over any backdrop and in either mode.
8. **A mode is a token set and a material tuning, never a new shape.**

## 2. Inputs

| Input | Values | Default | Where it is set |
|---|---|---|---|
| mode | `dark`, `light`, `auto` | `auto` | settings: Look → Appearance |
| accent | a named pair (see §3.2) | `aqua` | settings: Look → Accent |
| neutral | `gruvbox`, `slate`, `pure` | `gruvbox` | settings: Look → Neutral |
| contrast | `standard`, `high` | `standard` | settings: Look → Contrast; `prefers-contrast: more` forces `high` |

The existing wallpaper **tint** (`Look.tint`: off, accents, full) stays: it
rotates the hue of the generated accent (and, at `full`, the neutral) the
way `src/palette.rs` does today, after generation and before emission.

Nix ships the defaults in `theme/settings.nix`; the settings pane changes
them per user. `src/tokens.rs` turns them into `tokens.css` on every change.

### 2.1 Auto mode follows the sun

`auto` is dark from dusk to dawn and light in between, at the location the
night light uses (`gammastep.nix`: 55.6 N, 13.0 E; Nix writes it to
`/etc/swaypplet/theme.json` so both read one value).

- **Light** once the sun is 3° above the horizon, **dark** once it is 3°
  below, with the state held in between. The 6° band stops a switch from
  flickering at dawn and keeps light mode out of the dimmest civil twilight.
- The elevation comes from the NOAA solar position formulas, in
  `src/tokens/sun.rs`, with no network and no new daemon: the long-lived
  panel process checks once a minute and at wake from suspend.
- **A switch never happens in front of you.** When the sun crosses, the
  switch waits for the first of: idle (the same idle hint the lock uses),
  lock, unlock, or 10 minutes with no swaypplet surface open. It then applies
  as one step: tokens reloaded, material values sent.
- Choosing `dark` or `light` in the pane sets that mode until you choose
  `auto` again.

## 3. Tokens

Three tiers. **Rules in `data/style.css` may use only the semantic tier.**
Primitives are private to `tokens.css`. Component tokens exist only where a
component needs a value no semantic token gives (listed in §3.8).

All tokens are CSS custom properties on `:root` (GTK ≥ 4.16). Rust never
reads them back: values the code needs (`src/tokens.rs`) come from the same
generator that writes the file.

### 3.1 Primitives: two 12-step scales per mode

`--neutral-1` … `--neutral-12` and `--accent-1` … `--accent-12`, generated in
OKLCH. **Each step keeps its job in both modes**, so a rule written against a
step is right in light and dark alike; only the lightness curve flips.

| Steps | Job |
|---|---|
| 1–2 | ground: the darkest (dark) or lightest (light) surfaces |
| 3–5 | component fills at rest, hover, pressed |
| 6–8 | lines: subtle, interactive, focus |
| 9–10 | solid: the accent itself, and its hover |
| 11–12 | text: secondary, primary |

**Neutral** is generated from three anchors per mode, so the preset keeps its
exact identity: step 1, step 3 and step 12. The steps between interpolate
lightness along a fixed curve and chroma and hue linearly between the
anchors.

| Neutral | dark 1 / 3 / 12 | light 1 / 3 / 12 |
|---|---|---|
| gruvbox | `#1d2021` / `#32302f` / `#ebdbb2` | `#fbf1c7` / `#ebdbb2` / `#282828` |
| slate | `#15181c` / `#262b31` / `#e3e8ee` | `#f7f9fb` / `#e6ebf0` / `#1f252b` |
| pure | `#141414` / `#262626` / `#ebebeb` | `#fafafa` / `#e8e8e8` / `#1f1f1f` |

Lightness curve, as the fraction of the way from step 3 to step 12, for
steps 4–11: `0.05 0.10 0.17 0.26 0.40 0.52 0.66 0.83`. Steps 1–2 sit at
0 and 0.5 of the way from step 1 to step 3.

**Accent** is a pair per name, step 9 in each mode being the pair's colour:

| Accent | dark | light |
|---|---|---|
| aqua | `#689d6a` | `#427b58` |
| yellow | `#d79921` | `#b57614` |
| blue | `#458588` | `#076678` |
| purple | `#b16286` | `#8f3f71` |
| orange | `#d65d0e` | `#af3a03` |
| red | `#cc241d` | `#9d0006` |

Other accent steps take the pair's hue, chroma capped per step (`0.02 +
0.012 × step` below 9, `0.10` for 11, half that for 12), and the lightness of
the same step on the neutral scale, except 10 (step 9 ± 0.05 L, toward the
text end) and 11–12 (dark: 0.84 / 0.95; light: 0.45 / 0.30).

**On-accent** is white or the mode's ink (`--neutral-1` dark, `--neutral-12`
light), whichever reaches the higher APCA Lc on step 9. If neither reaches
60, `--accent-bg` steps down to step 8, where white does: gruvbox's dark
yellow carries Lc 54 either way. Decided per accent, as Radix does per hue.

**Status** is not generated; it is a fixed set per mode, from gruvbox:

| | dark text / fill | light text / fill |
|---|---|---|
| success | `#8ec07c` / `#689d6a` | `#427b58` / `#427b58` |
| warning | `#fabd2f` / `#b57614` | `#9a5b04` / `#b57614` |
| danger | `#fb6a5a` / `#cc241d` | `#9d0006` / `#cc241d` |

Text on a status fill is white in both modes.

**Categorical** colours mark identity and nothing else: which task a
workspace belongs to (t1–t4) and which app a notification came from
(a1–a6). Six slots per mode, each a readable tone for text, dots and rails,
and a fill: dark mode uses gruvbox's bright tones to read and its neutral
tones to fill; light mode its deep tones for both. The readable tone is
then lifted to an OKLCH lightness of at least 0.80 (dark) or deepened to at
most 0.50 (light), hue and chroma kept, so it clears Lc 45 on the glass.

| Slot | dark text / fill | light text / fill |
|---|---|---|
| 1 aqua | `#8ec07c` / `#689d6a` | `#427b58` / `#427b58` |
| 2 yellow | `#fabd2f` / `#b57614` | `#a06a0e` / `#b57614` |
| 3 blue | `#83a598` / `#458588` | `#076678` / `#076678` |
| 4 purple | `#d3869b` / `#b16286` | `#8f3f71` / `#8f3f71` |
| 5 green | `#b8bb26` / `#79740e` | `#6a6608` / `#79740e` |
| 6 orange | `#fe8019` / `#af3a03` | `#af3a03` / `#af3a03` |

### 3.2 Semantic tier

These are the only colour names a rule may use.

**Surfaces**

| Token | Value | Use |
|---|---|---|
| `--surface-key` | `#32302f` at 0.50 | what a glass card paints, and only so the compositor finds its shape (§4). Never shown. |
| `--surface-raised` | `--neutral-4` | a surface with no glass behind it (the `.no-glass` fallback, the greeter's bare card) |
| `--scrim` | black at 0.20 | the dimming layer behind modal surfaces; the lock screen's arithmetic depends on it |

**Text**

| Token | Value (standard / high) | Use |
|---|---|---|
| `--fg` | `--neutral-12` | names, values, anything read first |
| `--fg-muted` | `--neutral-12` at 84 % / 88 % (light: 82 / 88) | state, secondary text |
| `--fg-faint` | `--neutral-12` at 67 % / 72 % (light: 62 / 72) | metadata, captions, placeholders |
| `--fg-disabled` | `--neutral-12` at 40 % / 45 % | text of controls that cannot be used |

The muted and faint levels are the lowest that pass §5 through the glass;
the steps between text levels are small on purpose, because the type scale
carries the hierarchy (principle 3).

**Fills on glass.** Mixes of `currentColor` into transparent, so they follow
the text colour of whatever they sit in.

| Token | Value | Use |
|---|---|---|
| `--fill-1` | 6 % | sections, fields, menus inside a card |
| `--fill-2` | 10 % | controls at rest, selected rows |
| `--fill-3` | 16 % | tracks, strong fills, the avatar ring |

**Lines**

| Token | Value (standard / high) | Use |
|---|---|---|
| `--border-subtle` | `--neutral-12` at 8 % / 16 % (light 10 / 16) | card edges, list dividers |
| `--border-strong` | `--neutral-12` at 16 % / 30 % (light 20 / 30) | focused fields, outlined controls |
| `--focus-ring` | `--accent-11` at 70 % | keyboard focus, 2 px, offset 2 px |

**Accent**

| Token | Value | Use |
|---|---|---|
| `--accent-bg` | `--accent-9`, or `--accent-8` (§3.1) | on, selected, the primary action, slider fill |
| `--accent-bg-hover` | `--accent-10` | its hover, where an overlay would muddy it |
| `--on-accent` | see §3.1 | text and icons on `--accent-bg` |
| `--accent` | `--accent-11` | accent text: 15 px bold and larger only |
| `--accent-tint` | `--accent-9` at 16 % | a quiet accent ground: the key badge, a pinned mark |

**Status**: `--success`, `--warning`, `--danger` (text and dots),
`--success-bg`, `--warning-bg`, `--danger-bg` (fills), `--on-status`, and
`--success-tint`, `--warning-tint`, `--danger-tint` (the fill at 16 %, for a
quiet status ground such as an urgent notification).

**Categorical**: `--cat-1` … `--cat-6` (text, dots, rails), `--cat-n-bg`
(fills), `--cat-n-tint` (the fill at 30 %).

### 3.3 States

One model for every interactive widget. The overlay is a
`background-image` layered over whatever `background-color` the widget has,
so the same rule works on a fill, on the accent and on nothing:

```css
--state-hover:    color-mix(in srgb, currentColor 7%, transparent);
--state-pressed:  color-mix(in srgb, currentColor 16%, transparent);
--state-selected: color-mix(in srgb, currentColor 10%, transparent);
```

| State | Treatment |
|---|---|
| rest | the widget's own fill |
| hover | `background-image: linear-gradient(var(--state-hover), var(--state-hover))` |
| pressed (`:active`, `:checked` while pressed) | the same with `--state-pressed` |
| selected (`:selected`, `.selected`) | `--fill-2`, or `--accent-bg` where selection is the point (a chip, a toggle) |
| focus (`:focus-visible`) | `outline: 2px solid var(--focus-ring); outline-offset: 2px` |
| disabled | text `--fg-disabled`, no overlay, accent fills fall back to `--fill-2` |

Transitions on state: `background-color`, `background-image`, `color` and
`outline-color` over `--dur-fast` with `--ease-standard`. GTK cannot
animate custom properties, so transitions name the real properties.

### 3.4 Type

One unit (px), one family per role, seven sizes, three weights.

| Token | Size | Weight | Use |
|---|---|---|---|
| `--type-display` | 36 | 700 | the clock on the lock screen, the OSD value |
| `--type-display-sm` | 28 | 700 | a greeting, a big number on a card |
| `--type-title` | 18 | 600 | a card's title (polkit, a notification's summary) |
| `--type-title-sm` | 15 | 600 | a section's name, a tile's name |
| `--type-body` | 13 | 400 | rows, fields, running text |
| `--type-label` | 12 | 400 / 600 | state lines, buttons in dense places, chips |
| `--type-caption` | 11 | 400 | metadata, timestamps, footnotes |

Weights: `--w-regular` 400, `--w-strong` 600, `--w-heavy` 700.
Families: `--font` (Ubuntu Sans Nerd Font, then Ubuntu Sans, Noto Sans,
sans-serif) and `--mono` (JetBrains Mono Nerd Font, monospace) for keys,
code and tabular numbers. Numbers that change in place (clock, percentages,
sizes) use `font-feature-settings: "tnum"`.

The bar is the one place under 11 px: its glyph labels use `--type-label`
and its tiny superscripts are glyphs, not a size.

### 3.5 Space

A 4 px grid with one half step:

| Token | px | Use |
|---|---|---|
| `--space-1` | 2 | hairline gaps: badge padding, icon to its dot |
| `--space-2` | 4 | inside a chip, between a label and its value |
| `--space-3` | 8 | between siblings in a row; a row's vertical padding |
| `--space-4` | 12 | a row's horizontal padding; between rows in a group |
| `--space-5` | 16 | a card's padding; between groups |
| `--space-6` | 24 | between sections of a large card |
| `--space-7` | 32 | around a surface's content on large surfaces (lock, greeter) |

Box `spacing` in Rust takes the same values through `tokens::space(n)`.

### 3.6 Shape

| Token | px | Use |
|---|---|---|
| `--radius-control` | 6 | buttons, fields, chips' square cousins, menu items |
| `--radius-tile` | 10 | tiles, rows, sections, menus |
| `--radius-thin` | 14 | thin glass: the bar, the OSD, the face cue |
| `--radius-card` | 18 | every floating card: panel, launcher, notifications, polkit, keybinds, switcher, lock |
| `--radius-pill` | 999 | pills, chips, switches, toggle tiles in the bar |

The glass geometry in `glass.nix` (`crest_radius`) reads these two values:
thin 14, card 18. The cross-repo guard checks both.

### 3.7 Elevation

Glass needs no shadow: the material's rim and refraction are its elevation.
Two tokens for the rest:

| Token | Value | Use |
|---|---|---|
| `--shadow-float` | dark: none; light: `0 10px 30px rgb(20 16 10 / 0.16)` | a floating glass card in light mode, where the rim alone is weak |
| `--shadow-solid` | `0 8px 32px rgb(0 0 0 / 0.45)` | a surface with no glass (`.no-glass`) |

### 3.8 Motion

The ladder the stylesheet already keeps (`docs/MOTION.md`), as tokens.
`src/anim.rs` reads the same numbers from `tokens.rs`.

| Token | Value | Use |
|---|---|---|
| `--dur-fast` | 150 ms | colour, opacity, state |
| `--dur-standard` | 200 ms | small movement: chevrons, revealers, switches |
| `--dur-emphasis` | 300 ms | entrances |
| `--dur-spatial` | 400 ms | surfaces moving: panels, the switcher strip |
| `--dur-long` | 500 ms | the lock and greeter crossfades |
| `--ease-standard` | `cubic-bezier(0.2, 0, 0, 1)` | everything by default |
| `--ease-decelerate` | `cubic-bezier(0, 0, 0, 1)` | entering |
| `--ease-accelerate` | `cubic-bezier(0.3, 0, 1, 1)` | leaving |

Reduced motion (`Look.motion`) scales the durations as today.

### 3.9 Component tokens

Only these, because no semantic token gives their value:

| Token | Value | Why |
|---|---|---|
| `--control-height` | 30 px | buttons, fields, segmented controls |
| `--row-height` | 40 px | list rows, so every list has one rhythm |
| `--tile-height` | 52 px | quick-settings tiles |
| `--track-height` | 6 px | slider and progress tracks |
| `--knob` | dark `--neutral-12`, light white | the slider and switch knob |

## 4. Glass and the modes

The compositor draws the glass (`liquid_glass_*`, nixos
`patches/scenefx-liquid-glass.patch`). Per pixel it refracts and frosts the
backdrop, absorbs along the path through the slab, applies the photochromic
term, mixes in the body fill, then draws the client's content and the rim.

**The card paints `--surface-key` (`#32302f` at 0.50) in both modes**, only so
the compositor can see its shape; the compositor drops those pixels
(`fillKey` in `glass.nix`). What the glass shows is the material's own body
fill. So a mode switch never touches the mask, the key, the mask threshold
or the lock screen's scrim arithmetic. It changes the tokens and these
material values, per glass namespace, over IPC:

| Value | dark | light | Why |
|---|---|---|---|
| `fill_color` | `--neutral-3` | `--neutral-2` | smoked body against milky body; follows the neutral input |
| `fill_alpha` | 0.50 (high 0.72) | 0.65 (high 0.80) | milky glass over a black terminal needs more body to keep dark text readable |
| `absorb` | 1.0 | 0.25 (high 0.20) | milky glass must not grey out the light it lets through |
| `photochromic` | 0.35 (high 0.25), a ceiling | −0.40 (high −0.50), a lift | dark caps a white page behind; light floors a black terminal behind |
| `edge_light` | 0.09 (high 0.12) | 0.14 (high 0.18), drawn darker | a light rim disappears on a light body |
| `frost` | 0.33 (high 0.45) | 0.45 (high 0.55) | light glass shows detail behind it more clearly |

Everything else (refraction, dispersion, bevel, crest, specular, grain) is
one material in both modes. A negative `photochromic` is the one shader
change the light mode needs: a soft floor on luminance,
`lum' = lum + f·exp(−lum/f)`, the mirror of the ceiling.

`glass.nix` keeps its one material and gains `modes.dark` and `modes.light`
overrides for these six values; the settings pane's glass tab edits the
current mode's.

## 5. Contrast

Every text token must reach these APCA Lc values over the glass body,
computed through the material (absorption, photochromic, fill). Over a
mid-grey or black backdrop (a desktop, a terminal) the full value; over a
pure white page 15 less at standard contrast and 5 less at high contrast,
because the dark material as tuned cannot fully overcome white (`--fg`
reaches Lc 62 there, 71 at high contrast):

| Token | Lc |
|---|---|
| `--fg` | 75 |
| `--fg-muted` | 60 |
| `--fg-faint` | 45 |
| `--accent` (15 px bold) | 60 |
| `--on-accent` on `--accent-bg` | 60 |
| `--on-status` on each status fill | 60 |

`src/tokens.rs` tests all of them for all 144 input combinations (2 modes,
6 accents, 3 neutrals, 2 contrasts). A token set that fails does not ship.

## 6. Components

One builder per component in `src/ui/`, one CSS class family each. A
surface is assembled from these; a surface-specific class is allowed only
for layout (where things go), never for colour, type or shape.

| Component | Builder | Class | Replaces |
|---|---|---|---|
| Card | `ui::card(kind)` (`Card::Thin`, `Card::Floating`) | `.card`, `.card.thin` | `.glass-card` added by hand in 13 places; `GlassSurface` |
| Section | `ui::section(icon, title, summary)` → header + revealer | `.section`, `.section-header` | 8 inline copies in `widgets/` |
| Row | `ui::row(icon, title, subtitle)` + `.end(widget)` | `.row`, `.row.activatable`, `.row.selected` | device, network, user, backup, launcher result, dmenu, keybind, session rows |
| Toggle tile | `ui::tile(icon, title, state)` + optional drill-in | `.tile`, `.tile.on` | `widgets/tiles.rs` (kept, restyled) |
| Slider row | `ui::slider_row(icon, range)` | `.slider-row` | audio, brightness, display, glass pane |
| Switch row | `ui::switch_row(title, subtitle)` | `.row` + `switch` | settings panes, network |
| Button | `ui::button(label, Kind)` (`Primary`, `Secondary`, `Flat`, `Destructive`), `ui::icon_button` | `.btn.primary` etc. | per-surface button classes |
| Chip | `ui::chip(label)` | `.chip`, `.chip.selected` | launcher filters, lock user chips |
| Badge | `ui::badge(text)` | `.badge` | counts |
| Key | `ui::key(text)` | `.key` | `.jump-chord`, keybind sheet keys |
| Status | `ui::status(kind, label)` | `.status.ok/.warn/.bad` | ad hoc coloured labels |
| Field | `ui::field(label, entry, help)` | `.field`, `.field.error` | entries in network, polkit, lock |
| Menu | `ui::menu(items)` | `.menu`, `.menu-item` | notification card menu, bar popovers |
| Avatar | `ui::avatar(user, size)` | `.avatar` | lock and users rows |
| Progress | `ui::progress(fraction)` | `.progress` | OSD, media, power, notifications |

## 7. Enforcement

Tests in `src/tokens.rs` and `src/theme.rs`, run by `cargo test`:

1. **No literal colours in the rules.** `data/style.css` contains no hex,
   `rgb()`, `rgba()`, `hsl()`, named colour (`white`, `black`), `alpha()`,
   `shade()`, `mix()` or `@name` outside comments. Colours come from `var()`.
2. **Only semantic tokens.** Every `var(--x)` in the rules names a token from
   §3.2–3.9. Primitives (`--neutral-n`, `--accent-n`) are refused.
3. **Only the scales.** Every `font-size` is a `--type-*` token; every
   `border-radius` a `--radius-*` token; every padding, margin and
   `border-spacing` a `--space-*` token or 0; every duration a `--dur-*`
   token.
4. **Contrast** (§5) for every input combination.
5. **Rust**: no `set_spacing`, `margin_*` or `spacing(...)` with a literal
   other than those in `tokens::SPACE`; no Cairo colour literals outside
   `src/tokens.rs`. Checked by a source scan test.

## 8. Migration

Each step lands on its own, with before and after screenshots of every
surface from `dev/render.sh`.

1. `tokens.css` generated and loaded; the rules move from `@name`,
   `alpha()` and `shade()` to the semantic tokens.
2. Type, radius, space and motion onto the scales.
3. The components in `src/ui/`, and every surface rebuilt on them, one at a
   time: bar, panel and its sections, launcher and dmenu, OSD,
   notifications, polkit, keybinds, lock and greeter, settings, switcher and
   pins, screenshot and annotate.
4. The enforcement tests switched on as each rule becomes true.
5. Light mode: the light token set, the material overrides, the shader's
   lift, and the sun.

## 9. Migrating a surface

A surface is migrated when its file in `data/css/` and its Rust code
satisfy §7, and a render from `dev/render.sh` has been compared with the
one before. The steps:

1. **Build with the components.** Replace hand-built widgets with the
   builders in `src/ui/`: `ui::card` for the glass card, `ui::row` and
   `ui::row_button` for rows, `ui::section` for collapsible groups,
   `ui::button` for buttons, `ui::slider_row`, `ui::switch`, `ui::chip`,
   `ui::badge`, `ui::key`, `ui::status`, `ui::field`, `ui::menu`,
   `ui::progress`. Put text on the scale with `ui::text`,
   `ui::set_text_style` or `ui::glyph`. Box spacing is `ui::vbox(n)` /
   `ui::hbox(n)` with a step of the space scale.
2. **Shrink the surface's CSS to layout.** What stays in its file places
   things: padding, margins, min sizes, alignment, the apron around a
   card. Colour, font size and weight, radius and state belong to the
   components. Where a surface needs a look no component gives, add it to
   `00-components.css` and `src/ui/` as a component, not to the surface.
3. **Map what is left** with this table:

| Legacy | Token |
|---|---|
| `@fg` | `--fg` |
| `@fg_dim` | `--fg-muted` |
| `@text_faint` | `--fg-faint` |
| `@surface` (the card fill) | `--surface-key` |
| `@surface_raised`, `@bg0`, `alpha(@bg0, …)` as a solid | `--surface-raised` |
| `@surface_hover`, `@bg1`, `alpha(@fg, 0.04–0.08)`, `alpha(white, 0.03–0.06)` | `--fill-1` or `--fill-2` |
| `@surface_track`, `@bg2`, `alpha(@bg3, …)`, `alpha(@fg, 0.10–0.16)` | `--fill-3` |
| `@border_subtle` | `--border-subtle` |
| `@border_strong`, `@bg3` as a line | `--border-strong` |
| `@accent`, `@aqua` as a fill | `--accent-bg` |
| `@accent`, `@aqua` as text or an icon | `--accent` (15 px strong and up), else `--fg` |
| `@accent_tint`, `@accent_tint_hi`, `alpha(@accent, < 0.5)` | `--accent-tint` |
| `alpha(@accent, ≥ 0.5)` as a ring | `--focus-ring` |
| `@danger`, `@red`, `@accent_quinary` | `--danger` (text) / `--danger-bg` (fill) |
| `@danger_tint`, `alpha(@red, …)` | `--danger-tint` |
| `@accent_secondary`, `@yellow` as a warning | `--warning` / `--warning-bg` / `--warning-tint` |
| `@green`, `@accent_bright_5` as health | `--success` / `--success-bg` |
| task colours t1–t4 | `--cat-1` … `--cat-4` (and `-bg`, `-tint`) |
| notification app colours a1–a6 | `--cat-1` … `--cat-6` |
| `shade(@x, k)` for a hover | the state overlay (§3.3), or `--accent-bg-hover` |
| `alpha(black, …)` in a text-shadow on the desktop | `--scrim` |
| `alpha(black, …)` as a shadow under glass | remove: the compositor frosts it |
| a font size | the nearest `--type-*`; a glyph keeps the size, not the weight |
| a radius | `--radius-control` 6, `--radius-tile` 10, `--radius-thin` 14, `--radius-card` 18, `--radius-pill` |
| a padding, margin or spacing | the nearest `--space-*` |
| a duration | the nearest `--dur-*` |
