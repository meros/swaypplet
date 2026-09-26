---
name: Android Frame Pacing library (Swappy)
slug: android-frame-pacing-swappy
domain: motion
kind: project
platform: Android
vendor: Google
years: 2019–present
status: current
tags: [frame-pacing, presentation-time, buffer-stuffing, swap-interval]
relevance: medium
---

## What it is
A library for games that paces presentation to the display. It uses presentation timestamps (`EGL_ANDROID_presentation_time`, `VK_GOOGLE_display_timing`) and fences so frames present at even intervals.

## What was new
It named the problem that average fps hides: 30 fps on a 60 Hz panel can present at 49, 16, 33 ms intervals, which stutters although the average is right. It picks a swap interval the device can sustain (45 fps on a 90 Hz panel rather than a ragged 60) and stops the app from stuffing the queue.

## What went wrong / limits
Games only; UI toolkits pace through the platform instead.

## Lessons for swaypplet
- Take: frame-bench should report interval *variance* (or the count of intervals not equal to one refresh), since an even 30 fps looks better than an uneven 50.
- Take: when glass cannot hold full rate on a low-end GPU, a steady half rate is the lesser failure.

## Sources
- https://developer.android.com/games/sdk/frame-pacing — pacing problem, stuffing, presentation timestamps [verified]
