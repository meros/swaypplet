---
name: Auto Dark Mode for Windows
slug: windows-auto-dark-mode
domain: theming
kind: project
platform: Windows
vendor: Armin Osaj and Samuel Schiegg (AutoDarkMode)
years: 2018–present
status: current
tags: [dark-mode, schedule, sunset, wallpaper-switch, idle, open-source]
relevance: high
---

## What it is
An open-source app filling Windows' missing scheduled dark mode. It switches apps and system theme at set times or sunset/sunrise (with offsets), and can also switch wallpaper, accent colour, cursor, Windows theme files and Office theme on the same schedule. Installed via Store or `winget`.

## What was new
- Switches a bundle, not a flag: wallpaper, accent, cursors, and themes move together.
- Postponement rules: waits while a game or full-screen app runs, can wait for system idle, and supports manual pause "until next switch".
- Hotkeys and a tray toggle that respect the schedule afterwards.

## What went wrong / limits
- Windows has no native schedule, so behaviour depends on a background service; some apps (Explorer, Office) only partly follow at runtime.
- The switch is visible (Explorer repaints) when postponement is off.

## Lessons for swaypplet
- Take "pause until next switch": a manual dark/light choice in the panel could hold until the next sunset/sunrise and then return to auto, instead of staying until the user picks auto again (§2.1). Many users forget to switch back.
- Take the bundle: switch wallpaper with the mode when the user has set a pair.

## Sources
- https://github.com/AutoDarkMode/Windows-Auto-Night-Mode — features [verified via search summary]
- https://pureinfotech.com/switch-light-dark-mode-automatically-windows-11/ — wallpaper, accent, cursor switching [verified via search summary]
- Postponement while gaming and idle checker [memory]
