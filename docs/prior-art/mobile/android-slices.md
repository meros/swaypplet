---
name: Android Slices
slug: android-slices
domain: mobile
kind: feature
platform: Android
vendor: Google
years: 2018–2021
status: failed
tags: [templates, app-content-in-system, search, deprecated]
relevance: low
---

## What it is
Announced at Google I/O 2018: an app publishes templated, interactive fragments of its UI ("slices": a ride estimate, a toggle, a list) that the system could show elsewhere, first in Google Search and Assistant results. Google paused Search integration soon after, and later marked the App Actions use deprecated.

## What was new
- Templates (rows, sliders, toggles) rendered by the host, so an app's content fits the host's look.
- Backwards compatible to Android 4.4 through Jetpack.

## What went wrong / limits
- The main host, Google Search, never shipped it widely; developers reported that indexed slices never appeared.
- Without a visible host, apps had no reason to build slices. Android 10's Settings panels were the one lasting use.

## Lessons for swaypplet
- **Avoid building a provider API before its surface**: build the surface that uses a type of content first, then generalise.
- Host-rendered templates are still right for any plugin idea: the source sends data, and the shell draws it with its own tokens.

## Sources
- https://medium.com/@hanru.yeh/the-feature-to-show-slices-in-google-search-app-has-been-paused-a581eec53c06 — Search integration paused [verified via search summary]
- https://developer.android.com/guide/slices — templates, Jetpack [verified via search summary]
