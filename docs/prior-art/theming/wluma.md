---
name: wluma
slug: wluma
domain: theming
kind: project
platform: Wayland
vendor: Maxim Baz
years: 2021–present
status: niche
tags: [brightness, ambient-light, screen-content, learning, wayland]
relevance: medium
---

## What it is
A Wayland daemon that sets backlight brightness from ambient light (ALS sensor, webcam, or time of day) and the luminance of what is on screen, captured via wlroots screencopy or export-dmabuf. Its default "adaptive" algorithm learns from the user's manual adjustments.

## What was new
- Screen content as an input: a white page dims the backlight, a dark terminal brightens it, reducing the flash when switching.
- Learning from corrections rather than a fixed curve: does nothing on first launch, then imitates.

## What went wrong / limits
- Cold start: needs several manual corrections in different conditions before it helps.
- Screen capture every few hundred ms costs power; configuration is per device (the shipped config of one distro hard-coded one ThinkPad).

## Lessons for swaypplet
- The same backdrop luminance swaypplet's photochromic term reacts to could drive a gentle brightness correction, but that belongs in the gamma/backlight service, not the tokens.
- Take "learn from corrections" for auto mode: if the user flips to light every morning before +3°, suggest a lower threshold.

## Sources
- https://github.com/max-baz/wluma — adaptive algorithm, ALS, screen contents [verified via search summary]
- https://github.com/ashlaros/ashlaros/issues/41 — per-device config problem [verified via search summary]
