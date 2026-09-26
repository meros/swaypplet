---
name: Xfce and LXQt panels
slug: xfce-lxqt-panels
domain: shells
kind: product
platform: other
vendor: Xfce, LXQt projects
years: 1996–present
status: current
tags: [panel, lightweight, conservative, plugins, x11]
relevance: low
---

## What it is
Xfce (GTK) and LXQt (Qt) are lightweight traditional desktops with configurable panels of plugins (menu, window buttons, pager, tray, clock). Xfce 4.20 (December 2024) began experimental Wayland support; LXQt 2.x runs on Wayland through a choice of compositors (labwc, KWin, wayfire, Hyprland, niri).

## What was new
Little in interaction; the value is stability and low resource use. LXQt's approach of letting the user pick the Wayland compositor underneath its panel is close to what swaypplet is: a shell on someone else's compositor.

## What went wrong / limits
- Slow evolution: Xfce's Wayland port took years and depends on wlroots protocols.
- Features that need compositor cooperation (window previews, blur, overview) are missing or depend on the chosen compositor.

## Lessons for swaypplet
- Note: a shell separate from its compositor is limited by the protocols the compositor exposes; swaypplet's patched swayfx is the way around that, and the cost is maintaining the patches.

## Sources
- https://www.xfce.org/about/news — 4.20 release and Wayland status [memory]
- https://lxqt-project.org/release/ — LXQt 2.x Wayland session options [memory]
