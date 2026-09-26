---
name: macOS Dashboard
slug: macos-dashboard
domain: notifications-wm
kind: feature
platform: macOS
vendor: Apple
years: 2005–2019
status: discontinued
tags: [widgets, overlay, html, konfabulator]
relevance: medium
---

## What it is
A full-screen overlay of small HTML/CSS/JavaScript widgets (weather, stocks, calculator, dictionary) summoned by a key in Mac OS X 10.4 Tiger (2005). Lion folded it into Mission Control as a Space. Disabled by default in Yosemite (2014), removed in Catalina (2019).

## What was new
A separate layer for glanceable mini-apps, out of the window stack, summoned and dismissed with one key. Widgets written with web tech, so thousands appeared quickly. Widely compared with Konfabulator (2003), the third-party engine it resembled.

## What went wrong / limits
A layer you must summon is a layer you forget; usage faded as the phone took over weather and stocks. Widgets were out of date between summons. It stayed on 32-bit WebKit plumbing and died with 32-bit support.

## Lessons for swaypplet
- Avoid: a summoned "glance layer" of information; if information is worth a glance it belongs in the panel section already opened for other reasons.
- Note: supports the roadmap's rejection of weather/agenda surfaces opened twice a day.

## Sources
- https://en.wikipedia.org/wiki/Dashboard_(macOS) — dates, 32-bit, Konfabulator comparison. [verified via search summary]
- https://www.imore.com/mac/macos/a-history-of-widgets-on-the-mac-from-dashboard-to-desktop — history to Sonoma. [verified via search summary]
