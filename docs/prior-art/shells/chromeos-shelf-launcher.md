---
name: ChromeOS shelf and launcher
slug: chromeos-shelf-launcher
domain: shells
kind: product
platform: other
vendor: Google
years: 2011–present
status: current
tags: [shelf, launcher, search, everything-key, continue-section]
relevance: medium
---

## What it is
ChromeOS uses a bottom "shelf" (pinned and running apps, launcher button at the left, status area with clock, Wi-Fi and battery at the right). The status area opens Quick Settings with notifications above it. Since ChromeOS 100 (2022) the launcher is a compact side panel with search and a "Continue where you left off" row, opened with the Everything/Launcher key on the keyboard.

## What was new
- A dedicated keyboard key for the launcher, replacing Caps Lock.
- Launcher search across local files, Drive, apps, settings, browser tabs and history, with inline answers.
- "Continue" row: recent files and tabs as the first thing in the launcher.

## What went wrong / limits
- Earlier launcher (2015–2022) was a full-screen app grid with a Google search box at the top, slow for keyboard users; it was replaced.
- Shell depends heavily on network services and Google accounts; offline the launcher is thin.

## Lessons for swaypplet
- Take: open the launcher on a key tap and search immediately; ChromeOS's side panel moved away from full-screen grids for the same reason Windows 11 later did.
- Take: a small "continue" row from local recent files only, no network.

## Sources
- https://www.techradar.com/how-to/how-to-be-a-chromebook-poweruser-in-2022 — ChromeOS 100 launcher, continue section [verified]
- https://chromeunboxed.com/chrome-os-productivity-launcher-new-animation/ — productivity launcher [verified]
- https://en.wikipedia.org/wiki/ChromeOS — shelf, status area [memory]
