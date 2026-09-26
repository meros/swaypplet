---
name: Raycast
slug: raycast
domain: launchers
kind: product
platform: cross
vendor: Raycast Technologies
years: 2020–present
status: current
tags: [extension store, react, action panel, ai, window management, snippets, clipboard]
relevance: high
---

## What it is
A launcher that became a platform: apps, commands, clipboard history, snippets, window management, floating notes and an AI chat in one bar. Extensions are React/TypeScript, published to a public store from one open-source monorepo. Windows public beta opened 2025-11-20 and left beta on 2026-08-25.

## What was new
- **Action panel** (Cmd+K): every row has a list of actions; Enter is the first, Cmd+Enter the second, the rest are searchable. Uniform across all extensions.
- **Declarative UI**: extensions describe List/Detail/Form views; Raycast renders them natively, so third-party UI looks first-party.
- **Store with review**: all extensions open source, reviewed by PR, CI-validated, auto-updated.
- **Quick AI**: Tab from the root search sends the query to an LLM; Windows adds "screen awareness" (focused window, selection, screenshot as context).

## What went wrong / limits
- Extensions run in a Node child process, each in its own V8 isolate, but are "not heavily sandboxed": filesystem and network are open; OS permission prompts are the only gate.
- AI and cloud sync are behind a subscription; the free/paid line moves, which users notice.
- Node runtime adds memory and a cold start per extension.

## Lessons for swaypplet
- Take the action panel pattern; Tab-for-windows is one instance of it (see [action-panel](action-panel.md)).
- Take the declarative row model: providers return data, the shell owns the look; this keeps the design system intact.
- Avoid a JS runtime in the shell process; if a plugin API ever exists, keep it out of process.

## Sources
- https://developers.raycast.com/information/security — open source, review, V8 isolates, not sandboxed [verified]
- https://developers.raycast.com/basics/review-pullrequest — PR-based store [verified]
- https://www.raycast.com/blog/raycast-for-windows — Windows beta [verified via search summary]
- https://devtoolpicks.com/blog/raycast-2-public-beta-windows-indie-hackers-2026 — native host per platform [verified via search summary]
