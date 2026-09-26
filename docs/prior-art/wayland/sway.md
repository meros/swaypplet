---
name: Sway
slug: sway
domain: wayland
kind: product
platform: Wayland
vendor: Drew DeVault, then Simon Ser and the swaywm community
years: 2016–present
status: current
tags: [compositor, tiling, i3, wlroots, ipc]
relevance: high
---

## What it is
A drop-in i3 replacement for Wayland built on wlroots. It reads i3's config format, speaks i3's JSON IPC over a Unix socket (with event subscriptions), and ships swaybar, which speaks the i3bar protocol. It is the reference consumer of wlroots, so new `ext-*` protocols tend to land in sway shortly after a wlroots release.

## What was new
- Proved a tiling WM could move to Wayland without breaking its users' config or scripts: i3 IPC clients (py-i3ipc, swayipc) work against both.
- Conservative protocol policy: sway 1.11 (wlroots 0.19) added `ext-image-copy-capture-v1` and `ext-image-capture-source-v1`; sway 1.12 (wlroots 0.20) added `ext-workspace-v1` and HDR10/colour management.
- Treats the bar, lock, idle and notifications as separate programs linked by standard protocols, which is what made the whole third-party shell ecosystem possible.

## What went wrong / limits
- Deliberately refuses eye candy (blur, rounded corners, animations); that refusal produced swayfx and pushed many users to Hyprland.
- No animations at all, so any shell motion that depends on the compositor (window fly-in, workspace slide) is impossible upstream.
- The first `ext-image-copy-capture` PR shipped without per-toplevel capture.

## Lessons for swaypplet
- Keep sway IPC for what only sway knows (tree, workspaces, marks), and prefer an `ext-*` protocol wherever one exists so the code survives a compositor change.
- `ext-workspace-v1` is now in sway 1.12: the bar's workspace list could move off IPC once swayfx rebases.

## Sources
- https://github.com/swaywm/sway/releases/tag/1.11 — 1.11 protocol list [verified via search summary]
- https://www.phoronix.com/news/wlroots-0.20-Sway-1.12-rc1 — 1.12 / wlroots 0.20, ext-workspace [verified via search summary]
- https://github.com/swaywm/sway/pull/7976 — ext-image-copy-capture PR, no toplevel capture yet [verified via search summary]
