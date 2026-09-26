---
name: ProMotion adaptive refresh
slug: ios-promotion-adaptive-refresh
domain: motion
kind: feature
platform: iOS
vendor: Apple
years: 2017–present
status: current
tags: [120hz, variable-refresh, CADisplayLink, frame-rate-range, power]
relevance: medium
---

## What it is
ProMotion displays (iPad Pro 2017, iPhone 13 Pro 2021) vary refresh between about 10 and 120 Hz. Apps state a `preferredFrameRateRange` (`CAFrameRateRange` minimum, maximum, preferred) on `CADisplayLink` or a Core Animation animation, and the system picks the real rate.

## What was new
Frame rate became a per-animation request instead of a global constant. Fast, large motion asks for 120; a slow fade can ask for 30–60 and save power. System animations run at 120, and on iPhone third-party apps must opt in (`CADisableMinimumFrameDurationOnPhone`) before Core Animation goes above 60.

## What went wrong / limits
- At launch iPhone 13 Pro capped third-party animations at 60 Hz without the opt-in key, and developers read it as a bug.
- Mixed rates in one scene pace badly: 24 fps content on a 120 Hz panel is fine, 50 fps is not.
- Low Power Mode caps at 60 Hz, and users on ProMotion report it as feeling worse than a native 60 Hz phone.

## Lessons for swaypplet
- Adapt: an animation's frame budget is 1/refresh, not 16.7 ms. The frame gate's fixed 25 ms "late" threshold is 1.5 frames at 60 Hz but 3 frames at 120 Hz; make it refresh-relative.
- Take: slow ambient loops (2 s breathing, 0.8 s pulse) need far fewer than 60 fps; a lower frame rate for them is a direct power saving.

## Sources
- https://developer.apple.com/documentation/quartzcore/optimizing-iphone-and-ipad-apps-to-support-promotion-displays — frame rate ranges, opt-in key [verified, summary only]
- https://www.phonearena.com/news/iphone-13-pro-and-pro-max-limit-third-party-apps-to-60Hz_id135299 — 60 Hz cap for third-party apps at launch [memory]
