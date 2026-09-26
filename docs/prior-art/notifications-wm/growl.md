---
name: Growl
slug: growl
domain: notifications-wm
kind: project
platform: macOS
vendor: Growl project (Christopher Forsythe, lead)
years: 2004–2020
status: discontinued
tags: [third-party, notification-hub, themes, gntp, network]
relevance: low
---

## What it is
A third-party notification system for Mac OS X, started 2004 (first as "Global Notifications Center") because the OS had none. Apps registered notification types; the user chose display style per type from many themes, and could forward notifications over the network (GNTP). Retired 27 November 2020.

## What was new
Registered notification types per app, years before Android channels: the user could restyle or silence one type. A single hub that many apps (Adium, Colloquy) adopted voluntarily. Network forwarding between machines.

## What went wrong / limits
Apple shipped Notification Center in 2012 and developers moved to it. The move to a paid App Store app in 2011 annoyed users. The lead cited Apple Silicon and having no obvious way to improve it.

## Lessons for swaypplet
- Take: per-type registration and control is an old, proven idea; freedesktop `category` is the nearest equivalent.
- Note: a platform feature beats a third-party hub the day the platform ships one; swaypplet is the platform on this desktop, so it can do what Growl could not (context awareness).

## Sources
- https://take.surf/2020/11/30/chris-forsythe-growl-in-retirement — retirement and reasons. [verified via search summary]
- https://en.wikipedia.org/wiki/Growl_(software) — history, naming. [verified via search summary]
- Paid App Store move in 2011. [memory]
