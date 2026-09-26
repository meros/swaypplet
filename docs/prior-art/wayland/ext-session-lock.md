---
name: ext-session-lock-v1
slug: ext-session-lock
domain: wayland
kind: protocol
platform: Wayland
vendor: Isaac Freund; wayland-protocols
years: 2022–present
status: current
tags: [protocol, lock-screen, security, failsafe]
relevance: high
---

## What it is
A staging protocol for lock screens: the client asks to lock, the compositor blanks all outputs and confirms `locked` only when every output shows a lock surface or is blanked. If the locker dies without unlocking, the session stays locked. swaylock, hyprlock, Quickshell's `WlSessionLock` and swaypplet's lock use it.

## What was new
- Security moved into the compositor: no window can appear above the lock, and a crash cannot unlock.
- The `locked` event lets a suspend path wait until the screen is actually covered before sleeping (hypridle's `inhibit_sleep = 3` uses this).

## What went wrong / limits
- The crash failsafe is a solid colour with no way back except starting a new locker; sway shows red, and recovery is a TTY (swaypplet's documented red-failsafe path).
- A locker that cannot draw fast enough on hotplug leaks one frame of desktop (hyprlock issue #780).

## Lessons for swaypplet
- Keep a supervisor that relaunches the locker when logind's `LockedHint` is set and no lock surface exists; already the design, keep it tested.
- Cover newly added outputs within one frame; test the hotplug-while-locked path.

## Sources
- https://wayland.app/protocols/ext-session-lock-v1 — protocol [memory]
- https://github.com/hyprwm/hyprlock/issues/780 — screen visible on wake with another display [verified via search summary]
- https://wiki.hypr.land/hypr-ecosystem/user/hypridle/ — inhibit_sleep 3 waits for lock [verified via search summary]
