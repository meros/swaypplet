---
name: Windows 8 Start screen and Charms
slug: windows-8-start-screen
domain: shells
kind: product
platform: Windows
vendor: Microsoft
years: 2012–2015
status: failed
tags: [start-menu, touch, live-tiles, hot-corners, charms, failed-idea]
relevance: medium
---

## What it is
Windows 8 replaced the Start menu with a full-screen Start screen of Live Tiles and removed the Start button from the desktop taskbar. System commands (search, share, settings, devices, power) moved to the Charms bar, revealed by swiping from the right edge or pointing at a right-hand corner.

## What was new
- Live Tiles: app icons that show live content (unread count, weather, photos) in a grid.
- One shell for tablets and desktops, with edge swipes as the primary navigation.

## What went wrong / limits
- Mouse users lost a visible start point; hidden corners had no affordance, and many could not find how to shut down.
- Full-screen Start took the user out of their work to launch an app.
- Microsoft restored the Start button in 8.1 (2013) and the Start menu in Windows 10 (2015); Live Tiles were dropped in Windows 11.

## Lessons for swaypplet
- Avoid: commands reachable only through invisible edges or corners. BAR_VISION P8 says the same about hover.
- Avoid: tiles that update continuously (motion and change at rest).
- Take: keyboard users were fine (type to search at Start); keep search-first launch.

## Sources
- https://en.wikipedia.org/wiki/Windows_8 — Start screen, Charms, reception, 8.1 changes [memory]
- https://www.nngroup.com/articles/windows-8-disappointing-usability/ — Nielsen's usability study of Windows 8 [memory]
