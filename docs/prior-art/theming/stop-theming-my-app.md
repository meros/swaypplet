---
name: "Please don't theme our apps" open letter
slug: stop-theming-my-app
domain: theming
kind: pattern
platform: GNOME
vendor: independent GNOME app developers
years: 2019
status: current
tags: [theming, platform-stylesheet, distro-themes, controversy]
relevance: medium
---

## What it is
An open letter (stopthemingmy.app, May 2019) from independent GNOME app developers asking distributions to stop shipping custom GTK stylesheets and icon themes by default. It argued that GTK had no theming API, only stylesheet overrides, so themes broke apps.

## What was new
- Named the real problem: a "theme" that overrides an undocumented stylesheet is an unversioned fork of the platform UI, and every app update can break under it.
- Led to libadwaita (2021) with a fixed stylesheet and, later, a supported accent API, i.e. theming through a small set of semantic inputs instead of arbitrary CSS.

## What went wrong / limits
- Read by many users as hostility to customisation; the debate still recurs. Themers moved to overriding libadwaita CSS variables anyway.
- Distros (Ubuntu Yaru, Pop!_OS) kept their brands, reaching compromise via accent and recolouring rather than full themes.

## Lessons for swaypplet
- swaypplet's "five inputs, nothing else picks a colour" is the letter's conclusion done right: customisation through supported inputs, never through overriding the stylesheet.
- Resist adding a "custom CSS" setting; if users need more, add an input.

## Sources
- https://stopthemingmy.app/ — the letter [verified via search summary]
- https://www.omgubuntu.co.uk/2019/05/open-letter-stop-gtk-theming-distros — context [verified via search summary]
- https://linuxreviews.org/GNOME_Developers_have_Made_Their_Moves_against_Themes — aftermath [verified via search summary]
