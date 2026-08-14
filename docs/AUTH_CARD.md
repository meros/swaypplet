# The authentication card

One design, three surfaces: the lock screen, the greetd greeter and the polkit
dialog. This is the spec they are built to. It supersedes the ad-hoc layout
that grew row by row.

Provenance: a design pass that read Apple's login window, GNOME Shell's
`authPrompt.js`, KDE Plasma's lockscreen, Windows Hello and Android's
`BiometricPrompt`, produced three independent proposals, and judged each on
beauty, honesty-under-stress and GTK4 feasibility. What follows is the
synthesis, including the corrections the feasibility judges found.

## The problem this solves

The card obeyed one rule already: its geometry is decided when it is built and
never changes again. That rule is right and is not up for
renegotiation — a card that resizes while it is fading in, or under a pointer
already moving toward a button, is the bug it was written to kill.

Obeying it cost 42% of the card. Every row that could ever appear was
allocated whether or not it had anything to say, so the resting lock card was
a password box with two empty bands under it: a 71 px fingerprint pill waiting
for fprintd, and a 36 px message band waiting for something to go wrong. The
geometry never moved and the card looked like a form with the middle deleted.

The fix is not to give the rule up. It is to stop needing the rows.

## The idea

**The card stops being a stack of rows about authentication and becomes a
single authentication object.** A password field carries its own state as
marks inside its box, and one caption underneath is never empty.

- The fingerprint reader is a mark at the field's leading edge and three words
  grafted onto a sentence that had to be there anyway: "Touch the reader **or**
  enter your password".
- Caps Lock is a mark at the field's trailing edge, permanently allocated.
- The old pill and the old message band collapse into the caption, which is
  reserved for two lines, **top-aligned**, and always occupied.

Reserved space stops reading as a hole the moment its default tenant is real
text. That sentence is the whole design.

The lock card goes from 289 px to **148 px**, and its only
reserved-and-sometimes-blank pixels are the caption's second line, sitting
directly above the bottom padding — where slack reads as margin, not as a hole.
Top-alignment is what puts it there; a one-line message centred in a two-line
band leaves blank space bounded on both sides, which is the definition of a
hole.

## The honesty rule

Stated once, enforced everywhere:

> **The card names only methods that are accepting input at this instant.**

The caption says `Enter your password` while fprintd is still enumerating,
while the device is claimed by another process, while the user has no enrolled
prints, and forever on a machine with no reader. It says the fingerprint
sentence only between `EngineEvent::Ready` and `EngineEvent::Unavailable`, and
reverts on `Unavailable`.

There is no "starting up" state, and **the fingerprint mark paints nothing
until the reader arms**. A ghosted whorl was in the running and is rejected:
it is the card gesturing at a reader that cannot read. The mark's slot stays
allocated — that is what puts the greeter's username and password text on one
rail — but an unarmed reader gets no ink.

## Structure

One widget tree, three surfaces, built once in `src/auth_field.rs`:

```
GtkBox .auth-field            horizontal, spacing 12, hexpand   ← carries ALL chrome
├─ GtkOverlay .auth-mark      22×27, can_target=false           ← leading slot
│  ├─ GtkLabel .auth-mark-fp     (icons::FINGERPRINT, 22px)
│  └─ GtkBox   .auth-mark-face   (22×22 face ring)     [polkit elevate only]
├─ GtkPasswordEntry .auth-input  transparent, borderless, hexpand
└─ GtkLabel .auth-mark-caps      (U+F0632, 14px, width_request 16)
GtkLabel .auth-caption        wrap, lines(2), ellipsize End, xalign 0, yalign 0
```

`.auth-field` takes over `background-color`, `border`, `border-radius`,
`:focus-within`, the arm pulse and the reject flash. `.auth-input` is stripped
to a text node with a caret. The greeter's username row is the same
`.auth-field` with both marks unpainted.

`src/slot.rs` does not survive. It existed to hold rows open and fade them in
place, and once the rows were gone it had no callers: the field paints its own
marks and the caption is never empty, so there is nothing left to reserve. The
rule it carried moved to `src/auth_field.rs`, which is where it is now
enforced. What genuinely varies card-to-card — the polkit identity row, the
elevate face rows, the greeter's username field — is still decided before the
surface is presented, which is the same rule stated without a helper.

## Lock card

Vertical ledger, **identical in every state**:

| y | element | h |
|---|---|---|
| 0 | border top | 1 |
| 1 | padding top | 24 |
| 25 | `.auth-field` | **62** |
| 87 | margin | 6 |
| 93 | `.auth-caption` *(reserved)* | **34** |
| 127 | padding bottom | 20 |
| 147 | border bottom | 1 |
| | **total** | **148** |

Card 360 wide, content column 306.

**The switch-user button lives outside the card**, in the centred column below
the glass, `margin-top: 20`. This was decided during implementation and is a
change from the first draft, which put it inside: measured at 194 px, a button
under the caption bounded the caption's reserved second line on both sides and
turned it back into exactly the hole this design exists to remove. Below the
glass it leaves the caption as the card's last element, so the slack falls
against the bottom padding and reads as margin. It is also the card's only
non-authentication action, and a link on the wallpaper is what GNOME does with
"Not listed?" for the same reason.

```
   ┌──────────────────────────────────────────────────────┐ 0
   │                                                      │ 24
   │  ╔════════════════════════════════════════════════╗  │ 25
   │  ║ ◉      Password                             ·  ║  │    field 306×62
   │  ╚════════════════════════════════════════════════╝  │ 87
   │                                                      │ 6
   │  Touch the reader or enter your password             │ 93  reserved 306×34
   │  ·················································· │     line 2 reserved
   │                                                      │ 127
   └──────────────────────────────────────────────────────┘ 148
```

States, all at 148 px, differing only in paint:

| state | mark | field | caption |
|---|---|---|---|
| no reader / unarmed | unpainted | static | `Enter your password` |
| reader armed | `@accent` | border pulses 0.28↔0.72 | `Touch the reader or enter your password` |
| fingerprint hint | `@accent` | pulsing | hint, `@accent`, 2000 ms |
| verifying | `@accent` | static `alpha(@accent,.55)` | `Checking…` |
| wrong password | `@red` 380 ms | reject flash 900 ms | `Wrong password (n attempts)`, `@red` |
| Caps Lock on | — | — | caps mark `@yellow`; caption 2000 ms |
| wrong + Caps Lock | `@red` | flash | one composed line, second half at 75% alpha |
| switching user | unpainted | insensitive | `Switching…` |

**Why the height cannot move.** The field is always 62 px: `min-height: 38px`
+ `padding: 11px 12px` + 1 px border, and none of its three children ever
changes visibility — `opacity` and `color` only. The caption is always 34 px:
`min-height: 34px` covers two 12 px lines and the label is capped at
`lines(2)` with `ellipsize=End`, so no string can claim a third. Every state
change is a CSS class swap plus `set_markup`. There is not one `set_visible`
after build.

## Greeter

Two chips, editable username, the same field twice. **360×292** (was 379).

| y | element | h |
|---|---|---|
| 1 | padding top | 24 |
| 25 | chip row | 54 |
| 79 | margin | 18 |
| 97 | `.auth-field` (username) | 62 |
| 159 | margin | 10 |
| 169 | `.auth-field` (password) | 62 |
| 231 | margin | 6 |
| 237 | `.auth-caption` *(reserved)* | 34 |
| 271 | padding bottom | 20 |
| | **total** | **292** |

The username field's unpainted mark slot is why both fields' text starts at the
same x. That is the justification for reserving it, and it is a visible benefit
rather than a hole, because the space sits inside a painted border.

## Polkit

Content column 346. The fingerprint costs **zero** vertical pixels, so the
card is the same height with a reader and without one — `reserve_if` for the
fingerprint row disappears.

| y | element | h |
|---|---|---|
| 1 | padding top | 24 |
| 25 | `.polkit-icon-glyph` | 48 |
| 73 | margin | 8 |
| 81 | `.polkit-title` | 20 |
| 101 | margin | 6 |
| 107 | `.polkit-message` *(reserved, 2 lines)* | 40 |
| 147 | margin | 18 |
| 165 | `.auth-field` | 62 |
| 227 | margin | 6 |
| 233 | `.auth-caption` *(reserved)* | 34 |
| 267 | margin | 14 |
| 281 | `.polkit-details-toggle` | 26 |
| 307 | revealer (collapsed) | 0 |
| 307 | margin | 14 |
| 321 | `.polkit-actions` | 46 |
| 367 | padding bottom | 20 |
| | **as specified** | **388** |

**As built: 456** (was 529, so −14%). The row structure is the one specified —
one field, one caption, no pills — and the geometry is constant across every
state. What is not yet applied is the finer re-proportioning of the rows the
field does not touch: the icon block, the title/message margins, the details
toggle and the button sizes. Those are worth doing and are not what the
complaint was about, so they are left as a follow-up rather than smuggled into
a change whose claim is about dead space.

With the identity row: `+10 margin +32 row` after the caption → **430**.

The elevate variant (face confirm, no password path) builds the field with
`reserve_if(false)` and gives the caption row the mark overlay, so the face
ring holds the left edge and the wording changes beside it. **412** (was 523).

## The pinned face indicator

Constraint honoured: same position, new shape. It stays at `margin_top 56`,
top-centre under the lens, because looking at it aims the face on-axis to a
sensor with no depth channel. It becomes the caption line hoisted to the
camera: same 22 px mark, same 12 px type, same 12 px radius as the field, and
`min-width: 280px` so its outer box is exactly 306 — matching the card's
content column and fixing the 1 px overhang the current `274px` produces
(274 + 18 + 14 + 2 = 308 against 306). Height 36. Family by shared geometry
and material, not by proximity.

## Strings

| condition | caption |
|---|---|
| no reader, or reader not armed | `Enter your password` |
| reader armed | `Touch the reader or enter your password` |
| polkit, no reader | `Enter your password to allow this` |
| polkit, reader armed | `Touch the reader or enter your password to allow this` |
| elevate | `Look at the camera to allow this` |
| fp `verify-retry-scan` | `Try again` (2000 ms) |
| fp `verify-remove-and-retry` | `Remove and try again` (2000 ms) |
| Caps Lock turns on | `Caps Lock is on` (2000 ms) |
| submitted | `Checking…` |
| one failure | `Wrong password` |
| n failures | `Wrong password (n attempts)` |
| failure + Caps Lock | `Wrong password (n attempts)  ·  Caps Lock is on` |
| switching user | `Switching…` |
| arbitrary PAM/greetd text | verbatim, escaped, ≤2 lines, full text in the tooltip |

Separator `"  ·  "` (U+00B7). Priority is **total and static**: error > caps
edge > fp hint > resting. No queue, no dwell arithmetic, no two transients
alternating. Errors clear on the next keypress; hints and the caps edge on a
2000 ms timeout; the resting sentence is the floor underneath all of them and
is recomputed from `(fp_armed, surface_kind)` on every clear.

## GTK4 corrections found in review

These are the traps. Each was caught by a feasibility reviewer reading the
real widget behaviour, and each would have shipped a bug.

1. **No `line-height` on the caption.** GTK maps CSS `line-height` to
   `pango_attr_line_height_new()`, a factor on the *logical* line height, not
   on `font-size`. A 12 px UI font has a ~16 px logical line, so
   `line-height: 1.35` gives ~21.6 px per line and two lines measure ~43 px
   against a reserved 34 — `min-height` is a floor, not a cap, so the card
   grows the first time PAM says something long. That is exactly the
   regression this whole rule exists to prevent. Without it, 2 × 16 = 32 ≤ 34.
2. **Zeroing the entry needs `min-height: 0` on both the `entry` node and its
   `text` child**, or Adwaita's default 34 px content height silently sets the
   field's floor.
3. **The placeholder needs the node selector**, not the widget class.
4. **`set_accessible_description` is not a gtk4-rs method.** Overflowing text
   goes to `set_tooltip_text`.
5. `:focus-within` on a `GtkBox` reproduces `:focus` on the entry, and is
   already used in this stylesheet with a comment explaining why.
6. GTK does not collapse margins and a box has one `spacing`. Every gap here
   is an explicit margin, and the card boxes drop to `spacing: 0`.

## What is deleted

1. `.lock-fp-pill` / `.polkit-fp-pill`, widgets and chrome. The fingerprint
   has three facts and each keeps a home: *a reader is armed* is the lit mark,
   *how to use it* is the caption, *what it just said* is the caption. The
   pill was a container whose only job was to hold those next to each other,
   and the field already is that container.
2. `.lock-fp-label` / `.polkit-fp-label` and their ellipsize/hexpand
   scaffolding. The words move to a wider column with two lines of headroom.
3. The two-line 13 px `.lock-message` / `.polkit-status` band, replaced by the
   12 px top-aligned `.auth-caption`.
4. `set_fp_expected` and the `fp_slot` cell. Nothing about the card's geometry
   depends on whether a reader exists any more, so a wrong answer costs zero
   pixels instead of 71. `self_enrolled_blocking` stays — it still decides
   whether to start an fprintd verify at all.
5. The chrome on `.lock-entry` / `.polkit-entry`.
6. `.lock-verifying` / `.polkit-verifying` `opacity: 0.75`. Greying the whole
   card to say "PAM is thinking" is a card-scale signal for a field-scale
   event; on a 148 px card it reads as a fault.
7. Six of eleven type sizes and one of four weights.

Kept: `auth-shake` on the card, `auth-card-enter`, the handoff choreography,
the glass pane and its blur ramp, the face ring animations, the chip row, the
identity picker, the details revealer, and the constant-geometry rule itself.
The rule is not
weakened — it is made cheap enough that obeying it stops costing 42% of the
card.

`auth-shake` animates margins, which is a layout pass per frame, and a
reviewer argued for replacing it with a paint-only reject. Deferred
deliberately: it is a working, self-reverting animation and its cost is 380 ms
once per failure. The field's reject flash is added alongside it, not instead.

## The strongest objection

The caption is one channel with more tenants than before, and the
fingerprint's sentence is the one that always loses. Today the pill and the
message line are independent, so "Touch fingerprint reader" and "Wrong
password (3 attempts)" can both be on screen; after the fold they cannot — and
a rejected password is exactly when a user most needs telling the reader is an
alternative. GNOME shipped this bug: its band flickers between "(or place
finger on reader)" and "Sorry, that didn't work", and the priority-queue
machinery in `authPrompt.js` exists to arbitrate a slot GNOME made scarce.

Three answers. The offer is not evicted, only its wording: the mark stays lit
`@accent` inside the field for the whole error and the border keeps pulsing,
so the persistent fact keeps a persistent non-verbal home while the transient
fact gets the words. GNOME's flicker happened because both facts competed for
one verbal channel with nothing left behind when one won. The arbitration here
is a static total order, not a queue. And if the lit mark proves too quiet,
the remedy is a string inside an already-allocated box —
`Wrong password (3 attempts)  ·  or touch the reader` wraps to the reserved
second line and costs nothing. Being able to answer a legibility complaint by
editing a sentence rather than adding a row is what this design buys.

## Verification

Render each state and diff the card's bounding box; only paint may differ.

```sh
for st in "" fp fp-hint fp,error face face-ok; do
  SWAYPPLET_PREVIEW_LOCK_STATE="$st" dev/render.sh --mode preview:lock --out /tmp/lock-$st.png
done
SWAYPPLET_PREVIEW_CAPS=1 SWAYPPLET_PREVIEW_LOCK_STATE=fp,error dev/render.sh --mode preview:lock
SWAYPPLET_GREET_USERS=a,b dev/render.sh --mode preview:lock
SWAYPPLET_PREVIEW_POLKIT_STATE=fp,prompt,error dev/render.sh --mode preview:polkit
```

Required: identical bounding boxes within each surface, including the long-PAM
two-line case, which is the one the `line-height` trap breaks.

Measured at implementation, all bounding boxes identical within each surface:

| surface | states verified | before | after |
|---|---|---|---|
| lock | idle, armed, hint, error, long two-line error, Caps Lock, error+Caps | 289 | **148** |
| greeter | idle, armed, error+Caps | 379 | **~290** |
| polkit | fresh, armed, prompt, error+Caps | 529 | **456** |

The `long` preview token exists for the two-line case specifically: without it
a stylesheet regression that reintroduces `line-height` passes every other
check and only shows up in front of a user whose account has expired.
