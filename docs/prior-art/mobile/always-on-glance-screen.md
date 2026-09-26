---
name: Always-on displays (Nokia Glance Screen to AOD)
slug: always-on-glance-screen
domain: mobile
kind: pattern
platform: cross
vendor: Nokia, Motorola, Samsung, Google, Apple
years: 2012–present
status: current
tags: [ambient, low-power, clock, notifications, glanceable, oled]
relevance: medium
---

## What it is
A low-power view shown while the phone is locked and "off": the time, the date and icons for pending notifications, drawn with few lit pixels. Nokia's Glance Screen (Nokia 808, then Lumia with the Amber update, 2013) made it mainstream, and it even ran on LCD Lumias with a dim backlight. Motorola's Active Display (Moto X, 2013) lit it on movement. It is now standard on Android (AOD) and on iPhone 14 Pro and later.

## What was new
- **Information without a wake**: you learn whether anything needs you without unlocking.
- **Three policies** (off, timed for 15 minutes after lock, always), which set expectations about battery.
- Motorola's version showed pending notifications briefly on pickup, then went dark again.

## What went wrong / limits
- Burn-in and battery worries lead to pixel-shifting and dimming rules.
- Notification icons alone say little; people still unlock to check.

## Lessons for swaypplet
- **Take "is anything waiting?" at rest**: the lock screen's summary row (roadmap item 3) should answer that with a count and a kind, not previews, for privacy.
- The Glance principle, few lit pixels and near-still content, is P1/P2 for a surface.

## Sources
- https://allaboutwindowsphone.com/features/item/17736_Nokia_Glance_Screen_and_displa.php — Glance Screen options and power [verified via search summary]
- https://blogs.windows.com/devices/2014/01/29/nokia-glance-screen-2-0/ — Glance 2.0 [verified via search summary]
- https://en.wikipedia.org/wiki/Moto_X_(1st_generation) — Active Display [memory]
