---
name: wl-clipboard, cliphist, clipse and stash
slug: clipboard-managers
domain: wayland
kind: product
platform: Wayland
vendor: Sergey Bugaev (wl-clipboard); sentriz (cliphist); savedra1 (clipse); NotAShelf (stash)
years: 2018–present
status: current
tags: [clipboard, history, data-control, sensitive, privacy]
relevance: medium
---

## What it is
wl-clipboard (`wl-copy`, `wl-paste --watch`) is the CLI over the data-control protocol. cliphist stores history (text and images) fed by `wl-paste --watch cliphist store` and pipes it to any picker. clipse is a TUI manager with image previews. stash is a newer daemon that skips entries marked sensitive.

## What was new
- The `x-kde-passwordManagerHint` MIME convention: KeePassXC and `wl-copy --sensitive` (2.3+) mark secrets, and managers that watch MIME types can skip them.
- Pipe architecture: storage and picker are separate programs.

## What went wrong / limits
- `wl-paste --watch` receives only the data, not the offer's MIME list, so cliphist cannot see the password hint; secrets end up in a history file on disk.
- History persisted as plain files; images cost disk and memory without caps.

## Lessons for swaypplet
- swaypplet already honours the hint and keeps history in memory (its Rejected list explains why); that is the right answer. Document it as a selling point.

## Sources
- https://github.com/sentriz/cliphist — cliphist [verified via search summary]
- https://github.com/NotAShelf/stash — sensitive MIME skip [verified via search summary]
- https://github.com/keepassxreboot/keepassxc/discussions/10704 — passwords in cliphist [verified via search summary]
