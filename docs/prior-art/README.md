# Prior-art databank

**332 entries on how operating systems and shells have done what swaypplet
does: what worked, what was new, what failed and why.** Written 2026-09-26
by seven research passes, one per domain. Each entry is one Markdown file
with a fixed frontmatter (see [TEMPLATE.md](TEMPLATE.md)), so the bank can
be read, grepped or queried.

- [INDEX.md](INDEX.md): every entry in one table per domain, sorted by
  relevance to swaypplet. Generated; do not edit.
- `index.json`: the same records for scripts, e.g.
  `jq -r '.[] | select(.status=="failed") | .name' docs/prior-art/index.json`.
- `dev/prior-art-index.py`: checks every entry (fields, sections, slug =
  file name, domain = folder) and regenerates both. Run it after adding one.

Status across the bank: 247 current, 30 discontinued, 23 failed, 26 niche,
6 research. Relevance: 141 high, 135 medium, 56 low.

**Limits.** Most sources marked [verified] were read through search
summaries, not full pages; later entries in the shells and motion domains
lean on [memory] after the search budget ran out. Treat a single [memory]
claim as a lead, not a fact.

## The domains

| Domain | Entries | Synthesis |
|---|---|---|
| Desktop shells and OS desktop UI | 43 | [shells/README.md](shells/README.md) |
| Mobile, wearable, TV and spatial UI | 42 | [mobile/README.md](mobile/README.md) |
| Launchers, command palettes and search | 49 | [launchers/README.md](launchers/README.md) |
| Wayland, tiling and custom shells | 50 | [wayland/README.md](wayland/README.md) |
| Theming, colour, materials and settings | 50 | [theming/README.md](theming/README.md) |
| Notifications, focus, widgets, window management, failures | 53 | [notifications-wm/README.md](notifications-wm/README.md) |
| Motion, rendering and performance | 45 | [motion/README.md](motion/README.md) |

Thirteen subjects appear in two domains, each for its own lesson; INDEX.md
lists them.

## What several domains found independently

These came up in three or more domains without the researchers seeing each
other's work. They are the strongest signals in the bank.

1. **Quiet mode that knows the context, and says what it held.** Windows
   Focus Assist, Plasma, iOS interruption levels and swaync all converge:
   switch on for a mirrored output, active screen capture or a fullscreen
   window; name the trigger on the tile; announce nothing at the start and
   one "7 while you were presenting" card at the end. Critical breaks
   through, low urgency never pops. (shells, mobile, notifications-wm,
   wayland; roadmap item 3.)
2. **Tell the apps.** Every major desktop exports its appearance through
   the XDG settings portal and gsettings; swaypplet exports only sway's
   borders, and `~/nixos/users/modules/scaling.nix` pins apps to
   `prefer-dark`, so in light mode the shell turns light and the apps stay
   dark. Publish color-scheme, accent, contrast and reduced motion; later,
   export the tokens as base16, a libadwaita `gtk.css`, a Qt palette and
   ANSI colours. (shells, theming.)
3. **One overview, many zoom levels.** Mission Control, the KDE Overview
   and the GNOME 40 overview keep windows moving from their real positions;
   Super+Tab and the overview should be one surface on one axis, never
   reordered by recency. (shells, notifications-wm, mobile; roadmap item 4.)
4. **A solid twin for the glass.** Apple (Reduce Transparency, the tinted
   step added after Liquid Glass shipped) and Microsoft (Mica falls back to
   solid on battery saver) both keep an opaque material beside the glass:
   for accessibility, for high contrast, for power saving. (mobile,
   theming, motion.)
5. **The launcher learns queries, not items.** Quicksilver, LaunchBar and
   Firefox map a typed prefix to the item picked for it; add fallback rows
   when a query finds little, an action list per row, a scored fuzzy
   matcher, and settings rows as results. (launchers, shells, theming.)
6. **Measure motion the way the platforms do.** Lateness relative to the
   refresh rate, per-animation attribution, the compositor's own render
   time, a cold first open, and the shipped glass settings in the bench;
   keep velocity when an animation is interrupted. (motion, wayland.)

## The failures, by cause

Twenty-three entries failed outright and thirty were discontinued. The
causes repeat:

- **Attention at rest.** Live Tiles, Dashboard, the Vista sidebar, the
  Windows 11 widgets board: a surface that moves or updates when nothing
  needs the user trains them to ignore it. BAR_VISION's P1 and P2 already
  encode this.
- **A second way to group work.** KDE Activities beside virtual desktops,
  Windows Sets tabs beside windows: two axes nobody could keep in mind.
- **Network content in local chrome.** The Unity Shopping Lens, Ubuntu
  Touch scopes, Google Now cards: latency, privacy, and results nobody
  asked for.
- **A mode change forced on the user.** Windows 8's full-screen Start, Unity
  convergence, KDE 4.0 and GNOME 3.0 at launch: the model changed before
  the old one's habits were served.
- **Rewriting what the user receives.** Apple Intelligence notification
  summaries misreported news and were paused; a summary line must be
  mechanical and styled apart from sender text.
- **Capturing too much.** Windows Recall: screenshots on disk by default.
  Workspace pictures stay in memory.
- **Extensions in the shell's process.** GNOME extensions and Plasma
  widgets break on every release; the Wayland ricing scene loses shells to
  maintainer burnout. Keep providers out of process, as elephant does.

## Where swaypplet already stands

Ahead: contrast tested through the glass material (no shipping system
publishes that), bounded and tested wallpaper tinting, a sun-deferred mode
switch that only macOS matches, a lint that enforces the token tiers, and
no network in the launcher's default path. Behind: reach to other apps,
settings search, and accessibility breadth (forced colours, colour-vision
presets, a user-facing transparency switch).

The candidates this produced are in [ROADMAP.md](../ROADMAP.md) under
"From the prior-art bank".
