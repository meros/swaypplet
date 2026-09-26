# Launchers: prior art

49 entries: 34 products and features, 15 patterns, protocols and algorithms. Sources marked [verified] were read on 2026-09-26; [memory] ones were not re-checked.

## Landscape

Three lineages. **Verb-object shells** ([Quicksilver](quicksilver.md), [LaunchBar](launchbar.md), [Kupfer](kupfer.md), [GNOME Do](gnome-do.md)) treat the bar as a grammar; their ideas survive as action menus. **Platform launchers** ([Alfred](alfred.md), [Raycast](raycast.md), [PowerToys Command Palette](powertoys-command-palette.md), [Vicinae](vicinae.md)) win on an extension ecosystem with a uniform row and action model. **System search** ([Spotlight](spotlight.md), [Windows Start](windows-start-search.md), [GNOME providers](gnome-shell-search-providers.md), [KRunner](krunner-milou.md), [ChromeOS](chromeos-launcher.md)) owns the default key and is judged on trust and speed. On Wayland, the [dmenu protocol](dmenu-protocol.md) family ([rofi](rofi.md), [fuzzel](fuzzel.md), [tofi](tofi.md), [wofi](wofi.md)) handles scripts, and the newer Rust/GTK4 launchers ([walker and elephant](walker-elephant.md), [anyrun](anyrun.md), [Sherlock](sherlock.md)) compete on providers. In-app palettes ([VS Code/Sublime](vscode-sublime-command-palette.md), [Cmd+K apps](cmd-k-web-apps.md), [omnibox](browser-omnibox.md)) set user expectations for prefixes and shortcut hints.

## The 10 best ideas

1. **Query-to-choice learning**: rank the item the user picked for this typed prefix first ([adaptive-input-history](adaptive-input-history.md)).
2. **Uniform secondary actions** per row kind, with the key shown ([action-panel](action-panel.md)).
3. **Decayed frecency stored as a date**, so no decay math runs at read time ([firefox-frecency](firefox-frecency.md)).
4. **Fallback rows** that touch the network only when picked ([fallback-searches](fallback-searches.md)).
5. **Local-first, async merge, stable top row** ([latency-budgets](latency-budgets.md), [browser-omnibox](browser-omnibox.md)).
6. **Optimal fuzzy scoring with boundary bonuses** ([fzf](fzf-algorithm.md), [fzy](fzy-algorithm.md), [nucleo](nucleo-matcher.md)).
7. **Top Hit** across all providers ([spotlight](spotlight.md)).
8. **Teach the shortcut on the row** ([macOS Help search](macos-help-menu-search.md), [VS Code](vscode-sublime-command-palette.md)).
9. **Out-of-process providers, the app as provider** ([GNOME SearchProvider2](gnome-shell-search-providers.md), [plugin-security-models](plugin-security-models.md)).
10. **Pins and contextual rows on the empty query** ([Command Palette](powertoys-command-palette.md), [ChromeOS continue](chromeos-launcher.md)).

## 8 instructive failures

1. [Unity shopping lens](unity-shopping-lens.md): local queries sent to Amazon by default; trust never recovered.
2. [Windows Start search](windows-start-search.md): Bing in the local path; a 2020 cloud outage blanked Start search.
3. [Unity HUD](unity-hud.md): menu search that depended on menus apps were dropping.
4. [Synapse and Zeitgeist](synapse-zeitgeist.md): a desktop-wide activity log that apps never fed.
5. [GNOME Do](gnome-do.md): bound to Mono and to specific apps that faded.
6. [Cerebro](cerebro.md): Electron and unreviewed npm plugins for a one-frame surface.
7. [Ulauncher v6](ulauncher.md) and [Wox](wox.md): long rewrites that froze the plugin API; the community forked ([Flow Launcher](flow-launcher.md)).
8. [PowerToys Run](powertoys-run.md) and [anyrun](anyrun.md): in-process plugins; Microsoft replaced Run rather than retrofit isolation.

## Ideas for swaypplet, ranked

| # | Idea | Size | Source |
|---|---|---|---|
| 1 | Learn (typed prefix → item): a decayed map beside `frecency.rs`, consulted before the per-item boost | S–M | [adaptive-input-history](adaptive-input-history.md) |
| 2 | Fallback rows when fewer than 3 results: web search, run, search files | S | [fallback-searches](fallback-searches.md) |
| 3 | Action list per row kind, generalising Tab-for-windows; show elephant's `actions`; hint the second action's key | M | [action-panel](action-panel.md) |
| 4 | Score the local narrowing pass with `nucleo-matcher` (fzf bonuses, å/ä/ö folding) instead of word containment | S | [nucleo-matcher](nucleo-matcher.md) |
| 5 | Weight launches by how they happened (typed query > empty-state click) so suggestions do not feed themselves | S | [firefox-frecency](firefox-frecency.md) |
| 6 | Keep the selected row still once the user is about to press Enter when elephant's answer lands | S | [browser-omnibox](browser-omnibox.md) |
| 7 | `?` lists prefixes with examples; a typed prefix becomes a chip | S | [VS Code](vscode-sublime-command-palette.md), [Sherlock](sherlock.md), [KRunner](krunner-milou.md) |
| 8 | Keybinding hints on rows (panel pages, sway bindings) | S | [macos-help-menu-search](macos-help-menu-search.md) |
| 9 | Empty query: user pins above frecent; recent files from `recently-used.xbel` | S–M | [Command Palette](powertoys-command-palette.md), [ChromeOS](chromeos-launcher.md) |
| 10 | Focused-window actions (move, float, close) as context rows | M | [cmd-k-web-apps](cmd-k-web-apps.md) |
| 11 | Launch the selection: a binding that opens the launcher with the primary selection as query | S | [LaunchBar](launchbar.md) |
| 12 | Measure and publish open and keystroke latency in LAUNCHER.md | S | [latency-budgets](latency-budgets.md), [tofi](tofi.md) |
| 13 | Replace the qalc fallback with the `rink` crate (no process per query) | M | [anyrun](anyrun.md) |
| 14 | Consume GNOME SearchProvider2 providers as a source | M | [gnome-shell-search-providers](gnome-shell-search-providers.md) |

## Gap analysis against the current launcher

Already at or above prior art: local-first rows in the key's frame, 40 ms debounce with generation drop, lazy-decay frecency that reorders but never adds, no network in the default path, providers out of process through elephant, `=` and `>` prefixes matching Command Palette and VS Code conventions.

Missing, in order of user-visible effect:
- **Learning is per item, not per query.** Typing "te" twice for Telegram does not make "te" mean Telegram; Quicksilver did this in 2003.
- **Only one action per row**, plus Tab for windows; elephant's other actions are unreachable from the keyboard.
- **Nothing for "no results".** No fallback rows.
- **Local matching is word containment**, not scored fuzzy matching; abbreviations like "vsc" rely on elephant's answer arriving.
- **Two history owners.** elephant keeps its own launch history for desktop apps; swaypplet's frecency sits on top. One should own it ([walker-elephant](walker-elephant.md)).
- **No discoverability** of prefixes or keybindings from inside the launcher.

Deliberately out of scope, consistent with the Rejected list: web results in the ranked list, live network tiles, AI rows ([assistant-launchers](assistant-launchers.md)), in-process plugins.
