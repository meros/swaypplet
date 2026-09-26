---
name: The dmenu protocol
slug: dmenu-protocol
domain: launchers
kind: protocol
platform: cross
vendor: suckless (dmenu), adopted by rofi, wofi, fuzzel, tofi, bemenu, walker
years: 2006–present
status: current
tags: [stdin, stdout, unix pipe, adapter, scripts]
relevance: high
---

## What it is
dmenu reads newline-separated items on stdin, lets the user filter and pick one, and prints it on stdout. Exit code 1 on cancel. Every Linux menu since accepts the same contract with `--dmenu`.

## What was new
- A menu as a filter in a pipe: `pass | dmenu | xargs pass -c`. Thousands of scripts (password pickers, wifi, power menus, emoji) work with any compliant menu.
- Swappable front ends: users change the look without touching their scripts.

## What went wrong / limits
- Text only: icons, hidden payloads and multi-select needed per-tool extensions (rofi's `\0` options, fuzzel's icon syntax), which fragments the contract.
- One-shot: no streaming updates, no async results.

## Lessons for swaypplet
- Keep the dmenu chassis strictly compatible with the plain contract, then accept rofi's row options (`icon`, `meta`, `info`) as the one extension, since the most scripts already emit it.

## Sources
- https://tools.suckless.org/dmenu/ — dmenu [memory]
- https://davatorium.github.io/rofi/current/rofi-script.5/ — rofi's extension of it [verified]
