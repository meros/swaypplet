---
name: Ubuntu convergence (Unity 8, Ubuntu Touch, Ubuntu Edge)
slug: ubuntu-convergence
domain: notifications-wm
kind: project
platform: other
vendor: Canonical
years: 2011–2017
status: failed
tags: [convergence, phone-as-desktop, unity8, mir]
relevance: low
---

## What it is
Canonical's plan for one shell (Unity 8, on its own Mir display server) across phone, tablet and desktop, with a phone that docks to become a PC. The 2013 Ubuntu Edge crowdfunding sought 32 million dollars and fell short. In April 2017 Mark Shuttleworth ended Unity 8, the phone and convergence; Ubuntu returned to GNOME. UBports continues it as Lomiri.

## What was new
Adaptive shell that changes layout with the attached display and input.

## What went wrong / limits
Years late, with its own display server splitting the Linux graphics effort; no app ecosystem for phones; the desktop languished while resources went to the phone.

## Lessons for swaypplet
- Avoid: building for a second form factor; swaypplet targets one owner's desktop and laptop outputs.
- Note: per-output layout (docked vs laptop panel) is the useful slice of convergence, and display profiles (roadmap item 2) cover it.

## Sources
- https://www.omgubuntu.co.uk/2017/10/why-did-ubuntu-drop-unity-mark-shuttleworth-explains — reasons. [verified via search summary]
- https://www.phoronix.com/news/Ubuntu-Dropping-Unity — April 2017 announcement. [verified via search summary]
- https://en.wikipedia.org/wiki/Lomiri — continuation. [verified via search summary]
- Edge crowdfunding shortfall. [memory]
