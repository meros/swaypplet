---
name: Firefox OS
slug: firefox-os
domain: mobile
kind: product
platform: other
vendor: Mozilla
years: 2013–2016
status: failed
tags: [web-platform, low-end, ecosystem, html-apps]
relevance: low
---

## What it is
A phone OS whose whole UI (Gaia) and every app were web pages running on Gecko over a Linux base (Gonk). Aimed at cheap first smartphones in emerging markets (ZTE Open, 2013). Mozilla stopped phone development in December 2015 and ended the project in 2016; its code lives on in KaiOS.

## What was new
- The system UI written with web technology, with hardware reached through new web APIs (WebTelephony, WebSMS), some later standardised.
- Adaptive app search: type a word and get web apps for it, installed or not.

## What went wrong / limits
- Mozilla's own words: it "could not create a compelling and differentiating end-user value proposition" or build the full ecosystem.
- The hardware was very low-end and slow, which made the web UI feel slower still; cheap Android phones undercut it.
- The UI was an Android copy, not a reason to switch.

## Lessons for swaypplet
- **Avoid "same UI, different stack" as the value**: a shell has to earn its place with interactions, not with the technology beneath.
- Performance is the UI on low-end hardware; the frame-bench gate is the right defence.

## Sources
- https://blog.mozilla.org/futurereleases/2016/02/04/firefox-os-smartphones-and-2-6/ — end of smartphone work [verified via search summary]
- https://en.wikipedia.org/wiki/Firefox_OS — timeline, reasons [verified via search summary]
