---
name: Qtile
slug: qtile
domain: wayland
kind: product
platform: Wayland
vendor: Qtile community
years: 2008–present
status: niche
tags: [python, tiling, wm-with-bar, x11, wayland]
relevance: low
---

## What it is
A tiling window manager written and configured in Python, with the bar and widgets built into the WM. It runs on X11 and, through a wlroots backend, on Wayland.

## What was new
- WM, layouts, keybindings and bar share one language and one process, so a widget can call layout functions directly.
- Hot reload of the Python config.

## What went wrong / limits
- The Wayland backend depended on pywlroots bindings that lagged wlroots releases; Wayland support stayed second-class for years [memory].
- Python in the compositor process means GIL stalls stall the display.

## Lessons for swaypplet
- Avoid scripting in the frame path; a typed Rust shell outside the compositor is the right split.

## Sources
- https://docs.qtile.org/ — docs, Wayland backend [memory]
