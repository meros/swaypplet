---
name: wluma
slug: wluma
domain: wayland
kind: product
platform: Wayland
vendor: Maximilian Bazhenov (max-baz)
years: 2021–present
status: current
tags: [brightness, ambient-light, screen-contents, learning, screencopy]
relevance: medium
---

## What it is
A daemon that sets backlight brightness from the ambient light sensor and the luminance of what is on screen, which it samples through screencopy (Vulkan). Its default "adaptive" algorithm learns from the user: every manual brightness change is recorded as a data point for the current light level and screen luminance, and after a few corrections it predicts the user's choice.

## What was new
- Screen content as an input: a fullscreen dark terminal brightens the backlight, a white web page dims it.
- Learning from corrections instead of a curve the user must tune; no settings UI needed.

## What went wrong / limits
- Needs several manual corrections in varied conditions before it acts; early behaviour looks random.
- Continuous screen capture has a GPU cost; shipped configs are hardware-specific (a distro issue notes one shipped config defeats auto-detection).

## Lessons for swaypplet
- Take "every manual change is a training point" for auto-brightness and for night-light strength: the OSD brightness keys already give the signal.
- Sample luminance at low rate from the existing capture path rather than a second capture client.

## Sources
- https://github.com/max-baz/wluma — algorithm, ALS and screen contents [verified via search summary]
- https://github.com/ashlaros/ashlaros/issues/41 — shipped config defeats auto-detection [verified via search summary]
