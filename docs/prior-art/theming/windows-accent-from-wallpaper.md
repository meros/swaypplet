---
name: Windows automatic accent colour from background
slug: windows-accent-from-wallpaper
domain: theming
kind: feature
platform: Windows
vendor: Microsoft
years: 2015–present
status: current
tags: [accent, wallpaper, extraction, slideshow]
relevance: medium
---

## What it is
Windows 10 and 11 option "Automatically pick an accent colour from my background" (Windows 11: Accent colour → Automatic). The system picks a dominant colour from the wallpaper and uses it for Start, taskbar, title bars and window borders; it updates on every wallpaper change, including slideshows.

## What was new
- Mainstream desktop feature six years before Material You; one checkbox, no configuration.
- Accent applies to chrome only; content colours are untouched.

## What went wrong / limits
- The pick takes lightness as well as hue, so dark wallpapers yield muddy, near-black accents and light ones washed-out accents; users routinely switch it off.
- No contrast guarantee for text on the accent; Windows 11 adjusts accent shades (Light1–3, Dark1–3) but the base can still be poor.

## Lessons for swaypplet
- Confirms swaypplet's choice of taking hue only from the image and lightness from the role.
- Slideshow wallpapers should re-sample on change; check `wallpaper-source` cache invalidation covers a changed file at the same path.

## Sources
- https://support.microsoft.com/en-us/windows/personalize-your-colors-in-windows-3290d30f-d064-5cfe-6470-2fe9c6533e37 — the option [verified via search summary]
- https://pureinfotech.com/change-accent-color-windows-10/ — updates on wallpaper change [verified via search summary]
- Muddy-accent complaints and accent shade ramp [memory]
