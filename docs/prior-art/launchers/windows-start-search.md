---
name: Windows Start search (Cortana, Bing, Copilot)
slug: windows-start-search
domain: launchers
kind: product
platform: Windows
vendor: Microsoft
years: 2007–present
status: current
tags: [start menu, web results, bing, cortana, copilot key, failure]
relevance: high
---

## What it is
Type-to-search in the Start menu since Windows Vista (2007): apps, settings, files from the Windows Search index. Windows 10 (2015) merged it with Cortana and Bing web results; Windows 11 keeps Bing on by default and added a Copilot key on keyboards from 2024.

## What was new
- Vista made "press Win, type, Enter" the default way to launch on the most-used desktop OS.
- Settings pages as first-class results, which Command Palette's `$` prefix later made explicit.

## What went wrong / limits
- **Web results by default**: local queries go to Bing; users find it slow and a privacy leak, and disabling it took a registry key (`BingSearchEnabled`) for years.
- **Coupled to a cloud service**: in February 2020 a Bing-side failure left the Start search box blank worldwide.
- **Cortana** was pushed into search, then split out, then retired as a standalone app in 2023.
- EU DMA pressure (2023–24) forced clearer labels and an EEA opt-out; in 2026 Microsoft announced a simpler off switch and promised faster local search.

## Lessons for swaypplet
- Avoid any network provider in the default query path; web search belongs in explicit fallbacks.
- Avoid a local result waiting on a remote one: swaypplet's local-first rows and 40 ms elephant debounce already follow this; keep it as a rule.

## Sources
- https://www.windowslatest.com/2026/06/18/microsoft-announces-you-can-kill-bing-in-windows-11-search-and-boost-performance-after-years-of-lag/ — 2026 off switch [verified via search summary]
- https://woshub.com/disable-web-search-windows-start-menu/ — BingSearchEnabled [verified via search summary]
- https://www.techradar.com/computing/windows/windows-11s-start-menu-search-gets-new-clearer-labels-as-microsoft-tries-to-avoid-eu-regulation-trouble — EU labels [verified via search summary]
- https://www.theverge.com/2020/2/5/21124472/microsoft-windows-10-search-bing-outage — Feb 2020 outage [memory]
