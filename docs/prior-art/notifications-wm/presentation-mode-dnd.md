---
name: Presentation mode detection (cross-platform pattern)
slug: presentation-mode-dnd
domain: notifications-wm
kind: pattern
platform: cross
vendor: Microsoft, Apple, KDE and others
years: 2006–present
status: current
tags: [presentation, screen-share, mirroring, fullscreen, dnd, context]
relevance: high
---

## What it is
The pattern of silencing notifications, screensaver and sleep when the user is presenting. Windows Vista/7 had manual "presentation settings" in Mobility Center; Windows 10/11 turn on DND when duplicating a display; Plasma 5.17 when mirroring; macOS has "show notifications when mirroring or sharing the display" (off by default). Video-call apps (Zoom, Teams) hide their own notifications while sharing.

## What was new
The move from a manual presenter switch to inference from signals: output mirroring, fullscreen, active screen capture.

## What went wrong / limits
Each platform uses one proxy and misses the others: mirroring misses a Zoom share on an extended display; fullscreen misses a windowed slide deck being shared. macOS Tahoe's version spread DND to the user's iPhone and Watch through Focus sync, which users reported as a bug.

## Lessons for swaypplet
- Take: combine three signals (mirrored outputs, active screen capture from the privacy indicator, fullscreen on the focused output) with OR, and name which one triggered in the panel.
- Take: scope it to this machine; never propagate.
- Take: also inhibit OSD popups that would appear in a capture, not only notifications.

## Sources
- https://kde.org/announcements/plasma/5/5.17.0/ — DND when mirroring. [verified via search summary]
- https://www.elevenforum.com/t/manage-focus-assist-automatic-rules-in-windows-11.1363/ — duplicating display rule. [verified via search summary]
- https://discussions.apple.com/thread/256207562 — macOS mirroring DND and cross-device spread. [verified via search summary]
- Vista/7 presentation settings in Mobility Center. [memory]
