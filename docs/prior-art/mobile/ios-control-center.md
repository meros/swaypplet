---
name: iOS Control Center
slug: ios-control-center
domain: mobile
kind: feature
platform: iOS
vendor: Apple
years: 2013–present
status: current
tags: [quick-settings, toggles, customization, controls-api, pages, sliders]
relevance: high
---

## What it is
A system panel pulled from a screen edge (the bottom in iOS 7, the top-right corner since the iPhone X) holding radios, brightness, volume, media and shortcuts. iOS 11 made it one customizable page. iOS 18 rebuilt it on WidgetKit with third-party **Controls**, resizable tiles and several pages (favourites, media, Home, connectivity), with a long press or tap expanding a module in place.

## What was new
- **Module expands in place**: a long press on the connectivity block opens a bigger card with Wi-Fi, Bluetooth, AirDrop and hotspot, without navigating to a new screen.
- **Sliders as primary controls**: brightness and volume are tall direct-manipulation sliders you drag straight away, not a tap and then a drag.
- iOS 18: the Controls API is the same as widgets, so one control can appear in Control Center, on the lock screen and on the Action button.

## What went wrong / limits
- iOS 7–10 spread it across swiped pages, and people did not find the second page. iOS 11 reverted to one page; iOS 18 brought pages back and drew the same criticism.
- The Wi-Fi and Bluetooth toggles only *disconnect* until the next day and do not turn the radio off, which confused users and drew privacy criticism (iOS 11 onwards).
- The iOS 18 grid (4 wide) and page model made it harder to learn where a control lives.

## Lessons for swaypplet
- **Take expansion in place**: a panel tile that grows into its detail section, with no navigation, is the pattern that the rejected bar-to-panel morph does *not* rule out, since it stays inside one surface.
- **Avoid a toggle that means something else**: a toggle has to say what it does. iOS's "disconnect until tomorrow" is the counter-example.
- **Avoid hidden pages**: one scrollable panel is better than paged panels, unless the page indicator itself is labelled.

## Sources
- https://www.macstories.net/stories/ios-and-ipados-18-the-macstories-review/4/ — iOS 18 controls on WidgetKit, pages, resize [verified via search summary]
- https://support.apple.com/en-us/HT208086 — Wi-Fi/Bluetooth toggle disconnect behaviour [memory]
