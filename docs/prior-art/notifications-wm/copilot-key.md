---
name: Copilot key
slug: copilot-key
domain: notifications-wm
kind: feature
platform: Windows
vendor: Microsoft
years: 2024–present
status: current
tags: [keyboard, hardware, assistant, remapping, accessibility]
relevance: low
---

## What it is
A dedicated key on Windows laptops from 2024, billed as the biggest keyboard change in about 30 years, usually replacing the right Ctrl or Menu key. It sends Win+Shift+F23. A later Windows 11 update let users remap it to open another app; remapping back to Ctrl needed PowerToys or third-party tools.

## What was new
Hardware real estate given to a product feature.

## What went wrong / limits
It removed a key people and screen readers relied on; Microsoft had to document the accessibility regressions. The emitted combination makes clean remapping awkward.

## Lessons for swaypplet
- Take: keybindings are for the owner's workflows; never claim a common key for a shell feature, and make every binding remappable in one place (the Nix keybind table).
- Note: if such keyboards show up, map Win+Shift+F23 to something useful (launcher) rather than leaving it dead.

## Sources
- https://support.microsoft.com/en-us/accessibility/windows/copilot/understand-updates-to-the-copilot-key-on-windows-devices — accessibility issues, remap setting. [verified via search summary]
- https://www.tomsguide.com/news/windows-copilot-is-getting-its-own-key-on-the-keyboard-heres-why-thats-a-big-deal — announcement. [verified via search summary]
- https://learn.microsoft.com/en-us/answers/questions/1818415/map-copilot-key-back-to-control-key — remapping trouble. [verified via search summary]
