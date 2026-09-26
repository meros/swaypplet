---
name: grim, slurp, swappy, satty and hyprpicker
slug: screenshot-tools
domain: wayland
kind: product
platform: Wayland
vendor: emersion (grim, slurp); jtheoof (swappy); gabm (satty); hyprwm
years: 2018–present
status: current
tags: [screenshot, region-select, annotation, colour-picker, composition]
relevance: medium
---

## What it is
The composable screenshot chain. grim captures an output or region; slurp draws a region selector on a layer surface and prints geometry; swappy (GTK3) and satty (GTK4, Relm4) annotate the image; hyprpicker freezes the screen and picks a colour. Users pipe them: `grim -g "$(slurp)" - | satty -f -`.

## What was new
- Unix composition for a desktop task: each tool is replaceable.
- satty: annotation immediately after capture with fullscreen editing, copy-and-exit in one key, and tools picked by single keys [memory].
- Freezing the frame before selection (hyprpicker, `grim` with a frozen layer) so menus and tooltips can be captured.

## What went wrong / limits
- The pipeline has visible seams: a frame of delay between slurp closing and grim capturing, and no window-snap selection unless the script queries the compositor tree.
- No shared UX; every user builds their own script.

## Lessons for swaypplet
- swaypplet's in-process screenshot already removes the seams; take satty's single-key tool switching and "copy and close" as the annotation defaults, and freeze-before-select as the default capture mode.

## Sources
- https://sr.ht/~emersion/grim/ — grim [memory]
- https://github.com/gabm/Satty — satty [memory]
- https://github.com/hyprwm/hyprpicker — hyprpicker [memory]
