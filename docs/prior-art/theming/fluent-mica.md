---
name: Fluent Mica
slug: fluent-mica
domain: theming
kind: feature
platform: Windows
vendor: Microsoft
years: 2021–present
status: current
tags: [material, wallpaper-tint, opaque, performance, fallback]
relevance: high
---

## What it is
Windows 11's base material for long-lived windows. It is almost opaque and tinted by the desktop wallpaper and theme, sampled once, not live. Mica Alt is a stronger-tinted variant for tabbed title bars.

## What was new
- A "personal" material without live blur cost: the wallpaper is sampled once and cached, so there is no per-frame backdrop read.
- Inactive windows fall back to a flat colour, which also signals focus.
- Solid fallback on battery saver, when transparency effects are off, or on low-end hardware.

## What went wrong / limits
- The tint is subtle enough that many users cannot tell it is on.
- Adoption by Win32 apps was slow; many shipped only after 22H2 exposed `DWMWA_SYSTEMBACKDROP_TYPE`.

## Lessons for swaypplet
- Take: "tinted by the wallpaper, not showing it" is a cheap mode for surfaces that do not need glass (settings page body, the greeter). swaypplet's `full` tint on neutrals is Mica's idea applied to tokens.
- Take the inactive fallback: a card that loses focus could drop to its solid key; worth testing for the launcher and settings.
- Take the battery fallback: glass off (or frost off) on battery saver.

## Sources
- https://learn.microsoft.com/en-us/windows/apps/design/style/mica — samples wallpaper once, fallback rules [verified]
- https://learn.microsoft.com/en-us/windows/apps/develop/ui/system-backdrops — backdrop API [verified via search summary]
