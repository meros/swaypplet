---
name: Snarl
slug: snarl
domain: notifications-wm
kind: project
platform: Windows
vendor: full phat products
years: 2005–present
status: niche
tags: [third-party, growl-clone, gntp, network, windows]
relevance: low
---

## What it is
A Growl-style notification hub for Windows, free and open source, with its own network protocol and GNTP compatibility. Growl for Windows was a parallel port, since discontinued.

## What was new
Brought per-app registered notification classes and network forwarding to Windows before Windows had a notification center.

## What went wrong / limits
Like Growl, made redundant by native toasts (Windows 8) and Action Center (Windows 10); remained a niche tool for scripts and home servers.

## Lessons for swaypplet
- Note: the surviving use is scripts and servers sending notifications to a desktop; `notify-send` over SSH or ntfy fills that role on Linux. No action needed beyond good handling of generic senders without a desktop entry.

## Sources
- https://github.com/fullphat/snarl_network_protocol/wiki/GNTP — Snarl's GNTP support. [verified via search summary]
- https://alternativeto.net/software/growl — Snarl as Growl alternative; Growl for Windows discontinued. [verified via search summary]
