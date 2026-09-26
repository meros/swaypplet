---
name: Windows Desktop Window Manager
slug: windows-dwm
domain: motion
kind: product
platform: Windows
vendor: Microsoft
years: 2006–present
status: current
tags: [compositor, aero, glass, flip-model, directcomposition, fallback]
relevance: medium
---

## What it is
The compositing window manager introduced with Windows Vista (2006/2007). It composites every window's offscreen surface with Direct3D and drew Aero Glass: blurred, tinted title bars.

## What was new
Composition made whole-desktop effects cheap enough to ship on by default: Aero Glass blur, Flip 3D, live thumbnails. Later DirectComposition (Windows 8) let apps hand visual trees and animations to DWM, and flip-model presentation reduced copies and latency.

## What went wrong / limits
- Vista's Aero required a WDDM driver and DX9 GPU; on weaker machines it fell back to "Basic", and the "Vista Capable" logo on machines that could not run Aero became a class action.
- Windows 8 removed Aero Glass blur entirely; Windows 10 and 11 brought it back as Acrylic and Mica with explicit fallbacks.
- DWM always composited from Windows 8 on, adding a frame of latency that game players fought with fullscreen exclusive and later independent flip.

## Lessons for swaypplet
- Avoid: shipping a material that silently needs hardware not everyone has; name the minimum GPU and define the fallback.
- Take: direct scanout / independent flip for fullscreen clients is the latency escape hatch; swayfx should never apply glass paths when a fullscreen client could scan out.

## Sources
- https://learn.microsoft.com/en-us/windows/win32/dwm/dwm-overview — DWM overview [memory]
- https://en.wikipedia.org/wiki/Windows_Aero — Aero requirements, Vista Capable lawsuit, removal in Windows 8 [memory]
