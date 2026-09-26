---
name: Alt-Tab and Cmd-Tab switcher designs
slug: alt-tab-switchers
domain: notifications-wm
kind: pattern
platform: cross
vendor: Microsoft, Apple, GNOME, KDE
years: 1990–present
status: current
tags: [alt-tab, mru, thumbnails, app-vs-window, keyboard]
relevance: high
---

## What it is
The hold-modifier-and-tap switcher. Windows 3.0 (1990) had Alt-Tab with window titles; Windows Vista (2007) showed live DWM thumbnails; macOS Cmd-Tab switches apps, not windows (Cmd-` cycles an app's windows); GNOME 3 groups by app with Alt-` for windows; KWin offers many switcher layouts.

## What was new
Most-recently-used order makes the most common switch (back to the previous thing) one tap. Live thumbnails (Vista) let users recognise rather than read. Hold-to-browse, release-to-commit is one of the best-learned interactions in computing.

## What went wrong / limits
App-vs-window disagreement: Cmd-Tab to an app with a minimised window shows nothing, a long-standing complaint. Thumbnails too small with many windows; MRU order means positions move, so users cannot learn where a thing is. Windows 10 mixed Edge tabs into Alt-Tab (2020), which many turned off.

## Lessons for swaypplet
- Take: Super+Tab's tap-to-previous (MRU for the first step) with stable spatial order for browsing is the combination that avoids both failure modes; state which order applies.
- Take: release commits, Escape cancels, no pointer needed.
- Avoid: mixing other kinds of items (browser tabs, files) into the switcher.

## Sources
- https://devblogs.microsoft.com/oldnewthing/20080701-00/?p=21793 — Vista Alt-Tab order change. [verified via search summary]
- https://learn.microsoft.com/en-us/windows/win32/dwm/thumbnail-ovw — DWM thumbnails. [verified via search summary]
- Windows 3.0 Alt-Tab, macOS Cmd-Tab behaviour, Edge tabs in Alt-Tab. [memory]
