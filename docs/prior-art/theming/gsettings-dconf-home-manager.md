---
name: GSettings/dconf and Home Manager dconf.settings
slug: gsettings-dconf-home-manager
domain: theming
kind: pattern
platform: GNOME
vendor: GNOME; nix-community
years: 2010–present
status: current
tags: [declarative, schema, defaults, locks, nix, home-manager]
relevance: high
---

## What it is
GSettings is GNOME's typed settings API: XML schemas declare keys, types, ranges and defaults; dconf stores user values in a binary database, with system databases for vendor defaults and **locks** that pin keys for all users. Home Manager's `dconf.settings` writes values declaratively from Nix (typed with `lib.hm.gvariant`), and `dconf2nix` converts a dump.

## What was new
- Schema-first: an app cannot read an undeclared key, defaults live in the schema, and "reset" means removing the user value.
- Layered databases (user → site → vendor) with locks, the enterprise answer to "defaults vs policy".
- Home Manager turned dconf into something reviewable in Git.

## What went wrong / limits
- Home Manager writes values on activation, so it overwrites runtime changes; users toggle something in the GUI and lose it on the next switch unless they stop managing that key.
- GVariant typing (`mkUint32`) is a frequent source of silent errors.
- Schema changes need recompiling schemas, a common NixOS gotcha.

## Lessons for swaypplet
- swaypplet's model (binary defaults < Nix system file < user file; absent section = default; reset removes the section) is exactly dconf's layering without Home Manager's overwrite problem. Keep it.
- Borrow locks: a Nix-side `locked = [ "elevate" ]` that makes the pane show a section read-only, for policy (e.g. a shared machine that must lock).
- The cross-repo guard that fails `nx-check` on a misspelled key is swaypplet's schema check; keep it at eval time.

## Sources
- https://github.com/nix-community/home-manager/blob/master/modules/misc/dconf.nix — module [verified via search summary]
- https://determinate.systems/blog/declarative-gnome-configuration-with-nixos/ — dconf2nix, typing [verified via search summary]
- https://en.wikipedia.org/wiki/Dconf — databases and locks [verified via search summary; locks from memory]
