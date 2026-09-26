---
name: KDE 4.0 launch
slug: kde-4-0-launch
domain: notifications-wm
kind: product
platform: KDE
vendor: KDE
years: 2008
status: failed
tags: [release, rewrite, plasma, expectations, communication]
relevance: medium
---

## What it is
KDE 4.0 (January 2008) was a rewrite on Qt 4 with the new Plasma shell. The developers considered it a developer release for a platform; distributions shipped it as the new default desktop. Missing features, crashes and a changed desktop metaphor produced what Linux.com called an unprecedented user revolt; some developers quit under the abuse. By 4.2 to 4.4 it was broadly accepted.

## What was new
Plasma's widget-based shell and the architecture that Plasma 5 and 6 still use.

## What went wrong / limits
Version number and messaging did not match the state: "4.0" read as ready. Feature parity with 3.5 was not a release criterion. Distributions had no way to know it was not meant for users.

## Lessons for swaypplet
- Take: never ship a rewrite of a surface the owner uses daily until it has parity with what it replaces; run the new one on a branch (as swaypplet does with `theme-anim`, `launcher`).
- Take: label unfinished work where people meet it.

## Sources
- https://www.linux.com/news/what-went-wrong-kde-4-release/ — the revolt, developer release intent. [verified via search summary]
- https://www.osnews.com/story/19113/aaron-seigo-on-kde-40/ — Seigo's explanation. [verified via search summary]
- https://en.wikipedia.org/wiki/KDE_Software_Compilation_4 — release history. [verified via search summary]
