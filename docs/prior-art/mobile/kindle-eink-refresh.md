---
name: Kindle and e-ink UI constraints
slug: kindle-eink-refresh
domain: mobile
kind: pattern
platform: other
vendor: Amazon, E Ink
years: 2007–present
status: current
tags: [e-ink, refresh, ghosting, stillness, low-power, cadence]
relevance: low
---

## What it is
E-ink screens hold an image with no power and redraw slowly. Kindle UIs mix **partial refreshes** (fast, but they leave ghosting after a few cycles) with a periodic **full refresh** (a black-white flash that clears ghosts). Changing a full screen with high fidelity takes about a second. The UI is therefore designed around few, deliberate redraws.

## What was new
- **Redraw as a budgeted resource**: designers decide which change deserves a flash.
- Stillness by default, with no animation at all and state change as a discrete step.
- A "refresh every N pages" setting exposes the trade-off to the user.

## What went wrong / limits
- Ghosting and flashing still bother users; menus and web browsing feel poor.
- Colour e-ink (Kaleido) lowers contrast and resolution.

## Lessons for swaypplet
- **Take redraw as a budget**: the e-ink constraint is BAR_VISION P7 taken to the limit. It is a good test: "would this change be worth a refresh on e-ink?" filters decorative motion.

## Sources
- https://www.pocket-lint.com/i-got-rid-of-page-ghosting-by-changing-this-kindle-setting/ — partial vs full refresh [verified via search summary]
- https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/10181169 — about 1 s high-fidelity full update [verified via search summary]
