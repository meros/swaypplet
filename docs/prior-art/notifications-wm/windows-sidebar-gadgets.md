---
name: Windows Sidebar and Desktop Gadgets
slug: windows-sidebar-gadgets
domain: notifications-wm
kind: feature
platform: Windows
vendor: Microsoft
years: 2007–2012
status: discontinued
tags: [widgets, gadgets, sidebar, security, html]
relevance: medium
---

## What it is
Windows Vista (2007) shipped a Sidebar docked to the screen edge holding HTML/JScript gadgets (clock, CPU meter, feeds, slideshow). Windows 7 freed them onto the desktop. In July 2012 Microsoft Security Advisory 2719662 offered a Fix it to disable Sidebar and Gadgets because gadgets could allow remote code execution; Windows 8 dropped them.

## What was new
Persistent, always-visible mini-apps beside windows, with a gallery of third-party gadgets.

## What went wrong / limits
Gadgets ran with the user's full rights and many did not follow secure coding practices, so the platform became an attack surface Microsoft chose to remove rather than fix. The docked Sidebar ate screen width on the 4:3 and 16:10 screens of the time. Always-on feeds and meters are motion and change at rest.

## Lessons for swaypplet
- Avoid: a third-party widget API executing untrusted code in the shell process.
- Avoid: permanent sidebars of information; BAR_VISION's still bar is the stronger design.

## Sources
- https://learn.microsoft.com/en-us/security-updates/securityadvisories/2013/2719662 — advisory. [verified via search summary]
- https://en.wikipedia.org/wiki/Windows_Desktop_Gadgets — history, Windows 8 deprecation. [verified via search summary]
- Screen width cost. [memory]
