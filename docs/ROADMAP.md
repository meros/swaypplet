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

| Item | Branch | Detail |
|---|---|---|
| Colour and material transitions: a mode, tint or wallpaper change fades over `page` (500 ms) instead of cutting | `theme-anim` | [design-system.md §2](design-system.md), [MOTION.md](MOTION.md) |
| No 1-second polling: the theme follows its files through file monitors | main | |

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

### 2. Display profiles in the process (M), after night light

Night light is in the process since 2026-09-26 (`services::gamma`). The
other half retires kanshi: match
and apply output profiles in the panel over `zwlr_output_manager_v1`, which
applies every output in one step and reports success or failure. Sway IPC
stays for moving workspaces. Profiles live in a `displays` settings section
with Nix defaults; kanshi's matching carries over (every output claimed,
first match wins, the current profile stays while it matches). About 1,300
lines. Design, sources and risks: [research/display-profiles.md](research/display-profiles.md).

### 3. Focus and quiet modes that know the context (M)

Do not disturb switches on by itself while a screen is shared or a window is
fullscreen, and off after. Notifications that arrive meanwhile do not pop
up; afterwards one card says "7 while you were presenting" and opens the
list. The lock screen gets the same summary row. The screen-share signal is
the privacy indicator's (camera and screen capture are the blocked half of
that item in [history](history/shell-ideas-2026-08.md)).

### 4. The overview with a real start (L)

A long Super hold opens a full-screen overlay whose first frame is an output
capture, identical to the screen; the windows then fly from their real
positions into a grid of workspaces. Needs the dmabuf capture path first
(`GdkDmabufTextureBuilder`, gtk4 `v4_16`): full-size frames through shm cost
a CPU copy and a box filter each, which the Super+Tab tiles can afford and a
full screen cannot. Builds on item 1's motion.

### 5. Emoji and character picker that types (M)

`zwp_input_method_manager_v2` and `zwp_virtual_keyboard_manager_v1` are
both advertised, so the picker inserts into the focused surface instead of
going through the clipboard. The dmenu chassis is most of the UI. Could
also be a launcher prefix once the `launcher` branch lands.

### 6. Tailscale in the network section (S)

Exit-node state and peer reachability. `tailscaled` is its own daemon, so
the NetworkManager VPN rows do not see it; the network code only filters
out the `tailscale0` interface today.

## Research

| Topic | Status | Detail |
|---|---|---|
| Glass that responds to what is behind it | Researched, not scheduled. Photochromic already covers brightness. The one experiment worth running: a busyness term in the shader (4 extra frost taps, under 0.1 ms a frame) that adds frost and body over a busy backdrop and is zero over a flat one, so every contrast test holds. Stop if it shows no visible legibility win. No per-surface readback, no light/dark flip per surface | [research/adaptive-glass.md](research/adaptive-glass.md) |
| Display profiles in the process (kanshi's replacement) | Researched, design written, not started | [research/display-profiles.md](research/display-profiles.md) |
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
  in the panel, and item 3 makes it automatic.
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
