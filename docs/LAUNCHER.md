# Launcher

The search at the top of the Helm card (and the standalone launcher, the
same `LauncherView`). Code: `src/launcher/`. Settings: the Launcher tab
(`launcher` in the settings file, docs/SETTINGS.md).

## What a query can be

| Typed | Lists | Enter |
|---|---|---|
| nothing | what you launch most, then the installed apps | launches |
| `firefox` | apps, open windows, commands, pages and the other kinds switched on | launches the row; a page row opens that page |
| `2*21`, `sqrt 2` | a `= 42` row on top when the text is arithmetic | copies the result |
| `=expr` | the calculator only; what it cannot read goes to elephant's `calc` (units, currencies) | copies |
| `>cmd` | a "Run cmd" row, then commands on `PATH` from elephant's `runner` | runs `$SHELL -c cmd` through sway's `exec`, so it outlives the panel |
| `:prefix` | the Helm card's pages (unchanged; a bare `:` lists them) | |

Tab on an app row lists every open window of that app; Tab again goes back
to the results. On a row with no windows, the row shakes.

## Where results come from

Elephant (the walker daemon) is asked over its socket for the providers the
Launcher settings switch on (`sources::providers`). The launcher makes three
kinds of row itself, on the main thread: the calculator (`calc.rs`, a
recursive-descent parser over `f64`, no code evaluated), the command row, and
the pages by name. Open windows come from sway's tree, on a worker
(`windows.rs`).

Elephant's own `calc` is kept for what arithmetic is not: it runs `qalc`,
which costs a process start per query, so plain arithmetic never reaches it.

## Speed

A keystroke is one frame of work: the local rows are made, and the rows
elephant gave for the previous query are narrowed to those that still have
every query word in their name. The list redraws from that at once. Elephant
is asked 40 ms after the last key, on a worker, and its answer replaces the
narrowed rows when it lands; an answer to an older query is dropped by its
generation number. Nothing on the main thread waits on a socket or a file.

## Ranking by use

`frecency.rs`: one score per launched item that gains 1 per launch and halves
every 7 days, decayed lazily when read. A row keeps elephant's order and moves
up 3 places per unit of `ln(1 + score)`: ten launches this week lift a row
about seven places, one launch a month ago barely moves it. Frecency reorders
what matched and never adds what did not. Elephant is asked for twice the
rows shown, so a frecent row just below the cut can rise into view.

The empty query opens with up to six of the most used items, drawn from
memory before elephant answers. Calculator results, clipboard entries, symbols
and windows are not remembered: they are gone by the next launch.

The store is `$XDG_STATE_HOME/swaypplet/launcher-frecency.json`, read on a
worker once per process, written on a worker 2 s after the last launch (write
then rename), at most 300 items. Forget history on the Launcher tab deletes it.
