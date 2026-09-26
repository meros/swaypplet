---
name: Sublime Text and VS Code command palettes
slug: vscode-sublime-command-palette
domain: launchers
kind: feature
platform: cross
vendor: Sublime HQ; Microsoft
years: 2011–present
status: current
tags: [command palette, goto anything, quick open prefixes, keybinding hints, recently used]
relevance: high
---

## What it is
Sublime Text 2 (2011) introduced Goto Anything (Ctrl+P) and the Command Palette (Ctrl+Shift+P). VS Code (2015) merged them into one Quick Open whose prefix picks the mode.

## What was new
- **Prefix modes in one box**: `>` commands, `@` symbols, `:` line, `#` workspace symbols, `?` lists the prefixes. Deleting the prefix returns to file search.
- **Recently used on top**: the palette opens with the last commands run.
- **Keybinding shown on every command**, so the palette doubles as the shortcut reference.
- Every extension command is in the palette by contract; there is no hidden command.

## What went wrong / limits
- Long command lists with vendor-prefixed titles ("GitLens: ...") get noisy; users rely on recency more than search.

## Lessons for swaypplet
- Take `?` to list the prefixes (`=`, `>`, `:`) with an example each.
- Take keybinding hints on rows (see [macos-help-menu-search](macos-help-menu-search.md)).
- swaypplet's `>` for commands matches the VS Code convention; keep it.

## Sources
- https://code.visualstudio.com/docs/getstarted/userinterface#_command-palette — prefixes, palette [memory]
- https://www.sublimetext.com/docs/command_palette.html — command palette [memory]
