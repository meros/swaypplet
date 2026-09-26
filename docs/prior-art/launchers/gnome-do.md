---
name: GNOME Do
slug: gnome-do
domain: launchers
kind: product
platform: GNOME
vendor: David Siegel and contributors
years: 2007–c. 2013
status: failed
tags: [mono, quicksilver clone, docky, abandoned]
relevance: low
---

## What it is
A Quicksilver clone for GNOME written in C#/Mono, with plugins for Pidgin, Tomboy, Twitter and more. Its "Docky" theme became the standalone Docky dock.

## What was new
The first popular Linux launcher with Quicksilver's object-action model and a plugin catalogue.

## What went wrong / limits
- Built on Mono just as Linux distributions dropped Mono from default installs; plugins depended on apps (Tomboy, Pidgin) that faded.
- Maintainers moved to Docky, then Docky itself stalled.

## Lessons for swaypplet
- Avoid tying launcher features to specific third-party apps; route through standard sources (desktop files, D-Bus, MPRIS).

## Sources
- https://en.wikipedia.org/wiki/GNOME_Do — history, Docky [memory]
