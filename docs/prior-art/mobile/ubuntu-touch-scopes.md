---
name: Ubuntu Touch scopes
slug: ubuntu-touch-scopes
domain: mobile
kind: feature
platform: other
vendor: Canonical
years: 2013–2018
status: failed
tags: [aggregation, content-first, search, home-screen]
relevance: low
---

## What it is
Instead of an app grid, Ubuntu Touch's home was a set of **scopes**: swipeable pages that aggregated content by type (Today, Music, Video, News, Nearby) from local and web sources, each with search. Apps were one scope among many. It grew out of the Unity desktop's Dash lenses, which sent local searches to Amazon by default (2012) and drew a privacy outcry.

## What was new
- Content by kind, not by app: "music" regardless of which service holds it.
- A search and card model that let partners ship a scope without shipping an app.

## What went wrong / limits
- Scopes depended on web APIs that changed or disappeared, so they broke and went empty.
- Users wanted apps; UBports replaced the scope home with a plain app drawer.
- The Dash's online results damaged trust in the idea (the Amazon lens was later made opt-in).

## Lessons for swaypplet
- **Avoid online aggregation in the shell**: network sources in a local surface fail and leak (the same reason the roadmap rejects weather and agenda).
- **Avoid replacing the app grid** with a content model the user did not ask for.

## Sources
- https://en.wikipedia.org/wiki/Ubuntu_Touch — scopes, later removal [verified via search summary]
- https://en.wikipedia.org/wiki/Unity_(user_interface) — Dash lenses, Amazon results controversy [memory]
