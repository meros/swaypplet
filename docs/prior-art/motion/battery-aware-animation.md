---
name: Battery-aware animation (Low Power Mode, Battery Saver)
slug: battery-aware-animation
domain: motion
kind: pattern
platform: cross
vendor: Apple, Microsoft, Google
years: 2015–present
status: current
tags: [power, low-power-mode, battery-saver, transparency, fallback, refresh-cap]
relevance: high
---

## What it is
All three major platforms reduce visual cost on low battery. iOS Low Power Mode caps ProMotion at 60 Hz and trims some motion and blur. Windows Battery Saver turns Acrylic into its solid fallback colour. Android Battery Saver can scale down animations, and apps read the result via `ValueAnimator.areAnimatorsEnabled()`.

## What was new
The expensive material has a defined cheap twin (Acrylic's `FallbackColor`, Mica's solid fallback) chosen so the layout and contrast do not change when the effect switches off. Apps get the fallback automatically; they do not implement power policy themselves.

## What went wrong / limits
- The switch is abrupt and global; users notice a panel going flat at 20 % battery.
- iOS users report 60 Hz Low Power Mode as laggier than 60 Hz on a non-ProMotion phone, likely because animations tuned for 120 Hz lose more.

## Lessons for swaypplet
- Take: define a solid-fill twin for each glass namespace whose text passes the same APCA tests, then let the compositor switch to it on battery or a power-profile change (power-profiles-daemon `power-saver`).
- Take: on battery, freeze ambient loops and lower their frame rate first; transitions are short and matter more.
- Avoid: an abrupt swap; cross-fade the material over a `state` (150 ms) step.

## Sources
- https://learn.microsoft.com/en-us/windows/apps/design/style/acrylic — Battery Saver disables Acrylic, solid fallback [verified via search summary]
- https://learn.microsoft.com/en-us/windows/apps/design/style/mica — solid fallback on battery saver [verified, see research/adaptive-glass.md]
- https://www.macobserver.com/news/iphone-low-power-mode-feels-laggy-after-recent-ios-updates/ — 60 Hz cap perceived as laggy [memory]
- https://developer.android.com/reference/android/animation/ValueAnimator#areAnimatorsEnabled() — app-visible animator state [verified, partial]
