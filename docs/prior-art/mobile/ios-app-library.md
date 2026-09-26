---
name: iOS App Library
slug: ios-app-library
domain: mobile
kind: feature
platform: iOS
vendor: Apple
years: 2020–present
status: current
tags: [launcher, automatic-organization, categories, search, suggestions]
relevance: medium
---

## What it is
Since iOS 14, the page after the last home screen lists every app in automatic category folders (Social, Productivity, …), with *Suggestions* and *Recently Added* at the top and search at the top edge. Home-screen pages can be hidden, so the Library becomes the full app list.

## What was new
- **No-maintenance organisation**: the system categorises from App Store data. The user curates only the home screen.
- The top two groups are dynamic (suggested, recent); the rest is stable.
- Search first: pulling down turns the grid into an alphabetical list.

## What went wrong / limits
- Categories cannot be edited or renamed, and apps land in odd folders.
- Folder previews show only three icons plus a cluster, so the fourth app takes a second tap.

## Lessons for swaypplet
- **Take "dynamic groups on top, stable below"** for the launcher: recent and suggested first, then a stable list, so the dynamic part never shifts what is under the user's muscle memory.
- **Adapt categories**: `.desktop` `Categories=` gives the freedesktop grouping for free, but allow an override, the thing Apple's version lacks.

## Sources
- https://www.macrumors.com/guide/home-screen/ — App Library behaviour [verified via search summary]
- https://appleinsider.com/articles/20/06/24/in-depth-with-widgets-app-library-more-on-ios-14-home-screen — suggestions and recently added [verified via search summary]
