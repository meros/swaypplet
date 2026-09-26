---
name: Idle handling (ext-idle-notify, idle-inhibit, swayidle, hypridle)
slug: idle-stack
domain: wayland
kind: pattern
platform: Wayland
vendor: wayland-protocols; swaywm; hyprwm
years: 2018–present
status: current
tags: [protocol, idle, inhibit, logind, lock, suspend]
relevance: medium
---

## What it is
`ext-idle-notify-v1` tells a client when the seat has been idle for N ms and when it resumes; v2 adds a notification that ignores inhibitors. `zwp_idle_inhibit_manager_v1` lets a client with a visible surface block idle (video players). swayidle runs commands on timeouts and on logind `before-sleep`; hypridle adds D-Bus inhibit support (Firefox, Steam), `ignore_dbus_inhibit`, and `inhibit_sleep` levels that hold suspend until the lock surface is up. swaypplet handles idle in-process with logind.

## What was new
- hypridle `inhibit_sleep = 3`: sleep waits for the session lock's `locked` event, closing the "desktop visible on resume" gap.
- Honouring the org.freedesktop.ScreenSaver D-Bus Inhibit, which browsers use, instead of only the Wayland protocol.

## What went wrong / limits
- Idle-inhibit is surface-bound: a background audio player cannot inhibit, and a hidden video window stops inhibiting.
- Three inhibit channels (Wayland, ScreenSaver D-Bus, logind) that tools honour inconsistently.

## Lessons for swaypplet
- Make sure all three inhibit sources feed one state and that the panel shows who inhibits (hypridle shows none; this is a gap to fill).
- Use ext-idle-notify v2 for "user really idle" decisions (dim) and v1 with inhibitors for lock.

## Sources
- https://wiki.hypr.land/hypr-ecosystem/user/hypridle/ — dbus inhibit, inhibit_sleep [verified via search summary]
- https://github.com/hyprwm/hypridle/issues/39 — per-listener inhibit ignore [verified via search summary]
- https://wayland.app/protocols/ext-idle-notify-v1 — protocol [memory]
