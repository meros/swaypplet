# Shells: prior art

**43 entries on desktop shells and OS-level desktop UI: what each introduced, what failed and why, and what swaypplet should take.** Sources are marked [verified] (read during this pass, 2026-09-26) or [memory]; treat [memory] claims as leads to check before building on them.

## The landscape

Every shell since Windows 95 splits the same jobs across the same few surfaces: a persistent bar (start target, running apps, status, clock), a transient control card (Control Center, Quick Settings, GNOME's pill grid), a notification list, a launcher that has become a search field, and a spatial overview of windows and workspaces (Exposé, GNOME Activities, Plasma Overview). The live differences are elsewhere: how much the bar changes at rest, whether the overview is a real spatial transition or a cut, how the material behaves over the wallpaper, and whether context (presenting, fullscreen) drives quiet modes. The Linux shells split between one opinionated layout (GNOME, elementary, COSMIC) and configurable containers (Plasma, Budgie, Xfce); the opinionated ones ship more coherent interaction, the configurable ones keep users through upgrades. Transparency comes back every decade (Aqua 2001, Liquid Glass 2025) and is walked back within a year or two for legibility.

## The 10 best ideas

1. **Windows fly from where they are** into the overview, so the animation explains the mapping ([mission-control-expose](mission-control-expose.md)).
2. **One spatial map for all navigation**: workspaces on one axis, zoom levels on the other, gestures bound to both ([gnome-activities-overview](gnome-activities-overview.md)).
3. **Overview and grid as states of one surface** on one continuous gesture ([kde-overview-effect](kde-overview-effect.md)).
4. **Quiet mode triggered by context** (presenting, fullscreen, game) with a summary afterwards ([windows-focus-auto-dnd](windows-focus-auto-dnd.md)).
5. **Wallpaper-tinted material sampled once**, at the cost of a static texture, with a transient/long-lived material rule ([windows-mica](windows-mica.md)).
6. **Dark style from sunset to sunrise and accent from the wallpaper**, shipped in 2021 ([elementary-pantheon](elementary-pantheon.md)).
7. **Split toggle**: body switches, chevron opens detail in place ([gnome-quick-settings](gnome-quick-settings.md)).
8. **Host decides tray placement from the app's declared status** ([statusnotifier-status-filter](statusnotifier-status-filter.md)).
9. **One launcher with modes** for apps, files, actions and clipboard, plus user abbreviations ([spotlight-shell](spotlight-shell.md)); **a command palette over the shell's own actions** ([unity-hud](unity-hud.md)).
10. **Per-workspace tiling as a visible property**, and applets out of process ([cosmic-desktop](cosmic-desktop.md)).

## The 8 instructive failures

1. **Invisible corners and edges as the only path** to core commands ([windows-8-start-screen](windows-8-start-screen.md), [hot-corners](hot-corners.md)).
2. **Feeds and promoted content in the shell**, opened on hover; reversed after five years ([windows-widgets-board](windows-widgets-board.md), [windows-11-start-menu](windows-11-start-menu.md)).
3. **Local search sent to the network by default** ([unity-launcher-dash](unity-launcher-dash.md)); telemetry in shell apps ([deepin-dde](deepin-dde.md)).
4. **Legibility left to a user toggle** over clear glass ([liquid-glass-macos](liquid-glass-macos.md), [macos-menu-bar](macos-menu-bar.md)).
5. **Features that need every app to adopt a new API** ([windows-timeline](windows-timeline.md), [unity-hud](unity-hud.md), [kde-activities](kde-activities.md)).
6. **In-process plugins with no stable surface**, broken at every release ([gnome-shell-extensions](gnome-shell-extensions.md), [haiku-deskbar](haiku-deskbar.md) replicants).
7. **Removing long-held layout options in a rewrite** ([windows-11-taskbar](windows-11-taskbar.md), restored 2026; the 2011 GNOME Shell exodus in [cinnamon](cinnamon.md)).
8. **Mouse-only grouping models without keyboard verbs** and hard item limits ([stage-manager](stage-manager.md)).

## Ideas for swaypplet

Ranked by value for the effort. Sizes as in the roadmap (S a day, M a few days, L a week or more).

| # | Idea | Size | From |
|---|---|---|---|
| 1 | Context quiet mode: add "presenting" (duplicate or mirrored output) and "screen capture active" triggers, on by default, and keep the end-of-session summary card Windows 11 dropped. Refines ROADMAP item 3 | M | [windows-focus-auto-dnd](windows-focus-auto-dnd.md), [macos-notification-center](macos-notification-center.md) |
| 2 | Overview and Super+Tab strip as zoom levels of one surface on one axis, windows moving from their real rects; embed launcher search. Shapes ROADMAP item 4 | L | [mission-control-expose](mission-control-expose.md), [kde-overview-effect](kde-overview-effect.md), [gnome-activities-overview](gnome-activities-overview.md) |
| 3 | Publish `org.freedesktop.appearance.color-scheme` (and accent) through the settings portal from the sun-driven mode, so apps switch with the shell | S | [elementary-pantheon](elementary-pantheon.md) |
| 4 | Split toggles in the panel: body toggles, chevron expands the section in place with the shared-element motion; "on" shown by fill and shape, not hue alone | M | [gnome-quick-settings](gnome-quick-settings.md), [macos-control-center](macos-control-center.md) |
| 5 | Launcher action mode: search panel toggles, sway commands and settings panes by name, with user abbreviations | M | [unity-hud](unity-hud.md), [spotlight-shell](spotlight-shell.md) |
| 6 | Per-item tray override in the panel (always show, always hide) for apps that misuse `Active` | S | [statusnotifier-status-filter](statusnotifier-status-filter.md) |
| 7 | Panel row for background apps (the tray's `Passive` items and portal background apps) | S | [gnome-top-bar-no-tray](gnome-top-bar-no-tray.md) |
| 8 | Lock screen: typing on the curtain goes to the password field; summary row as per-app counts, no content | S | [lock-screen-curtain](lock-screen-curtain.md) |
| 9 | greetd greeter drawn by swaypplet, sharing AUTH_CARD, with a greeter-to-session transition | L | [display-managers-greeters](display-managers-greeters.md) |
| 10 | Panel control for the focused container's layout (split, tabbed, stacked) as a visual picker | S | [windows-snap-layouts](windows-snap-layouts.md), [cosmic-desktop](cosmic-desktop.md) |
| 11 | Audio output picker inside the media popover | S | [budgie-raven](budgie-raven.md) |
| 12 | Bar body tied to focus: slightly more frost on outputs without focus | S | [windows-mica](windows-mica.md) |

Confirmed rejections: weather and feeds ([windows-widgets-board](windows-widgets-board.md)), hover-opened surfaces ([windows-snap-layouts](windows-snap-layouts.md)), looping attention motion ([macos-dock](macos-dock.md), [mac-os-x-aqua](mac-os-x-aqua.md)), dynamic renumbering of keyed workspaces ([gnome-dynamic-workspaces](gnome-dynamic-workspaces.md)), a second grouping axis over workspaces ([kde-activities](kde-activities.md)), free-form widget containers ([kde-plasma-panels-widgets](kde-plasma-panels-widgets.md)).

## All entries

macOS: [macos-menu-bar](macos-menu-bar.md), [macos-dock](macos-dock.md), [macos-control-center](macos-control-center.md), [macos-notification-center](macos-notification-center.md), [mission-control-expose](mission-control-expose.md), [stage-manager](stage-manager.md), [spotlight-shell](spotlight-shell.md), [liquid-glass-macos](liquid-glass-macos.md), [mac-os-x-aqua](mac-os-x-aqua.md), [nextstep-dock](nextstep-dock.md).
Windows: [windows-95-start-menu](windows-95-start-menu.md), [windows-8-start-screen](windows-8-start-screen.md), [windows-11-taskbar](windows-11-taskbar.md), [windows-11-start-menu](windows-11-start-menu.md), [windows-quick-settings](windows-quick-settings.md), [windows-snap-layouts](windows-snap-layouts.md), [windows-widgets-board](windows-widgets-board.md), [windows-mica](windows-mica.md), [windows-timeline](windows-timeline.md), [windows-focus-auto-dnd](windows-focus-auto-dnd.md).
GNOME: [gnome-activities-overview](gnome-activities-overview.md), [gnome-quick-settings](gnome-quick-settings.md), [gnome-dynamic-workspaces](gnome-dynamic-workspaces.md), [gnome-shell-extensions](gnome-shell-extensions.md), [gnome-top-bar-no-tray](gnome-top-bar-no-tray.md).
KDE: [kde-plasma-panels-widgets](kde-plasma-panels-widgets.md), [kde-floating-panel](kde-floating-panel.md), [kde-overview-effect](kde-overview-effect.md), [kde-activities](kde-activities.md), [statusnotifier-status-filter](statusnotifier-status-filter.md).
Other: [chromeos-shelf-launcher](chromeos-shelf-launcher.md), [cosmic-desktop](cosmic-desktop.md), [elementary-pantheon](elementary-pantheon.md), [deepin-dde](deepin-dde.md), [budgie-raven](budgie-raven.md), [cinnamon](cinnamon.md), [unity-launcher-dash](unity-launcher-dash.md), [unity-hud](unity-hud.md), [haiku-deskbar](haiku-deskbar.md), [xfce-lxqt-panels](xfce-lxqt-panels.md).
Patterns: [lock-screen-curtain](lock-screen-curtain.md), [display-managers-greeters](display-managers-greeters.md), [hot-corners](hot-corners.md).
