# swaypplet design system

**Every surface swaypplet draws takes its colour, type, space, shape and
motion from one set of tokens, and its widgets from one set of components.**
The tokens are generated from five inputs (mode, accent, neutral, contrast,
and the wallpaper tint); nothing else in the stylesheet or the code picks a
colour or a size. This
file is the source of truth: the generator (`src/tokens/`), the runtime that
resolves its inputs and loads the stylesheet (`src/theme/`), the components
(`src/ui/`) and the lint tests implement it, and a change to any of them starts here.

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
| tint | `off`, `accents`, `full`, with the wallpaper's hue | `off` | settings: Look → Theme colour (§2.2) |

Nix ships the defaults in `theme/settings.nix`; the settings pane changes
them per user. `src/theme/inputs.rs` resolves them (the sun for `auto`, the
wallpaper's hue for the tint) into one `tokens::Inputs`, and `tokens::css`
(`src/tokens/emit.rs`) turns that into `tokens.css` on every change.

### 2.1 Auto mode follows the sun

`auto` is dark from dusk to dawn and light in between, at the same place
the night light uses: the one picked on the map in the Day and night group.
Sunrise and sunset are instants in UTC that follow from that place; the
time zone only formats them as clock times.

- **Light** once the sun is 3° above the horizon, **dark** once it is 3°
  below, with the state held in between. The 6° band stops a switch from
  flickering at dawn and keeps light mode out of the dimmest civil twilight.
- The elevation comes from the NOAA solar position formulas, in
  `src/theme/sun.rs`, with no network and no new daemon: the long-lived
  panel process checks once a minute and at wake from suspend.
- **A switch never happens in front of you.** When the sun crosses, the
  switch waits for the first of: idle (the same idle hint the lock uses),
  lock, unlock, or 10 minutes with no swaypplet surface open. It then fades
  over `--motion-page` (§2.3).
- Choosing `dark` or `light` in the pane sets that mode until you choose
  `auto` again.
- **Day and night can be moved.** The Look pane's Day and night group
  (`daylight`) takes the place on a map, moves sunrise and sunset
  earlier or later, or replaces the sun with fixed clock times. The night
  light follows the same day and night.
- **One process decides, every other one draws its answer.** The panel
  resolves the inputs (the mode, the Look settings, the wallpaper's tint and
  text backdrop), writes them to `$XDG_RUNTIME_DIR/swaypplet/theme-<display>.json`,
  and only then sends the glass from the same answer. The lock screen, the
  polkit agent and every other process read that file and reload when it
  changes (`theme::follow`), so their text changes with the glass. On the
  login screen the greeter is the one that decides.

### 2.2 The wallpaper tint

The tint is an input like the others: the generator builds the scales from
it, so the stylesheet, the glass body, Cairo drawing and sway's window
borders all move together, and §5 tests the tinted sets. What it needs from
outside the settings is a **palette** of up to three OKLCH hues of the
wallpaper, in whole degrees, each with one job. The wallpaper offers up to
four colours: Material's `Score` over the quantized image (area and
chroma), in its order, each at least 30° from every one before it and 2 %
of the image. The Look pane shows them as swatches under the tint
(**Accent from**, the setting `look.tint_colour`, a rank), the first
selected; picking a new wallpaper selects its first again.

| Hue | Picked as | Threshold |
|---|---|---|
| primary | the offered colour picked in the Look pane; by default the first, `Score`'s top | none offered, and the tint is off |
| ground | the chromatic hue with the most of the image within ±15° of it | at least 20 % of the image, else the first offered colour |
| secondary | the best-ranked other offered colour far enough from the primary | at least 45° from it, else none |

Before 2026-09 the sample kept only the primary and one secondary (45° and
5 % of the image), and nothing on screen showed the secondary: it only
turned the categorical set by up to 15°. A muted wallpaper (whose primary
was itself 1 % of the image by that count) then read as one colour, whatever
it had in it.

The image gives hue only. Taking its lightness is what breaks the contrast
of palette generators that do (pywal); Material and KDE take hue and keep
tone per role, and so does this. The panel samples the palette
(`src/theme/wallpaper.rs`) into `$XDG_CACHE_HOME/swaypplet/wallpaper-source`
once per wallpaper, off the main thread, and every process reads that one
line. A wallpaper with no usable colour, or one not sampled yet, leaves the
tint off; one with a single colour gives a palette of three equal hues.
`src/tokens/tint.rs` holds the rules.

Every rule keeps each colour's **lightness**, because lightness is what the
contrast is made of; only hue moves, and chroma gives way (never lightness)
where a hue does not fit in sRGB.

| Family | `accents` and `full` | Why |
|---|---|---|
| accent | every step takes the primary, keeping its lightness and chroma | the accent input still sets how loud it is; the wallpaper sets which colour |
| categorical | all six turn by one angle: slot 1 onto the primary, then by at most 15° more to bring the nearest other slot onto the secondary | rigid, so they stay as far apart as gruvbox put them. Slots are never assigned straight from the image: most wallpapers are analogous, and the six would collapse into a band |
| status | each turns toward the nearest palette hue by at most 12° | red stays red: 12° keeps it out of orange |
| neutral | `full` only: the three anchors take the ground at a chroma held within 0.010–0.025 | a cast, not a colour. The ground and not the primary, so a blue sky with a red boat gives blue-grey glass and a red accent; the glass body (`fill_color`) follows |

There is no second accent token; the other offered colours reach the
screen as the accent when picked, and through the categorical and status
rules. One is added when a component needs it.

`off` is the untinted token set, byte for byte.

### 2.3 A change of inputs fades

A mode switch, a new tint, a new wallpaper or a Look edit fades from the old
colours to the new over `--motion-page` (500 ms, standard curve, scaled by
the Motion setting; instant with Motion off or GTK's reduced motion). The
first stylesheet a process loads never fades, and nothing fades while no
window of the process is on screen.

- **The stylesheet** (`src/theme/fade.rs`). The main provider takes the new
  document at once, so every rule and every token that is not a colour is
  final from the first frame. A second provider above it holds only the
  colour tokens that changed (54 of them for a mode switch), starts at the
  old values and steps them through OKLCH on the frame clock; when it ends it
  is emptied, which leaves exactly what a reload without a fade shows. A full
  document is 0.1 ms to generate and 8–19 ms for GTK to parse, too much for
  every frame; the overlay parses in 0.1–0.6 ms. What each step does cost is
  GTK restyling every widget (a custom property on `:root` invalidates them
  all): below a millisecond with the bar alone, enough with the panel's
  launcher list open that a step slower than 25 ms is followed by a frame
  with no step, which keeps every other frame free for input.
- **The glass** (`src/settings/glass_fade.rs`). The material walks from what
  the compositor has to the new mode's over the same duration and curve, a
  push every 50 ms, on a thread of its own against the wall clock: it costs
  the main thread nothing and does not stutter when a GTK frame is slow.
  Every push goes through that thread in order, so the last value sent is
  the last value set. Two values do not move along their number: the fill's
  `none`/−1 sentinels resolve to the key the card paints, and
  `photochromic` moves by what it does (a ceiling fades out by its strength
  `1/p`, then a lift grows from zero), because a small positive number is
  the strongest ceiling and a straight line through it darkened the glass
  half way to light. A test walks the switch through the glass model and
  holds the body to getting lighter at every step.
- **The lock screen** fades its own stylesheet the same way while it is up.
  Its one-second check and the panel's are not synchronised, so its text can
  start up to a second before or after the glass.

Cairo drawing (`theme::observe`) is redrawn once, at the start, in the new
colours: it does not fade.

## 3. Tokens

Three tiers. **Rules in `data/css/` may use only the semantic tier.**
Primitives are private to `tokens.css`. Component tokens exist only where a
component needs a value no semantic token gives (listed in §3.8).

All tokens are CSS custom properties on `:root` (GTK ≥ 4.16). Rust never
reads them back: values the code needs (`crate::tokens`) come from the same
generator that writes the file.

`src/tokens/` is pure: inputs in, values out, no I/O and no GTK. One file
per concern: `color.rs` (sRGB, OKLCH), `inputs.rs`, `tint.rs`, `scales.rs`
(§3.1), `semantic.rs` (status, categorical, text levels), `material.rs`
(§4), `apca.rs` (§5), `fixed.rs` (space, type, radius, durations, the fill
key, component sizes), `motion.rs` (§3.8) and `emit.rs` (`tokens.css`).
`src/theme/` is the runtime around it: `inputs.rs` (the one place an
`Inputs` is built), `sun.rs` (§2.1), `wallpaper.rs` (§2.2), `locked.rs`,
`paint.rs` (the token colours for Cairo), `sway.rs` (window borders) and
`mod.rs` (the stylesheet, its reload, and `watch`).

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
| gruvbox | `#1d2021` / `#32302f` / `#ebdbb2` | `#f8f7f4` / `#ebe9e4` / `#282624` |
| slate | `#15181c` / `#262b31` / `#e3e8ee` | `#f7f9fb` / `#e6ebf0` / `#1f252b` |
| pure | `#141414` / `#262626` / `#ebebeb` | `#fafafa` / `#e8e8e8` / `#1f1f1f` |

Gruvbox's light anchors are not its own cream (`#fbf1c7`, `#ebdbb2`): as
glass that read as yellowed paper. They keep its warm hue at a fraction of
the chroma, so light mode is white glass with a trace of warmth.

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

**Status** is not generated; it is a fixed set per mode, from gruvbox
(harmonised toward the wallpaper under a tint, §2.2):

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
Under a tint the six turn together (§2.2).

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
| `--scrim` | black at 0.20 | the ground of a well and of an editor's canvas |

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
| `--fill-disabled` | `--neutral-12` at the hairline's share (8 % / 16 %; light 10 / 16) | the box of a control that cannot be used. Not a currentColor mix: the label greys to `--fg-disabled` and the box must not grey with it |

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
| disabled | the label `--fg-disabled`, the box `--fill-disabled`, no overlay; a flat button keeps no box. An action's accent falls back to `--fill-disabled`. A control that is *on* (a checked toggle, chip or tile) keeps its accent at half strength (`opacity: 0.5`), and a switch, check or slider, whose value is its shape, fades whole the same way: a value that cannot be changed is still a value |

Transitions on state: `background-color`, `background-image`, `color` and
`outline-color` over `--dur-fast` with `--ease-standard`. GTK cannot
animate custom properties, so transitions name the real properties.

### 3.4 Type

One unit (px), one family per role, seven sizes and a hero, three weights
and a light one for the hero.

| Token | Size | Weight | Use |
|---|---|---|---|
| `--type-hero` | 136 | 300 | the lock screen's clock, and nothing else |
| `--type-display` | 36 | 700 | the OSD icon, a big number on a card |
| `--type-display-sm` | 28 | 700 | a greeting, a big number on a card |
| `--type-title` | 18 | 600 | a card's title (polkit, a notification's summary) |
| `--type-title-sm` | 15 | 600 | a section's name, a tile's name |
| `--type-body` | 13 | 400 | rows, fields, running text |
| `--type-label` | 12 | 400 / 600 | state lines, buttons in dense places, chips |
| `--type-caption` | 11 | 400 | metadata, timestamps, footnotes |

Weights: `--w-regular` 400, `--w-strong` 600, `--w-heavy` 700, and
`--w-light` 300 for the hero size only: at 136 px a heavy face shouts.
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

The `cornerRadius` of each glass surface in `glass-config.nix` reads these
two values: thin 14, card 18. The cross-repo guard checks both.

### 3.7 Elevation

Glass needs no shadow: the material's rim and refraction are its elevation.
Two tokens for the rest:

| Token | Value | Use |
|---|---|---|
| `--shadow-float` | dark: none; light: `0 10px 30px rgb(20 16 10 / 0.16)` | a floating glass card in light mode, where the rim alone is weak |
| `--shadow-solid` | `0 8px 32px rgb(0 0 0 / 0.45)` | a surface with no glass (`.no-glass`) |

### 3.8 Motion

Motion is named by what it means on screen; the duration and curve follow
from the meaning, so two things that mean the same move the same way on
every surface. `src/tokens/motion.rs` decides them, the stylesheet gets
them as `--motion-*` (duration and curve in one token, for `transition`
and `animation`), and `src/anim.rs` reads the same values.

| Token | Duration | Curve | Means |
|---|---|---|---|
| `--motion-state` | 150 ms | standard | a control changing state: hover, press, colour, a check |
| `--motion-expand` | 200 ms | standard | something opening or closing in place: a section, a chevron, a switch knob, a revealer |
| `--motion-enter` | 300 ms | decelerate | something arriving: a card, a notification, a row that was not there |
| `--motion-exit` | 200 ms | accelerate | something leaving; shorter than its entrance, because waiting for a thing to go is dead time |
| `--motion-move` | 300 ms | standard | something on screen moving to where it now belongs: a reflow, a reorder, a resize |
| `--motion-travel` | 400 ms | standard | a whole surface crossing a distance: the switcher strip, a panel sliding in |
| `--motion-page` | 500 ms | standard | the whole screen changing: switching user, the greeter handing over, the mode or the tint (§2.3) |

The curves: standard `cubic-bezier(0.2, 0, 0, 1)`, decelerate
`cubic-bezier(0, 0, 0, 1)`, accelerate `cubic-bezier(0.3, 0, 1, 1)`. The
raw `--dur-*` and `--ease-*` stay available for a transition that lists
several properties with one meaning.

The Motion setting scales every duration when the tokens are generated
(full, half, or one frame), so a rule never has to know about it. Rust
reads the same scale through `anim::ms`. A GTK revealer or stack keeps its
`transition_duration` as a number, so `ui::revealer` and `ui::page_stack`
remember each one they build (weakly, with its motion) and set it again
whenever the stylesheet reloads or reduced motion flips: a change reaches
the transitions already on screen, not only the ones built after it.

**Rules.** An enter pairs with an exit, never with another enter. The lock
is an enter and an unlock an exit, not a page: the lock has to be up before
a lid-close suspend, and waiting to get back to work is dead time. Colour
and opacity changes of a control are `state`, even when they accompany a
move. A surface's entrance is `enter` for its content and `travel` for the
surface itself when it crosses the screen. Attention loops (a pulse, a
shake, breathing) are the only animations that name their own period, and
only inside `@keyframes` that repeat.

### 3.9 Component tokens

Only these, because no semantic token gives their value:

| Token | Value | Why |
|---|---|---|
| `--control-height` | 30 px | buttons, segmented controls |
| `--control-height-small` | 24 px | small buttons in dense places (a popover's actions) |
| `--field-height` | 34 px | text fields: a caret needs more room than a label |
| `--chip-height` | 26 px | chips |
| `--menu-item-height` | 32 px | menu items |
| `--row-height` | 40 px | list rows, so every list has one rhythm |
| `--row-height-dense` | 32 px | a row in a column of many (a settings row, the settings sidebar, a picker's hits, the prefix list): the same step for every one, so a group is a column of equal steps |
| `--tile-height` | 52 px | quick-settings tiles, and every control on the Helm's deck beside them, so the deck is one grid |
| `--track-height` | 6 px | slider and progress tracks |
| `--knob` | dark `--neutral-12`, light white | the slider and switch knob |
| `--on-accent-muted` | `--on-accent` at 80 % | the second line on an accent fill, a tile's state under its name |
| `--fg-on-wallpaper` | `--on-status` (white) or a dark ink (`#1d2021`), from the wallpaper | text standing on bare wallpaper (the lock's switch-user button, the greeter's users, the switcher's caption); see below |
| `--halo-on-wallpaper` | black or white, 0.20–0.40 per layer | the halo under that text, the opposite of the ink |

**Text on the wallpaper.** Text with no card behind it stands on whatever
the wallpaper is, and the mode does not change the wallpaper, so its tone
follows the image and not the mode. The panel's wallpaper sampler measures
the image's relative luminance on a 4 × 4 grid, once per wallpaper and off
the main thread, and writes it into the same one-line cache as the tint's
hues. Every process reads it; the lock screen never decodes the image. The
middle 2 × 2 cells, where all such text sits, are pooled into a mean and a
deviation, and `src/tokens/backdrop.rs` picks the ink and the halo:

- light ink on a black halo, or dark ink on a white one: whichever reaches
  Lc 75 at the glyph edge with the lighter halo, light first at a tie (the
  shipped look);
- the halo is four tight layers at 0.20, 0.30, 0.40 or 0.50 alpha each (a
  core of 0.59 to 0.94), denser over a busy image;
- the ground the contrast is measured on is the region's mean minus and
  plus one deviation, blended with
  the half of the halo's core that reaches a glyph edge (the shadows are
  blurred, so the edge sits on the halo's shoulder; 0.5 is calibrated on
  renders, where the full core called white on a white page readable).

Every region from black to white is tested: up to 10 % deviation the text
reaches Lc 75; over a bright region busier than that, no halo gets there
and it is held to Lc 68 (it reaches 69–75). The smaller text's Lc 60
holds everywhere. With no sample yet the answer is the shipped look: white
ink on a black halo at 0.20.

**The lock's clock stands on a shade.** The clock (`--type-hero`, 136 px,
light weight) and the date stand on the wallpaper with no card. A halo of
tight stacked shadows made an outline around every glyph, and a glass
plate read as a second card, so the lock draws a shade instead: a soft black
oval behind the clock, `SHADE_PEAK` 0.22 at the centre and fading along a
smoothstep to nothing, with light ink and one wide text shadow
(`--shadow-on-shade`, 0.05) for depth (`ui::on_shade`). Every lock-surface
pixel that is not a card must stay under the discard line (0.28), and the
shade and the text shadow stacked at full strength, as they are inside a
digit's counter, reach 0.259 (a test holds it). Measured on renders over the
alcohol-ink wallpaper, the clock's light ink reaches Lc 52 over the median
ground behind it and Lc 37 over the brightest tenth; APCA asks about Lc 45
of 136 px text at weight 300, so the brightest busy regions fall short, and
no treatment under the discard line closes that gap (a glass plate does). Nothing else tints the wallpaper: no scrim, no backdrop, and every
lock card paints the plain key. The switch-user button and the greeter's
users keep the halo.

The lock and the greeter follow the mode: the plate's and the card's glass
follow it like every other namespace. The greeter reads the system's Look
from `/etc/swaypplet/settings.json`, which the machine's owner sets in the
nixos flake, owns the theme on its compositor as the panel does in a
session, and sends that compositor the matching material (`settings::glass::apply_greeter`).

## 4. Glass and the modes

The compositor draws the glass (`liquid_glass_*`, nixos
`patches/scenefx-liquid-glass.patch`). Per pixel it refracts and frosts the
backdrop, absorbs along the path through the slab, applies the photochromic
term, mixes in the body fill, then draws the client's content and the rim.

**The card paints `--surface-key` (`#32302f` at 0.50) in both modes**, only so
the compositor can see its shape; the compositor drops those pixels
(`fillKey` in `glass.nix`). What the glass shows is the material's own body
fill. So a mode switch never touches the mask, the key, the mask threshold
It changes the tokens and these
material values, per glass namespace, over IPC, fading both (§2.3):

| Value | dark | light | Why |
|---|---|---|---|
| `fill_color` | `--neutral-3` | `--neutral-2` | smoked body against milky body; follows the neutral input |
| `fill_alpha` | 0.50 (high 0.72) | 0.50 (high 0.72) | the same body in both modes, so light glass lets as much through as dark; the lift below keeps dark text readable over a black terminal |
| `absorb` | 1.0 | 0.25 (high 0.20) | milky glass must not grey out the light it lets through |
| `photochromic` | 0.35 (high 0.25), a ceiling | −0.43 (high −0.50), a lift | dark caps a white page behind; light floors a black terminal behind |
| `edge_light` | 0.11 (high 0.12) | 0.17 (high 0.18), drawn darker | a light rim disappears on a light body |
| `frost` | 0.33 (high 0.45) | 0.30 (high 0.45) | light glass keeps more of the picture behind it: its lift already evens the backdrop out, so it needs less frost than dark glass |

Everything else (refraction, dispersion, bevel, specular, shine) is one
material in both modes. A negative `photochromic` is the one shader
change the light mode needs: a soft floor on luminance,
`lum' = lum + f·exp(−lum/f)`, the mirror of the ceiling.

The mode always sets these six, dark at standard contrast included
(`glass::for_mode`), so nothing a person tunes is one mode's material
only. The settings pane's Glass tab moves them only relative to the mode,
with the same meaning in both:

- **Clarity** (−1 to +1, 0 the mode's own) scales the body fill by up to
  ±50 % (`tokens::material_at`). Where a thinner fill would cost the text
  its §5 contrast over the mode's hard backdrop (black behind light glass,
  white behind dark), the lift or the ceiling strengthens (to at most
  −0.70 and 0.15), and past that the fill steps back toward the mode's own.
  At +1 both standard modes reach a fill of 0.25.
- **Frost** multiplies the mode's frost.

The rest of the tab is the one material: the profile, refraction,
dispersion, the highlight and the bevel. Presets are written in the same
terms: a smoked preset is a clarity of −0.8, never an absorb.

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

`src/tokens/apca.rs` tests all of them for all 72 untinted input
combinations (2 modes, 6 accents, 3 neutrals, 2 contrasts), and each of
those again under `accents` and `full` at every 5° of wallpaper hue: 10,440
token sets. A token set that fails does not ship.

## 6. Components

One module per component in `src/ui/`, one stylesheet per component in
`data/css/components/` (the same split), and one class family each. A
surface is assembled from these; a surface-specific class is allowed only
for layout (where things go), never for colour, type or shape.

The API has three shapes (the module docs of `src/ui/mod.rs`):

1. **A noun constructs and returns**: `ui::button(label, kind)`,
   `ui::row(..)`, `ui::badge(text, tone)`.
2. **`ui::<component>::adopt(&w, ..)` styles a widget the caller had to
   build**: one GTK builds (a search entry, a dropdown over a model, a scale
   on a shared adjustment) or whose shape the surface owns (the box that
   becomes a card).
3. **`ui::set_<state>(&w, TypedEnum | bool)` changes state at runtime.** A
   state is a typed value, never a class name: no `"ui-…"` string appears in
   Rust outside `src/ui/` (§7, `rust-ui-class`).

| Component (module, stylesheet) | Build | Adopt | State | Classes |
|---|---|---|---|---|
| Surface (`surface`) | | `ui::surface::adopt(&root)`, `ui::window::adopt(&root)` (solid), `ui::canvas::adopt(&w)` | | `.ui-surface`, `.ui-window`, `.ui-canvas` |
| Text (`text`) | `ui::text(s, Text, Tone)`, `ui::heading(s)`, `ui::overline(s, Tone)` | `ui::glyph::adopt(&l, Text, Tone)`, `ui::overline::adopt(&l)`, `ui::on_wallpaper::adopt(&w)`, `ui::on_shade::adopt(&w)`, `ui::live_caption::adopt(&l)` | `set_text_style`, `set_tone`, `set_weight(Weight)`, `set_numeric`, `set_mono` | `.ui-hero` … `.ui-caption`, `.ui-muted` … `.ui-danger`, `.ui-strong`, `.ui-glyph`, `.ui-mono`, `.ui-numeric`, `.ui-overline`, `.ui-on-wallpaper`, `.ui-on-shade`, `.ui-live-caption` |
| Layout (`layout`) | `ui::vbox(n)`, `ui::hbox(n)`, `ui::stack(o, n)`, `ui::separator(Orientation)`, `ui::pill_group(n)`, `ui::toolbar(n)`; `ui::pad(&w, n)` | | | `.ui-separator(.vertical)`, `.ui-pill-group`, `.ui-toolbar` |
| Card (`card`) | `ui::group(n)`, `ui::well()` | `ui::card::adopt(&w, Card)` (`Floating`, `Thin`, `Solid`) | `set_card_tint(CardTint, on)`, `set_success` | `.ui-card` (`.thin`, `.solid`, `.success`, `.danger`, `.recessed`), `.ui-group`, `.ui-well` |
| Button (`button`) | `ui::button(label, Kind)`, `ui::button_with(Face, Kind, Size)`, `ui::toggle_button(Face, Kind, Size)` | `ui::button::adopt(&b, Kind, Size)` | `set_button_kind`, `set_armed` | `.ui-btn` (`.primary`, `.flat`, `.destructive`, `.small`, `.icon`, `.pill`, `.armed`) |
| Chip, badge, key, status (`chip`) | `ui::chip(Face)`, `ui::toggle_chip(label)`, `ui::badge(text, BadgeTone)`, `ui::key(text)`, `ui::status(Status, label)` | | `set_status(Status)`, `set_handoff(Option<Handoff>)` | `.ui-chip` (`.rich`, `.picked`, `.dropped`), `.ui-badge(.neutral)`, `.ui-key`, `.ui-status` (`.success`, `.warning`, `.danger`, `.neutral`) |
| Row (`row`) | `ui::row(icon, title, subtitle)` → `Row`, `ui::row_button(..)`, `ui::list()`, `ui::list_row(&content)`; `Row::set_icon_image` | | `set_selected`, `set_instant`, `set_busy` | `.ui-row` (`.activatable`, `.selected`, `.instant`, `.busy`) and its parts, `.ui-list` |
| Expander (`expander`) | `ui::section(icon, title, summary)` → `Section`, `ui::disclosure(label)` → `Expander`; `set_open`, `Section::show_as_page` | | | `.ui-section` (`.open`, `.page`) and its parts, `.ui-disclosure(.open)`, `.ui-chevron` |
| Toggle tile (`tile`) | `ui::tile_toggle(icon, title)` | | `set_loading` | `.ui-tile(.on)`, `.ui-tile-toggle(.loading)`, `.ui-tile-title` |
| Split tile (`tile`) | `ui::tile_split(icon, title)` → `SplitTile { root, toggle, detail, status }` | | `set_tile_status`, `set_loading` | `.ui-tile-split(.on)`, `.ui-tile-body`, `.ui-tile-detail`, `.ui-tile-status`: the body toggles, the chevron opens the detail, never the other way round |
| Slider, switch, check (`slider`) | `ui::slider_row(icon, min, max, step)` → `SliderRow` (`icon_button(tooltip)`), `ui::switch()`, `ui::switch_row(..)`, `ui::check(label)` | `ui::slider::adopt(&scale, Density)` | `set_over_range` | `.ui-slider` (`.dense`, `.over`), `.ui-slider-row` and its parts, `.ui-switch`, `.ui-check` |
| Field (`field`) | `ui::field(label, &input)`, `ui::text_area(lines)` → `(ScrolledWindow, TextView)`, `ui::dropdown(choices)` | `ui::entry::adopt(&e, FieldSize)`, `ui::dropdown::adopt(&d)` | `set_field_state(FieldState, on)` | `.ui-field` (`.armed`, `.busy`, `.reject`), `.ui-field-label`, `.ui-entry(.large)`, `.ui-text-area`, `.ui-dropdown` |
| Menu (`menu`) | `ui::menu()`, `ui::menu_item(label, accel, danger)` | | | `.ui-menu`, `.ui-menu-item(.danger)`, `.ui-menu-accel` |
| Popover (`popover`) | `ui::popover(&child, position)` (its child is a `Card::Solid`) | | | `.ui-popover` |
| Progress (`progress`) | `ui::progress(fraction)` | `ui::progress::adopt(&p)` | `set_progress_status(Option<Status>)` | `.ui-progress` (`.success`, `.warning`, `.danger`) |
| Bar (`bar`) | `ui::segmented()`, `ui::mark(&child, quiet)`, `ui::bay(&child, task)`, `ui::bay_chip()`, `ui::rail(slot)` | `ui::segment::adopt(&w, quiet)`, `ui::mark::adopt(&b, quiet)`, `ui::meter::adopt(&area)` | `set_category`, `set_receded`, `set_selection(Selection)`, `set_danger`, `set_quiet`, `set_ribbon(Ribbon)`, `set_bay_state(BayState, local)` | `.ui-cat-1…6`, `.ui-receded`, `.ui-meter`, `.ui-segmented`, `.ui-segment`, `.ui-mark`, `.ui-bay`, `.ui-bay-chip`, `.ui-rail` |
| Media (`media`) | `ui::thumb()`, `ui::pick_thumb(&child)`, `ui::swatch(&child)`, `ui::ring()` | `ui::thumb::adopt(&w)`, `ui::choice_grid::adopt(&g)`, `ui::placeholder::adopt(&w)`, `ui::lifted::adopt(&w)` | `set_pinned` | `.ui-thumb`, `.ui-pick-thumb`, `.ui-choice-grid`, `.ui-swatch`, `.ui-ring(.pinned)`, `.ui-placeholder`, `.ui-lifted` |
| Face indicator (`face`) | `ui::face_ring(size)`, `ui::face_pill(size)` → `FacePill` | | `set_face_state(Option<FaceState>)`, `set_face_enter` | `.ui-face-ring`, `.ui-face-pill`, `.ui-face-eye`, `.ui-face-mouth`, `.ui-face-enter`; states `.looking`, `.dark`, `.found`, `.ok`, `.fail` |
| Avatar (`avatar`) | `ui::avatar(name, icon, size, logged_in)` | | | `.ui-avatar` (`.cat-n`, `.active`), `.ui-avatar-presence` |
| Motion (`motion`) | `ui::revealer(transition, motion)`, `ui::page_stack(transition, motion)` | | `set_breathing`, `ui::shake(&w)` (one-shot) | `.ui-breathing`, `.ui-shake` |

`ui::icons` holds the icon-font glyphs; `theme::paint()` the token colours
for code that draws with Cairo (it reads the inputs on screen, so it lives
with the theme), and `ui::set_source` puts one on a Cairo context. `data/css/components/gtk.css` styles what
GTK builds itself (windows, a popover menu's buttons and separators,
scrollbars) and comes last in the cascade.

### 6.1 Surfaces

The components go on a surface, and a surface is `shell::Surface`
(`src/shell/`): a layer-shell window, its root (`ui::surface`, on the
window's child and never the window), the glass card on the root
(`ui::card`) and the `anim::Reveal` that fades them in and out.

```rust
let surface = Surface::builder(app, Namespace::Osd)
    .monitor(Some(&monitor))       // None: the compositor picks
    .anchor(&[Edge::Bottom])       // .fill() for all four
    .margin(Edge::Bottom, 72)
    .card(ui::Card::Thin)          // or .no_card() for a stage
    .build();                      // .slide(axis, px) for a settle
surface.card().append(&content);
surface.set_content(&content);
surface.show();
```

- **Use it for every layer surface.** A card on it: `.card(kind)`, and
  `.slide(axis, px)` when the card should travel as it fades. A full-screen
  stage that draws its own ground (the Super+Tab row, the region selector)
  or a surface with no card to fade (the face cue): `.no_card()`, which
  maps and unmaps outright and has no Reveal. Anything placed around the
  card goes on `surface.root()`: a spacer (`shell::fit` sizes the launcher's
  and the panel's), an alignment.
- **Dropping it is its teardown.** The last handle releases the Reveal (the
  compositor alpha handle, which must go before its `wl_surface` or the
  process dies of a protocol error, and the namespace's glass count), then
  destroys the window, realizing one that was never shown so GTK does not
  dereference a NULL surface. A surface is dismissed with `hide()` and
  dropped from `connect_hidden`.
- **One per output** is `shell::PerMonitor<T>`: it builds an entry for each
  monitor and drops the entry of one that leaves (the bar, the OSD, the
  keybind sheet, the greeter's cards).
- **Click outside the card** is `connect_backdrop_click`, a hit test. A
  claiming gesture on the card is the wrong tool: it can starve the
  controls inside it.
- The lock and the greeter are session-lock surfaces with their own fade
  (`lock/fade.rs`); they share `Namespace` and `ui::surface` on the root,
  not `Surface`. Annotate is a normal toplevel on `ui::window`.

The namespace is `shell::Namespace`, never a string (`layer-namespace`,
§7): it is also the key the compositor's glass and the settings pane
address the surface by. `Namespace::glass()` is the geometry class the
session's sway config gives it, which must agree with the nixos repo's
`sessionSurfaces` table (`users/modules/theme/glass-config.nix`, delivered as
`/etc/swaypplet/glass.json`); `settings::glass::System::drift` compares
the two on load.

| Namespace | String | Glass | Surface |
|---|---|---|---|
| `Panel` | `swaypplet` | panel | the control centre (`app::panel_surface`) |
| `Bar` | `swaypplet-bar` | thin | the bar, one per output |
| `Launcher` | `swaypplet-launcher` | panel | the launcher, and the dmenu picker |
| `Osd` | `swaypplet-osd` | thin | the OSD, one per output |
| `Notification` | `swaypplet-notification` | panel | one per popup card |
| `Pin` | `swaypplet-pin` | panel | a pinned workspace or region |
| `WindowPicker` | `swaypplet-window-picker` | panel | screenshot → window |
| `Polkit` | `swaypplet-polkit` | panel | the polkit / sudo card |
| `Report` | `swaypplet-report` | panel | the report card (`swaypplet report`) |
| `FaceCue` | `swaypplet-face-cue` | thin | the look-at-the-camera pill |
| `Keybinds` | `swaypplet-keybinds` | panel | the held-Super sheet, one per output |
| `Jump` | `swaypplet-jump` | none | the Super+Tab stage |
| `Screenshot` | `swaypplet-screenshot` | none, on purpose | the region selector's frozen screen |
| `Greeter` | `swaypplet-greeter` | the greeter compositor's own | the greeter's cards |
| `LockWarm` | `swaypplet-lock-warm` | none | the locker's one-pixel warm-up |
| `SessionLock` | `session-lock` | lock | not a layer namespace: the lock surfaces' block, borrowed by `preview:lock` |
| `MotionProbe` | `swaypplet-motion-probe` | none | used by no surface; asks sway what it parses |

A new surface adds a variant, and, if it wears glass, a row in
`sessionSurfaces` of the same class.

### 6.2 Layout rules

What the components do not decide: where things go on a surface. These
hold on every surface, and a surface that needs an exception writes it
down in its file.

1. **One primary per surface.** The launcher's field on the Helm, the
   password on the lock, the chosen pane in settings. Everything else is
   secondary in size, weight or tone, never in colour.
2. **A row reads label, then value.** In a column of settings the control
   sits at the row's far end at its natural width, never narrower than
   `.settings-control`, so a column of controls lines up on both edges
   (`settings::form::Placement`). A rail and a text field fill the row: the
   one needs the length, the other is read where it is typed. Every row
   stands on `--row-height-dense`.
3. **A column is read across in one glance.** Settings' card stops at 980
   px: the sidebar and a pane column of about 720. Past that a label and
   its value drift apart.
4. **A grid has one height.** The Helm's deck stands every control at
   `--tile-height`, square where it is a glyph, and the four switches
   share one line at equal widths (`panel::HelmLayout`). The card is a
   launcher's width (800), so the results list is read at one glance and
   the card stands on the screen rather than across it.
5. **A result says its name first and its place second.** A launcher row
   for a setting is the setting's name over its subtitle, with the pane
   and group in the end slot (faint caption); a group named after its pane
   is said once.
6. **A list is scanned by its first column.** The prefix list puts the
   page in a fixed gutter and what you type beside it, quieter.

## 7. Enforcement

`src/design_lint.rs`, run by `cargo test`. The CSS rules read every file in
`theme::RULES` with comments removed; the Rust rules read `src/**/*.rs`
minus `src/tokens/`, `src/ui/` and `#[cfg(test)]` code.

| Rule | Refuses | Where |
|---|---|---|
| `colour-literal` | hex, `rgb()`/`hsl()`…, named colours | all CSS |
| `colour-function` | `alpha()`, `shade()`, `mix()`, `lighter()`, `darker()`, `color-mix()` | all CSS |
| `at-name` | `@name` colours and `@define-color` | all CSS |
| `token` | a `var(--x)` the generator does not emit; a custom property defined outside the generator and `data/css/components/` | all CSS |
| `primitive` | `--neutral-n`, `--accent-n` | all CSS |
| `font-size`, `font-weight`, `radius`, `space`, `motion` | a value off its scale (§3.4–3.8); a looping animation names its own period | all CSS |
| `surface-look` | colour, type, shape or state motion in a surface's file | CSS outside `components/` |
| `component-scope` | a selector in `components/X.css` whose last compound targets no class X's `owns:` line names (a GTK node under one is fine); a class owned twice or by nobody; a component class in `gtk.css`. Justified reaches are listed with their reasons in `COMPONENT_SCOPE_EXCEPTIONS` | `data/css/components/` |
| `rust-space` | a non-zero literal box spacing or margin (use `ui::vbox(n)`, `ui::pad`, `tokens::space(n)`) | Rust |
| `rust-colour` | a Cairo or `gdk::RGBA` colour from numbers | Rust |
| `rust-class` | a class added that no stylesheet styles | Rust |
| `rust-ui-class` | a `"ui-…"` string literal: a component's class named outside the component | Rust |
| `motion-bypass` | a `transition_duration` set, or a motion token's `.ms` read, outside `ui::revealer`/`ui::page_stack` and `anim::ms`/`anim::span`, so Look → Motion and reduced motion reach every animation | Rust except `src/anim.rs` |
| `surface-on-window` | `ui::surface::adopt` or `ui::window::adopt` given a window (GTK's `window.background` outranks the class there; it goes on the root child) | Rust |
| `layer-window` | a layer-shell window made outside `src/shell/` (`create_layer_window*`, `init_layer_shell`, and `make_layer_window` outside the greeter): build a `shell::Surface` (§6.1) | Rust |
| `layer-namespace` | a namespace spelled as a string (`namespace: "…"`, `set_namespace` with a literal) outside `src/shell/namespace.rs` | Rust |

The lint also carries a ledger of files not yet migrated, which can only
shrink (empty today). Contrast (§5) is checked by the tests in
`src/tokens/`.

`swaypplet --preview components.<page>` (controls, inputs, lists, tiles;
`src/preview/components.rs`) draws every component in every variant and
state on one card, the pointer and keyboard states forced through GTK's
state flags, so a change to a component is judged against its siblings;
`docs/component-zoo.html` holds the decisions made on it.
`dev/render-all.sh capture DIR` renders every surface in both modes over
black and over a wallpaper; `dev/render-all.sh compare BEFORE AFTER` diffs
two such runs and writes a sheet for every shot that changed. It is the
check for a change that should move no pixel.

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

1. **Build with the components** (§6). Replace hand-built widgets with
   `src/ui/`: `ui::card::adopt` for the glass card, `ui::row` and
   `ui::row_button` for rows, `ui::section` for collapsible groups,
   `ui::button` for buttons, `ui::slider_row`, `ui::switch`, `ui::chip`,
   `ui::badge`, `ui::key`, `ui::status`, `ui::field`, `ui::menu`,
   `ui::progress`. Put text on the scale with `ui::text`,
   `ui::set_text_style` or `ui::glyph::adopt`. Box spacing is `ui::vbox(n)` /
   `ui::hbox(n)` with a step of the space scale.
2. **Shrink the surface's CSS to layout.** What stays in its file places
   things: padding, margins, min sizes, alignment, the apron around a
   card. Colour, font size and weight, radius and state belong to the
   components. Where a surface needs a look no component gives, add it to
   `data/css/components/` and `src/ui/` as a component, not to the surface.
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
