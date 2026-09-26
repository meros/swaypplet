---
name: Windows Quick Settings and Action Center
slug: windows-quick-settings
domain: shells
kind: feature
platform: Windows
vendor: Microsoft
years: 2015–present
status: current
tags: [quick-settings, notifications, action-center, toggles]
relevance: medium
---

## What it is
Windows 10's Action Center (2015) was one right-edge sheet with notifications above and quick-action tiles below. Windows 11 split it: Quick Settings (network, volume, battery icons open a card of toggles and sliders) and the notification centre with a calendar (clock opens it). Tiles open a sub-page for Wi-Fi or Bluetooth device lists inside the card.

## What was new
- Windows 11: the status icons are one click target that opens the controls they represent; the clock opens time-related content.
- Editable tile set, with sub-pages for lists (Wi-Fi networks, audio outputs).

## What went wrong / limits
- Windows 11 at launch removed the ability to open a network's properties from the panel, and needed extra clicks for basic actions like switching audio output (later improved).
- Two separate panels for status and notifications confused users coming from Windows 10.

## Lessons for swaypplet
- Take: the status icon group is the handle for the control card; the bar shows state, the card acts.
- Take: sub-pages for device lists inside the same card, not a separate window.

## Sources
- https://en.wikipedia.org/wiki/Action_Center — Windows 10 Action Center [memory]
- https://en.wikipedia.org/wiki/Features_new_to_Windows_11 — Quick Settings split [memory]
