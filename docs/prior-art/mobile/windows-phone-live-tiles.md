---
name: Windows Phone Live Tiles
slug: windows-phone-live-tiles
domain: mobile
kind: feature
platform: other
vendor: Microsoft
years: 2010–2021
status: failed
tags: [tiles, glanceable, animation, start-screen, ambient-motion]
relevance: high
---

## What it is
The start screen of Windows Phone 7 (2010) was a grid of resizable tiles. A **live tile** flipped or cycled content from its app: unread counts, a photo, the next meeting, the weather. Windows 8 (2012) brought them to the PC start screen, Windows 10 to the start menu. Windows 11 (2021) removed them.

## What was new
- **The launcher is also the dashboard**: the icon shows state, so there is less need to open the app.
- Tiles pinned at three sizes; a strong typographic and flat look (see [metro-design-language](metro-design-language.md)).

## What went wrong / limits
- **Constant motion**: flipping tiles made a start screen that never sat still, and a count on a tile turned into a nag.
- On the desktop they made little sense: few people kept the start menu open long enough to glance at it, and Microsoft's telemetry showed low use.
- Tiles often went stale or stopped updating, a known bug class.
- Windows Phone itself failed for lack of apps, not because of tiles; the tiles did not save it.

## Lessons for swaypplet
- **Avoid animated state in the resting surface**: flipping tiles are the anti-pattern BAR_VISION P1/P2 rule out. State should be still, and change only on a transition.
- **Avoid dashboards in a place that is open for seconds**: the same reason weather and agenda were rejected (P7).
- The one lasting part is typographic hierarchy over icons.

## Sources
- https://www.windowscentral.com/live-tiles-are-not-cause-windows-phones-woes — tiles were not the cause of failure [verified via search summary]
- https://news.softpedia.com/news/three-reasons-live-tiles-are-no-longer-used-in-windows-10-529362.shtml — low use on desktop [verified via search summary]
- https://www.inverse.com/input/tech/report-windows-10-live-tiles-are-being-killed-off — removal [verified via search summary]
