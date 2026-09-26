---
name: xdg-activation-v1
slug: xdg-activation
domain: wayland
kind: protocol
platform: Wayland
vendor: wayland-protocols
years: 2020–present
status: current
tags: [protocol, focus-stealing, launch, startup-notification]
relevance: medium
---

## What it is
A staging protocol for passing focus between clients. A launcher (or notification daemon) asks the compositor for a token tied to the user's input event, passes it to the launched app via `XDG_ACTIVATION_TOKEN` (or D-Bus `activation-token` platform data), and the app presents a surface with that token to be allowed focus.

## What was new
- Focus-stealing prevention without heuristics: only an app handed a token from a real input event may take focus.
- The standard fix for "clicked a notification, nothing came to front".

## What went wrong / limits
- Every link in the chain must forward the token: launchers, `gtk-launch`, D-Bus activation, Flatpak portals. One link that drops it and the window opens unfocused or the compositor marks it urgent instead.
- Compositors differ on what they do with an invalid token (sway marks urgent) [memory].

## Lessons for swaypplet
- Check that the launcher, pins and notification actions hand a token to what they start or raise; GTK's `GdkAppLaunchContext` does it for `GAppInfo` launches, but raw `spawn` paths do not. A notification's default action should pass `activation-token` (FDO notifications 1.2).

## Sources
- https://wayland.app/protocols/xdg-activation-v1 — protocol [memory]
- https://specifications.freedesktop.org/notification-spec/latest/ — activation-token signal [memory]
