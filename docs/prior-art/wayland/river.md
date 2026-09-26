---
name: river
slug: river
domain: wayland
kind: product
platform: Wayland
vendor: Isaac Freund
years: 2020–present
status: current
tags: [compositor, zig, window-management-protocol, separation]
relevance: medium
---

## What it is
A Zig wlroots compositor that, from 0.4, contains no window manager. The WM is a separate process speaking the stable `river-window-management-v1` protocol and decides window position and size, keybindings, focus, decorations and even shell graphics; river keeps rendering, input plumbing and frame timing.

## What was new
- Splits policy from mechanism across a process boundary while keeping frame-perfect rendering: the WM proposes a layout, river applies it atomically in one frame.
- Before 0.4, `river-layout-v3` already let layout generators (rivertile, stacktile) run out of process.
- A WM crash does not take down the session.

## What went wrong / limits
- Every WM author now writes and maintains a Wayland client with non-trivial state; the ecosystem is small.
- The 0.3 to 0.4 break forced users to pick a new WM.

## Lessons for swaypplet
- The shell-as-separate-process model with atomic, protocol-level commits is the right shape; swaypplet should push any swayfx behaviour it needs into a small protocol rather than IPC string commands.
- A crash of the shell must never take down the session (already true; keep it).

## Sources
- https://isaacfreund.com/blog/river-window-management/ — design rationale [verified via search summary]
- https://linuxiac.com/river-0-4-wayland-compositor-debuts-pluggable-window-managers/ — 0.4 release [verified via search summary]
