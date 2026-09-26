---
name: Wox
slug: wox
domain: launchers
kind: product
platform: Windows
vendor: Wox project (qianlifeng)
years: 2013–present
status: niche
tags: [alfred clone, python plugins, stalled, fork, rewrite]
relevance: low
---

## What it is
An Alfred-style launcher for Windows with C# and Python plugins and a plugin site. It was the base for PowerToys Run and for Flow Launcher.

## What was new
Brought Alfred's plugin-with-keyword model to Windows with a public plugin registry, when Windows had nothing comparable.

## What went wrong / limits
- Development stalled around 2018–2020; pull requests sat, and the community forked it into Flow Launcher while Microsoft took the code into PowerToys.
- A later ground-up rewrite (Go core, Flutter UI) split attention again.

## Lessons for swaypplet
- Avoid rewrites that strand the plugin ecosystem; keep the provider contract stable across UI rewrites (the launcher UI changes, elephant's protocol does not).

## Sources
- https://github.com/Wox-launcher/Wox — history, rewrite [memory]
