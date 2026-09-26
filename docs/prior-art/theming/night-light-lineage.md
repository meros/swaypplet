---
name: f.lux, Redshift, Night Shift and GNOME Night Light
slug: night-light-lineage
domain: theming
kind: pattern
platform: cross
vendor: Herf (f.lux); Jon Lund Steffensen (Redshift); Apple; GNOME; Microsoft
years: 2009–present
status: current
tags: [colour-temperature, sun, gamma, schedule, blue-light]
relevance: high
---

## What it is
Software that warms the display after sunset. f.lux (2009) started it; Redshift 0.1 (November 2009) was the open-source equivalent; Apple shipped Night Shift in iOS 9.3 (March 2016) and macOS 10.12.4; GNOME Night Light arrived in 3.24 (2017); Windows Night Light in the Creators Update (2017); gammastep and wlsunset carried Redshift to wlroots.

## What was new
- f.lux: location-based sun schedule with slow transitions (60 minutes or 20 seconds), lighting presets (halogen, candle), and a "bedtime" mode that goes warmer later.
- Redshift/gammastep: elevation-driven ramp between day and night temperature through twilight, not a switch at sunset.
- OS versions: one toggle plus a warmth slider, integrated with the auto dark schedule.

## What went wrong / limits
- Evidence for sleep benefit from screen warming alone is weak; the lasting value is comfort.
- Gamma-ramp tricks conflict with colour-managed/HDR pipelines and with screenshots or colour-critical work (f.lux added per-app disable).
- On Wayland, gamma control is a privileged protocol; multiple tools fight over it.

## Lessons for swaypplet
- swaypplet's ramp in mired from +3° to −6° elevation is Redshift's best idea done correctly. Take f.lux's per-app/fullscreen exceptions (disable for an image editor or a video) as a later addition.
- Take f.lux's "later is warmer" as an optional second stage after bedtime or the night window.

## Sources
- https://en.wikipedia.org/wiki/F.lux — f.lux history [verified via search summary]
- https://en.wikipedia.org/wiki/Redshift_(software) — Redshift 0.1 date [verified via search summary]
- https://www.macrumors.com/how-to/use-ios-9-3-night-shift-mode/ — Night Shift [verified via search summary]
- https://www.omgubuntu.co.uk/2017/02/gnome-night-light-blue-light-filter-linux — GNOME 3.24 [verified via search summary]
