---
name: Assistant and AI launchers (Siri, Cortana, Copilot, Raycast AI)
slug: assistant-launchers
domain: launchers
kind: pattern
platform: cross
vendor: Apple, Microsoft, Raycast, others
years: 2011–present
status: current
tags: [ai, assistant, natural language, llm, screen context, failure]
relevance: medium
---

## What it is
Launchers that accept natural language and answer or act: Siri (iPhone 2011, Mac 2016), Cortana in Windows search (2015–2023), the Windows Copilot key (2024), Raycast Quick AI (Tab from the root search), Spotlight with Apple Intelligence actions (2025).

## What was new
- **Same box, different engine**: Raycast's Tab turns the typed query into an LLM prompt with no mode switch.
- **Screen context**: Raycast on Windows (2026) and Copilot Vision can pass the focused window, selection or a screenshot as context.
- **Actions over apps**: App Intents let Spotlight run parameterised app actions.

## What went wrong / limits
- Cortana was pushed into the taskbar and search, then removed as a standalone app in 2023; users disabled it for speed and privacy.
- Voice-first launchers (Siri on the Mac) are rarely used at a desk.
- Cloud round trips blow the latency budget and send context off the machine.

## Lessons for swaypplet
- If added, an AI row belongs at the bottom as an explicit fallback ("Ask ..."), never in the ranked results, and never with screen context without a per-use confirmation.
- Low priority for swaypplet: not on the roadmap, and the host already runs Claude Code in terminals.

## Sources
- https://devtoolpicks.com/blog/raycast-2-public-beta-windows-indie-hackers-2026 — Quick AI, screen awareness [verified via search summary]
- https://en.wikipedia.org/wiki/Cortana_(virtual_assistant) — Cortana retirement 2023 [memory]
