---
name: Unity Shopping Lens (Amazon results in the Dash)
slug: unity-shopping-lens
domain: notifications-wm
kind: feature
platform: other
vendor: Canonical (Ubuntu)
years: 2012–2016
status: failed
tags: [search, privacy, ads, launcher, trust]
relevance: high
---

## What it is
Ubuntu 12.10 (2012) sent searches typed into the Unity Dash, the launcher and file search, to Canonical's servers and showed Amazon product results beside local apps and files, on by default. The EFF asked for opt-in; Richard Stallman called it spyware; it won an Austrian Big Brother Award in 2013. Ubuntu 16.04 made online search off by default.

## What was new
Nothing useful; revenue through the launcher.

## What went wrong / limits
A local, private action (searching your own files) became a network request without consent. It poisoned trust in the Dash for years and fed the "Ubuntu spyware" reputation.

## Lessons for swaypplet
- Avoid: any network call from the launcher or a shell search without an explicit, per-provider opt-in.
- Take: say in the UI when a result comes from the network.

## Sources
- https://www.eff.org/deeplinks/2012/10/privacy-ubuntu-1210-amazon-ads-and-data-leaks — EFF concerns. [verified via search summary]
- https://www.omgubuntu.co.uk/2013/10/ubuntu-wins-big-brother-austria-privacy-award/amp — award. [verified via search summary]
- https://www.omgubuntu.co.uk/2016/01/ubuntu-online-search-feature-disabled-16-04 — off by default in 16.04. [verified via search summary]
