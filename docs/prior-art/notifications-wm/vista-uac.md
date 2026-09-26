---
name: Windows Vista User Account Control
slug: vista-uac
domain: notifications-wm
kind: feature
platform: Windows
vendor: Microsoft
years: 2007–present
status: current
tags: [permissions, prompts, alert-fatigue, secure-desktop, dialog-blindness]
relevance: high
---

## What it is
Vista (2007) ran admins with standard rights and asked for consent on the dimmed secure desktop before any elevation. Many ordinary actions triggered it, sometimes several prompts for one task. Windows 7 (2009) added a four-step slider and removed prompting from 16 places; Microsoft said Windows 7 would prompt 29% less.

## What was new
Least privilege for desktop users, with a secure desktop that other apps cannot draw on or click through (the model polkit agents and swaypplet's auth card follow).

## What went wrong / limits
Prompt frequency trained users to click Continue without reading (dialog blindness), which defeats the prompt. Apple's "I'm a Mac" ads mocked it. A Microsoft manager was quoted saying UAC was designed to annoy users into pressuring developers.

## Lessons for swaypplet
- Take: every alert competes with every other; the value of a critical notification, a polkit prompt or a bar red state depends on how rarely it fires (BAR_VISION P9's cry-wolf point).
- Take: keep the secure-surface idea (auth card on its own layer, not spoofable).

## Sources
- https://learn.microsoft.com/ko-kr/archive/blogs/tarpara/windows-7-to-nag-29-less-than-vista — 29% fewer prompts. [verified via search summary]
- https://www.bruceb.com/2009/11/windows-7-uac-and-dialog-blindness/ — dialog blindness. [verified via search summary]
- https://www.informationweek.com/it-leadership/how-to-tame-microsoft-windows-vista-s-uac — prompt barrage. [verified via search summary]
- "Annoy users" quote. [memory]
