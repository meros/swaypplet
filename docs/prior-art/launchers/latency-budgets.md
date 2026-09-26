---
name: Launcher latency budgets
slug: latency-budgets
domain: launchers
kind: pattern
platform: cross
vendor: Nielsen (1993), Doherty and Thadani (1982), tofi, Everything
years: 1982–present
status: current
tags: [latency, 100 ms, frame budget, debounce, first frame, perceived speed]
relevance: high
---

## What it is
The numbers a launcher is judged by: time from hotkey to first frame, and from keystroke to updated list.

## What was new
- Nielsen's limits: 0.1 s feels instant, 1 s keeps flow, 10 s loses attention. Doherty's 400 ms threshold for productivity.
- For typing, the real budget is one frame (8–16 ms): a list that lags the text looks broken even at 50 ms.
- tofi publishes 2.3–3.6 ms to first frame for a dmenu-sized surface; Everything answers per keystroke over a million names.
- Common structure: local sources inline, remote ones debounced (30–150 ms) and merged when they land, stale answers dropped by generation.

## What went wrong / limits
- Launchers that wait for all providers (early GNOME Shell search, Windows search with Bing) feel slow regardless of hardware.
- Debounce too long feels laggy; too short floods backends.

## Lessons for swaypplet
- swaypplet's design (local rows in the key's frame, elephant at 40 ms, generation numbers) matches best practice. Add measurement: log hotkey-to-present and key-to-present in a debug build, and state the targets (<16 ms keystroke, <50 ms open) in LAUNCHER.md.

## Sources
- https://www.nngroup.com/articles/response-times-3-important-limits/ — 0.1/1/10 s [memory]
- https://github.com/philj56/tofi — measured startup [verified]
- https://www.voidtools.com/faq/ — Everything index scale [verified]
