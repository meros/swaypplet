---
name: tofi
slug: tofi
domain: launchers
kind: product
platform: Wayland
vendor: Phil Jones (philj56)
years: 2022–present
status: current
tags: [latency, startup, harfbuzz, dmenu, benchmark]
relevance: medium
---

## What it is
An "extremely fast" dmenu/rofi replacement for wlroots compositors whose README documents its startup time and the tricks behind it.

## What was new
- **Measured latency budget**: 2.3–3.6 ms to first frame for the dmenu theme, 6.9 ms fullscreen on a Ryzen 7 3700X.
- Font path given directly to HarfBuzz/Cairo skips Fontconfig (about 120 ms).
- Hinting off saves 3–5 ms; keyboard init deferred until after the first frame saves 1–2 ms (60 ms on a Raspberry Pi); a smaller surface renders faster (1 ms vs 20 ms fullscreen on battery).

## What went wrong / limits
- The speed comes from doing little: no icons, no async sources, no toolkit.

## Lessons for swaypplet
- Take the method: measure open-to-first-frame and keystroke-to-frame, publish the numbers in LAUNCHER.md, and defer everything that is not the first frame.
- Font lookup is a real cost for a spawned process; resident GTK sidesteps it.

## Sources
- https://github.com/philj56/tofi — latency numbers and techniques [verified]
