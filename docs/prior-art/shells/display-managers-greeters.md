---
name: Linux display managers and greeters (GDM, SDDM, LightDM, greetd)
slug: display-managers-greeters
domain: shells
kind: pattern
platform: cross
vendor: GNOME, KDE, Canonical, greetd project
years: 2004–present
status: current
tags: [greeter, login, lock-screen, consistency, wayland]
relevance: medium
---

## What it is
The login screen on Linux is a separate program from the session's lock screen: GDM (GNOME, which reuses GNOME Shell in greeter mode), SDDM (KDE, QML themes), LightDM (Canonical, pluggable greeters) and greetd (a minimal daemon with greeters such as tuigreet, gtkgreet or regreet). Lock screens are provided by the session (GNOME Shell, kscreenlocker, swaylock) through different code.

## What was new
GDM running GNOME Shell as the greeter makes login and lock look and behave the same, since they are the same code. greetd separates the privileged daemon from an unprivileged greeter UI, so any Wayland client can be a greeter.

## What went wrong / limits
- On most non-GNOME setups login and lock screens are visually unrelated, and the transition from greeter to session is a black flash.
- SDDM themes often broke on Qt major upgrades.
- LightDM's greeters (Unity greeter, slick-greeter) aged with their desktops.

## Lessons for swaypplet
- Take: greetd with a swaypplet-drawn greeter would let login, lock and polkit share AUTH_CARD's card, the way GDM shares GNOME Shell; the gap today is the login screen.
- Take: design the greeter-to-session handoff as a transition (the lock cross-fade already exists) instead of accepting a black frame.

## Sources
- https://git.sr.ht/~kennylevinsen/greetd — greetd architecture [memory]
- https://wiki.archlinux.org/title/Display_manager — overview of DMs [memory]
