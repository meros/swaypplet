---
name: rofi
slug: rofi
domain: launchers
kind: product
platform: cross
vendor: Dave Davenport (davatorium) and contributors
years: 2014–present
status: current
tags: [modes, combi, script mode, dmenu, x11, wayland, theming]
relevance: high
---

## What it is
A window switcher, launcher and dmenu replacement for X11, forked from simpleswitcher in 2014; Wayland support lived in the lbonn fork and was merged upstream in rofi 2.0 (2025). Built-in modes: window, run, drun, ssh, filebrowser, keys, combi.

## What was new
- **Modes** switched with a key (Shift+Right) or combined (`combi`), each with its own prefix.
- **Script mode**: any executable is a mode. It is called with no argument for the list, then with the chosen row; `ROFI_RETV` says why (0 start, 1 selected, 2 custom text, 3 delete, 10–28 custom keys).
- **Row options** in-band: `text\0icon\x1ffirefox\x1fmeta\x1fbrowser`: icon, `info` (hidden payload), `meta` (hidden search terms), `display` (shown text differs from matched text), nonselectable.
- **Mode options**: prompt, message, markup-rows, keep-selection, keep-filter, `data` round-tripped via `ROFI_DATA`.
- A CSS-like theme language widely shared as dotfiles.

## What went wrong / limits
- A new process per script step; stateful scripts are awkward.
- X11-first for a decade; Wayland users moved to wofi, fuzzel, tofi and walker meanwhile.

## Lessons for swaypplet
- Take `meta` (hidden search terms) and `display` vs match text: an app row should match on keywords and GenericName without showing them.
- Take the script-mode contract as the dmenu chassis's extended protocol (see [dmenu-protocol](dmenu-protocol.md)).

## Sources
- https://davatorium.github.io/rofi/current/rofi-script.5/ — script protocol, ROFI_RETV, row and mode options [verified]
- https://github.com/davatorium/rofi — modes, Wayland merge [memory]
