---
name: Android dark theme schedule and bedtime mode
slug: android-dark-theme-schedule
domain: theming
kind: feature
platform: Android
vendor: Google
years: 2019–present
status: current
tags: [dark-mode, schedule, bedtime, force-dark]
relevance: medium
---

## What it is
Android 10 (2019) added a system dark theme; Android 11 added a schedule (sunset to sunrise, or custom times), and Pixel added "turn on at bedtime" linked to Digital Wellbeing. `forceDarkAllowed` let the system auto-darken apps without a dark theme.

## What was new
- Bedtime as a schedule source: the theme follows the user's routine, not the sun.
- Force dark: an algorithmic dark rendering for unprepared apps (Android 10); later "extra dim" as a separate brightness reduction.

## What went wrong / limits
- Force dark produced odd results on images and brand colours; developers opted out.
- Schedules are opaque about thresholds; there is no deferral, so the flip happens mid-use.

## Lessons for swaypplet
- A third auto source, "follow the night window" (swaypplet already has an idle night window), is a small addition that matches routine rather than sun.
- Avoid force-dark style transforms of other apps' content.

## Sources
- https://developer.android.com/develop/ui/views/theming/darktheme — dark theme and force dark [memory]
- Android 11 schedule, Pixel bedtime option [memory]
