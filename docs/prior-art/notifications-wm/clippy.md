---
name: Clippy (Office Assistant)
slug: clippy
domain: notifications-wm
kind: feature
platform: Windows
vendor: Microsoft
years: 1997–2007
status: failed
tags: [assistant, interruption, proactive-help, anthropomorphism]
relevance: medium
---

## What it is
An animated paperclip in Office 97 that watched what the user did and offered help ("It looks like you're writing a letter"). Microsoft announced in April 2001 that it would be dropped; Office XP kept it but off by default, with an ad campaign mocking it; Office 2007 removed it.

## What was new
Proactive help inferred from behaviour, grounded in Microsoft Research's Bayesian user modelling work (Lumière).

## What went wrong / limits
It interrupted on weak inference, repeatedly, with no memory that the user had declined. It sat on top of the work and animated while idle. Research's cautious model was shipped with far more eagerness than it justified.

## Lessons for swaypplet
- Avoid: unsolicited suggestions from the shell. Every proactive surface needs a very high confidence bar and a permanent "no".
- Take: idle animation of a helper is itself an interruption (BAR_VISION P2).

## Sources
- https://en.wikipedia.org/wiki/Office_Assistant — dates, off by default in XP, removal in 2007. [verified via search summary]
- https://www.seattlemet.com/news-and-city-life/2022/08/origin-story-of-clippy-the-microsoft-office-assistant — history. [verified via search summary]
- Lumière research lineage. [memory]
