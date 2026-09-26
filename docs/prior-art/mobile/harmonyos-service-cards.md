---
name: HarmonyOS service widgets and atomic services
slug: harmonyos-service-cards
domain: mobile
kind: feature
platform: other
vendor: Huawei
years: 2021–present
status: current
tags: [widgets, swipe-up-icon, install-free, cards, service-center]
relevance: medium
---

## What it is
In HarmonyOS 2 (2021), swiping up on an app icon opens a **service widget**: a card showing the app's key information and actions, which can be pinned to the home screen. A swipe up from the bottom corner opens the **Service Center**, holding all collected cards. **Atomic services** are install-free mini apps delivered as cards.

## What was new
- **The icon unfolds into its widget**: a gesture on the icon itself, with no separate widget gallery.
- Cards in several sizes from one declaration, and a direct way from card to app.
- Install-free services as a distribution model (similar to WeChat mini programs and Android Instant Apps).

## What went wrong / limits
- Outside China the ecosystem stayed small after US sanctions cut Google services.
- Swipe-up on an icon is hidden; it clashes with the swipe-home gesture near the dock.

## Lessons for swaypplet
- **Adapt "the entry point unfolds"**: a secondary action on a bar segment could expand its compact detail card in place, a single object in two sizes, rather than opening a separate popover.

## Sources
- https://www.huaweicentral.com/harmonyos-features/ — swipe-up widgets, Service Center [verified via search summary]
- https://medium.com/@shikkerimath/working-with-atomic-widget-services-in-harmonyos-991b29f05ae8 — atomic services [verified via search summary]
