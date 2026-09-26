---
name: webOS Just Type
slug: webos-just-type
domain: mobile
kind: feature
platform: other
vendor: Palm, HP
years: 2009–2012
status: discontinued
tags: [launcher, search, actions, type-to-act, extensible]
relevance: high
---

## What it is
In webOS, typing on the hardware keyboard from the home screen or card view started a search at once, with no search box to open first. webOS 2.0 (2010) renamed it Just Type and added **Quick Actions**: the typed text became the start of an action, such as a new email, a new memo, a status update or a message, and apps could register their own actions.

## What was new
- **Type first, choose the verb after**: the text exists before you decide which app it belongs to.
- **Apps extend the launcher with verbs**, not only with search results.
- No mode switch: the keyboard in the resting state *is* the launcher.

## What went wrong / limits
- It needed a hardware keyboard to shine; the TouchPad's on-screen keyboard made it a two-step action again.
- The platform ended in 2011, so the extension ecosystem stayed small.

## Lessons for swaypplet
- **Take "text first, verb second"**: the launcher could offer actions on typed text (new note, search the web, type as emoji, run as command) below app results, and roadmap item 5's picker could live there.
- **Adapt verb registration**: freedesktop `.desktop` `Actions=` and the D-Bus search providers GNOME uses are an existing way for apps to add results.

## Sources
- https://en.wikipedia.org/wiki/WebOS — Just Type in webOS 2.0 [memory]
- https://www.howtogeek.com/finally-tried-webos-inspired-android-still-might-be-better/ — webOS interaction overview [verified via search summary]
