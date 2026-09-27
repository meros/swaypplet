# Roadmap

**The one list of what swaypplet builds next, ranked.** Each item says what
it is, why it is worth doing now, and where its detail lives. What shipped
is in the code and the git log; what was decided against is under
[Rejected](#rejected), with the reason, so it is not proposed again.

Last pruned 2026-09-26. The old idea files (`BAR_IDEAS.md`,
`STARTMENU_LAYOUT_PLAN.md`) are gone: their open items are below, their
shipped items are in `src/`, and the rest was rejected by
[BAR_VISION](BAR_VISION.md) or is listed here. `SHELL_IDEAS.md` moved to
[history/](history/shell-ideas-2026-08.md) as the record of what the
August pass shipped and why.

Sizes: S is a day or less, M a few days, L a week or more.

## In progress

Nothing. Everything started on 2026-09-26 is merged: the glass page,
colour transitions, the launcher, night light and display profiles in the
process, grouped notifications, the shared peek and pin, lock text that
follows the wallpaper, event-driven updates with no polling, the frame
gate, and the rebuilt network, audio, Bluetooth, power and tile settings.

## Next

### 1. Shared-element motion between surfaces (M–L)

An element that lives in two places moves between them instead of fading
out in one and in at the other: a notification popup into the notification
centre, a bar segment into its panel section, a Super+Tab tile into the full
workspace. Two concrete pieces, smallest first:

- **Sliding focus caret** (S). A 3 px accent caret under the workspace
  buttons slides to the focused one over `move` (300 ms); task workspaces
  colour it with their categorical colour. Needs `SlideBin` to take a 2-axis
  offset, which the tile piece also needs. BAR_VISION already names it
  as compatible backlog.
- **Workspace tile to workspace** (L). Super+Tab's chosen tile grows into
  the real workspace. Uses the per-output node restriction swayfx already
  has for the switcher strip.

Why now: the motion tokens (`move`, `travel`) and `anim::Reveal` exist, and
the design system says what each duration means, so this is the last
motion piece without a home.

### 2. The overview with a real start (L)

A long Super hold opens a full-screen overlay whose first frame is an output
capture, identical to the screen; the windows then fly from their real
positions into a grid of workspaces. Needs the dmabuf capture path first
(`GdkDmabufTextureBuilder`, gtk4 `v4_16`): full-size frames through shm cost
a CPU copy and a box filter each, which the Super+Tab tiles can afford and a
full screen cannot. Builds on item 1's motion.

### 3. Emoji and character picker that types (M)

`zwp_input_method_manager_v2` and `zwp_virtual_keyboard_manager_v1` are
both advertised, so the picker inserts into the focused surface instead of
going through the clipboard. The dmenu chassis is most of the UI. Could
also be a launcher prefix once the `launcher` branch lands.

## Waiting for a decision

- **Light-mode lock text over a dark or busy wallpaper.** Since 2026-09-26
  a dark halo in light mode stays under the glass mask, or it turns into
  milky glass. Light ink there reaches Lc 61 (calm) and 47 (busy) against
  75. Two fixes, both visible design changes: a glass plate behind the
  clock and date in light mode (the text then uses the card's tokens), or
  a deeper scrim over the wallpaper in light mode. `tokens/backdrop.rs`
  names the floors the tests hold.

## From the prior-art bank

Candidates from [prior-art/](prior-art/README.md), not yet ranked against
the items above. Each names the entries it comes from.

| Candidate | Size | Why | Source |
|---|---|---|---|
| Tell the apps: publish color-scheme, accent, contrast and reduced motion through the XDG settings portal and gsettings when the mode changes. Today nixos `scaling.nix` pins apps to `prefer-dark`, so in light mode the apps stay dark | S | the most visible gap in light mode | [xdg-portal-appearance](prior-art/theming/xdg-portal-appearance.md) |
| Per-app demotion of critical ("this app's critical is not critical"): the rest of the interruption levels shipped with quiet by context | S | freedesktop has no entitlement gate, so some apps send critical for trivia | [ios-interruption-levels](prior-art/notifications-wm/ios-interruption-levels.md) |
| The lock screen's summary row for quiet by context: the "N while you were presenting" count, per app, under the clock | S | the one part of the context item not built; the card covers the unlocked case | [windows-focus-assist](prior-art/notifications-wm/windows-focus-assist.md), [lock-screen-curtain](prior-art/shells/lock-screen-curtain.md) |
| Frame gate v2: lateness relative to refresh, per-animation tags, the compositor's render time, the shipped glass settings, a cold first open | S–M | the gate should say who dropped the frame | [motion/README](prior-art/motion/README.md) |
| Launcher learns (prefix → item), fallback rows, per-row actions, `nucleo` fuzzy matching | S–M each | ranked in the launchers synthesis | [launchers/README](prior-art/launchers/README.md) |
| Settings rows as launcher results, opening the pane on the row | M | every other platform searches settings | [settings-search](prior-art/theming/settings-search.md) |
| A solid twin of the glass for Reduce Transparency, high contrast and power saving | M | Apple and Microsoft both keep one | [apple-vibrancy-materials](prior-art/motion/apple-vibrancy-materials.md) |
| Export the tokens (base16, libadwaita `gtk.css`, Qt palette, ANSI) so terminals and apps follow | M | follows "tell the apps" | [matugen](prior-art/theming/matugen.md), [stylix](prior-art/theming/stylix.md) |
| Velocity kept when an animation is interrupted (`Reveal` keeps position only) | M | interruptions read as a jolt | [swiftui-spring-animations](prior-art/motion/swiftui-spring-animations.md) |
| `ext-background-effect-v1` in the swayfx patch: per-card blur regions | L | fixes frost on transparent gaps; KWin, niri, Mutter have it | [ext-background-effect](prior-art/wayland/ext-background-effect.md) |
| `xdg-activation-v1` tokens from the launcher, pins and notification actions | S | correct focus for what swaypplet raises | [xdg-activation](prior-art/wayland/xdg-activation.md) |

## Research

| Topic | Status | Detail |
|---|---|---|
| Glass that responds to what is behind it | Researched, not scheduled. Photochromic already covers brightness. The one experiment worth running: a busyness term in the shader (4 extra frost taps, under 0.1 ms a frame) that adds frost and body over a busy backdrop and is zero over a flat one, so every contrast test holds. Stop if it shows no visible legibility win. No per-surface readback, no light/dark flip per surface | [research/adaptive-glass.md](research/adaptive-glass.md) |
| Display profiles in the process (kanshi's replacement) | Built 2026-09-26 (`services::displays`); the research is the design record | [research/display-profiles.md](research/display-profiles.md) |
| Liquid glass optics | Reference for the shader's parameters | [research/liquid-glass.md](research/liquid-glass.md) |

## Rejected

Each was proposed at least once. The reason is why it stays out.

- **Bar-to-panel morph** (the panel card growing out of the start
  button). Dropped by the owner 2026-09-26.
- **claude-dash.** Retired 2026-09-26 (nixos `35e9de5`): not used; the bar's
  task board and popover show the same sessions.
- **Weather, calendar, agenda, clock calendar popover.** A network poll or a
  per-minute tick for a surface opened twice a day (BAR_VISION P7).
- **Notification bell with an unread badge.** A count on the bar is colour
  and change at rest; BAR_VISION P1 keeps the nominal bar still. DND lives
  in the panel, and quiet by context (`services/notifications/context.rs`)
  switches it by itself.
- **Bar toggle segments (night light, caffeine) on the bar.** Bar width is
  for state, not controls; the panel holds the tiles.
- **Media playback underline and hover controls on the bar.** Continuous
  motion for a state that needs no action, and controls reachable only by
  hover (BAR_VISION P2, P8). The media popover has seek and controls.
- **Indeterminate working arc and breathing pulses.** Frame-clock loops for
  hours (BAR_VISION P2).
- **Per-app context segment.** Reshapes the bar on the most frequent event
  in the workflow.
- **Clipboard persistence across restarts.** How clipboard managers leak
  passwords to disk. Revisit only with an encryption story.
- **Image entries in clipboard history.** A real want, but the preview,
  memory cap and eviction are a separate design; not worth it yet.
