---
name: Predictive Back
slug: predictive-back
domain: mobile
kind: pattern
platform: Android
vendor: Google
years: 2022–present
status: current
tags: [gesture, preview, cancelable, motion, navigation, scrubbing]
relevance: medium
---

## What it is
Android's back gesture shows *where it will go* while the finger is still down: the current screen shrinks and slides, showing the home screen, the previous activity or the previous app behind it. Releasing commits; sliding back cancels. It was introduced as a developer option in Android 13–14 and on by default for supporting apps in Android 15 (`enableOnBackInvokedCallback`).

## What was new
- **Motion driven by the gesture**: animation progress follows the finger (`handleOnBackProgressed`), not a timer.
- **A cancelable preview**: the user sees the result before committing, which makes a destructive gesture safe.
- System-wide consistency: apps plug into one progress API instead of each drawing its own back animation.

## What went wrong / limits
- The rollout took about three Android releases, because apps with custom back handling broke and had to migrate.
- Apps that ignore it still jump abruptly, so the experience is uneven.

## Lessons for swaypplet
- **Adapt scrubbable transitions**: Super+Tab and the planned overview could follow a held key or touchpad swipe's progress and cancel on release back. swayfx's touchpad gestures give the progress.
- **Take preview before commit** for closing things: dismissing a notification stack or closing a workspace could show the result while the input is held.

## Sources
- https://developer.android.com/guide/navigation/custom-back/predictive-back-gesture — preview, Android 15 default, manifest flag [verified via search summary]
- https://developer.android.com/about/versions/14/features/predictive-back — progress callbacks [verified via search summary]
