# Prior art: Wayland, tiling and custom shells

**50 entries on the compositors, shell toolkits, riced shells, protocols and small daemons that swaypplet sits among.** Sources are marked per entry; "verified via search summary" means read through a search result, not the full page. Written 2026-09-26.

## Landscape

Three layers, each fragmenting on its own schedule.

- **Compositors.** [sway](sway.md) is the conservative reference; [swayfx](swayfx.md) adds effects by fork; [Hyprland](hyprland.md) sells looks and churns config; [niri](niri.md) is where the energy moved in 2025–26 (scrollable tiling, overview, protocol blur); [river](river.md) split the WM into a separate process; [COSMIC](cosmic.md) shipped a Rust desktop with tiling built in. GNOME ([gnome-shell-mutter](gnome-shell-mutter.md)) still refuses layer-shell, so the whole ecosystem below excludes it.
- **Shell toolkits.** The GTK3 generation ([Waybar](waybar.md), [eww](eww.md), [AGS](ags-astal.md), [Fabric](python-shell-frameworks.md)) is giving way to [Quickshell](quickshell.md) (QML) and compiled GTK4 ([ironbar](ironbar.md), [Wayle](hyprpanel-wayle.md)). Riced shells ([end-4](end4-illogical-impulse.md), [Caelestia](caelestia.md), [DankMaterialShell](dankmaterialshell.md), [Noctalia](noctalia.md)) moved to Quickshell en masse.
- **Protocols.** The `ext-*` set now covers what a shell needs: [session lock](ext-session-lock.md), [image capture](ext-image-copy-capture.md), [toplevel list](foreign-toplevel.md), [workspaces](ext-workspace.md), [background effect](ext-background-effect.md). Output management and gamma remain `wlr-`.

## The 10 best ideas

1. **Client-requested blur by region**, applied by the compositor, including niri's cheap "x-ray" mode that blurs the wallpaper once ([ext-background-effect](ext-background-effect.md), [niri](niri.md)).
2. **Overview that zooms out from the real frame** with keyboard focus intact ([niri](niri.md), [gnome-shell-mutter](gnome-shell-mutter.md)).
3. **Hot reload of the shell** in under a second ([Quickshell](quickshell.md)).
4. **Atomic multi-output apply with a dry run** ([wlr-output-management](wlr-output-management.md)), plus shikane's ranked matching and "save current layout as profile" ([output-profile-tools](output-profile-tools.md)).
5. **Learning from manual corrections** instead of curves ([wluma](wluma.md)).
6. **Suspend waits for the lock surface** ([idle-stack](idle-stack.md), [ext-session-lock](ext-session-lock.md)).
7. **Notification modes with rules** rather than a DND bit ([mako](mako-dunst-fnott.md)); inline reply and a visible "who is inhibiting" row ([swaync](swaync.md)).
8. **Grace period after an idle lock** ([lockers](lockers.md)).
9. **Policy out of process, commits atomic** ([river](river.md)).
10. **Sensitive-MIME skip for clipboard history** ([clipboard-managers](clipboard-managers.md)); swaypplet already does it.

## 8 instructive failures

1. **Stack burnout**: HyprPanel archived in April 2026, the author citing GJS and dependency churn; rewritten as Wayle in Rust and GTK4 ([hyprpanel-wayle](hyprpanel-wayle.md)). Ignis is also leaving Python for Rust ([python-shell-frameworks](python-shell-frameworks.md)).
2. **Building on a host's internals**: Material Shell, Pop Shell, Forge, PaperWM and Bismuth were all broken by host releases or abandoned ([material-shell](material-shell.md), [pop-shell](pop-shell.md), [gnome-tiling-extensions](gnome-tiling-extensions.md), [scrollable-tiling-extensions](scrollable-tiling-extensions.md), [kwin-tiling](kwin-tiling.md)).
3. **No stable plugin ABI**: every Hyprland update rebuilds every plugin; hyprexpo was dropped upstream ([hyprland-plugin-api](hyprland-plugin-api.md)).
4. **Polling via exec**: Waybar custom modules pinning a core, eww `defpoll` configs ([waybar](waybar.md), [eww](eww.md)).
5. **Framework rewrites breaking every downstream**: AGS v1 to v2 ([ags-astal](ags-astal.md)).
6. **Protocols stuck in draft**: ext-workspace took about five years, so every bar wrote per-compositor backends ([ext-workspace](ext-workspace.md)).
7. **Half protocols**: ext-foreign-toplevel-list cannot activate or close; tools still need the wlr protocol ([foreign-toplevel](foreign-toplevel.md)).
8. **Effects-first compositor lost its biggest user on performance**: Raspberry Pi left Wayfire for labwc ([wayfire](wayfire.md)).

## Ideas for swaypplet, ranked

| # | Idea | Size | Source |
|---|---|---|---|
| 1 | Implement `ext-background-effect-v1` in the swayfx patch; swaypplet sets per-card blur regions, fixing binary frost on transparent gaps and giving GTK apps blur | L | [ext-background-effect](ext-background-effect.md), [swayfx](swayfx.md) |
| 2 | Display profiles: `test` before `apply`, shikane-style ranking, "save current layout" as the way to create a profile, DPI-derived default scale | fits roadmap item 2 (M) | [wlr-output-management](wlr-output-management.md), [output-profile-tools](output-profile-tools.md) |
| 3 | Quiet mode as rule-based modes ("presenting" still pops calls and critical), plus a "who is inhibiting" row | fits item 3 (M) | [mako-dunst-fnott](mako-dunst-fnott.md), [swaync](swaync.md) |
| 4 | x-ray tier: a pre-blurred wallpaper texture as the glass backdrop for the bar at rest (cheap, no smear) | M | [niri](niri.md) |
| 5 | Pass xdg-activation tokens from launcher, pins and notification actions | S | [xdg-activation](xdg-activation.md) |
| 6 | Lock grace period after idle lock; test hotplug-while-locked covers the new output in one frame | S | [lockers](lockers.md), [ext-session-lock](ext-session-lock.md) |
| 7 | Hot-reload CSS and tokens while developing | S–M | [quickshell](quickshell.md), [ironbar](ironbar.md) |
| 8 | Auto-brightness that learns from brightness-key presses, sampled from the existing capture path | M | [wluma](wluma.md) |
| 9 | Inline reply in notifications | M | [swaync](swaync.md) |
| 10 | Screenshot defaults: freeze before select, single-key tools, copy-and-close | S | [screenshot-tools](screenshot-tools.md) |
| 11 | Damage-aware tile updates, dmabuf for the overview (already planned) | fits item 4 | [ext-image-copy-capture](ext-image-copy-capture.md) |

Rejected here: Caelestia's morph shows the bar-to-panel morph can look right, but the owner's 2026-09-26 rejection stands ([caelestia](caelestia.md)).

## Protocols swaypplet does not use yet

swaypplet binds ext-foreign-toplevel-list, ext-image-copy-capture (output and toplevel), ext-session-lock, ext-idle-notify, ext-data-control, wlr-gamma-control, wp-alpha-modifier and ext-transient-seat. Worth adding:

- **`ext-background-effect-v1`**: needs the swayfx side first; biggest visual payoff.
- **`zwlr_output_manager_v1`**: planned (roadmap item 2).
- **`xdg-activation-v1`**: correct focus for everything swaypplet launches or raises.
- **`zwlr_foreign_toplevel_manager_v1`**: activate, close and minimise from pins and the switcher without sway IPC strings; optional while IPC works.
- **`ext-workspace-v1`**: once swayfx is on wlroots 0.20; portability, low urgency.
- **`zwp_input_method_v2` / `zwp_virtual_keyboard_v1`**: planned for the emoji picker (item 5).
