# Prior art: notifications, attention, widgets and window management

**53 entries. The strongest lesson across them: interruption is a budget, and the systems that lasted spend it by context and by the user's rule, not by the sender's wish.**

## Landscape

- **Notification systems** converged on the same shape in about 15 years: a sender declares type and urgency ([freedesktop](freedesktop-notification-spec.md), [Android channels](android-notification-channels.md), [iOS interruption levels](ios-interruption-levels.md)); the user owns presentation per app or per type ([macOS](macos-notification-center.md), [Growl](growl.md)); the shell groups per app ([Android bundles](android-bundled-notifications.md), [GNOME 48](gnome-shell-notifications.md), [Plasma](kde-plasma-notifications.md)) and holds everything during a context ([Focus Assist](windows-focus-assist.md), [Focus](ios-focus-modes.md), [presentation detection](presentation-mode-dnd.md)). The newest layer, AI summaries, [broke trust in a month](apple-intelligence-summaries.md).
- **Widgets** have been reinvented five times ([Dashboard](macos-dashboard.md), [Gadgets](windows-sidebar-gadgets.md), [Live Tiles](windows-live-tiles.md), [Windows 11 Widgets](windows-11-widgets.md), [Sonoma](macos-desktop-widgets.md)); only host-rendered, declarative ones survived ([Android](android-app-widgets.md), WidgetKit).
- **Window management** alternates between spatial overviews that work ([Exposé](expose-mission-control.md), [workspaces](virtual-desktops-spaces.md), [switchers](alt-tab-switchers.md)) and grouping schemes that did not stick ([Stage Manager](stage-manager.md), [Activities](kde-activities.md), [Sets](windows-sets.md), [Timeline](windows-timeline.md)). 3D desktops ([Looking Glass](sun-looking-glass.md), [BumpTop](bumptop.md), [Flip 3D](windows-flip-3d.md), [Compiz](compiz-cube-wobbly.md)) left one or two ideas each.

## 10 best ideas

1. **Sender declares, user decides.** Urgency and category from the app, final presentation from the user, unchangeable by the app afterwards ([channels](android-notification-channels.md)).
2. **Interruption levels, not colours.** Passive to the list, active pops, critical breaks through ([iOS](ios-interruption-levels.md), [freedesktop urgency](freedesktop-notification-spec.md)).
3. **Context triggers for quiet.** Fullscreen, mirroring, game, schedule ([Focus Assist](windows-focus-assist.md), [Plasma 5.17](kde-plasma-notifications.md)).
4. **Catch-up summary after quiet.** "What you missed" when the context ends ([Focus Assist](windows-focus-assist.md), [scheduled summary](ios-scheduled-summary.md)).
5. **Attenuate bursts.** First alert full, followers quieter, reset after silence ([cooldown](android-notification-cooldown.md)).
6. **Informative collapsed groups.** A summary line, children still actionable ([bundles](android-bundled-notifications.md)).
7. **Controls on the notification.** Silence this app or type from the card ([channels](android-notification-channels.md), [iOS 18.3](apple-intelligence-summaries.md)).
8. **Spatial continuity.** Windows travel from real positions to the overview and back ([Exposé](expose-mission-control.md)).
9. **Ambient goes achromatic when not the subject** ([Sonoma widgets](macos-desktop-widgets.md)).
10. **Tap-to-previous plus stable order** in the switcher ([switchers](alt-tab-switchers.md)).

## 10 instructive failures

| Failure | Cause |
|---|---|
| [Apple Intelligence summaries](apple-intelligence-summaries.md) | Synthesised text in the sender's slot |
| [Unity Shopping Lens](unity-shopping-lens.md) | Local action silently sent to the network |
| [Windows Recall](windows-recall.md) | Sensitive capture persisted, on by default |
| [Vista UAC](vista-uac.md) | Alert frequency destroyed alert value |
| [Clippy](clippy.md) | Proactive interruption on weak inference |
| [Windows 8 Start and Charms](windows-8-charms-start-screen.md) | Learned surface removed; invisible edge triggers |
| [Launchpad removal](launchpad-removal-tahoe.md) | Spatial memory replaced by search only |
| [Touch Bar](macbook-touch-bar.md) | Controls change with context, no muscle memory |
| [Sidebar gadgets](windows-sidebar-gadgets.md) / [Plasma store](plasma-widgets.md) | Third-party code in the shell |
| [KDE 4.0](kde-4-0-launch.md) / [GNOME 3.0](gnome-3-0-reception.md) | Rewrite shipped before parity |

**Common causes:** the vendor's goal outranked the user's attention (feeds, ads, assistants: also [Windows 11 Widgets](windows-11-widgets.md), [Cortana](cortana.md), [Copilot key](copilot-key.md)); surfaces whose meaning moves (Touch Bar, [Stage Manager](stage-manager.md), [Activities](kde-activities.md)' unclear scope); and trust spent faster than it was earned (Recall, summaries, UAC).

## Ideas for swaypplet (ranked)

| # | Idea | Size | Source |
|---|---|---|---|
| 1 | Roadmap item 3 as designed, with OR of three signals: mirrored outputs, active screen capture, fullscreen on the focused output. Panel names the trigger, one-click override. Never announce the start; announce the end only if something was held ("7 while you were presenting"). Also hold OSD popups during capture. | M | [presentation-mode-dnd](presentation-mode-dnd.md), [windows-focus-assist](windows-focus-assist.md), [kde-plasma-notifications](kde-plasma-notifications.md) |
| 2 | Urgency as interruption level: low goes to the list without a popup; critical breaks through quiet and never times out; per-app demotion of critical. | S | [ios-interruption-levels](ios-interruption-levels.md), [freedesktop-notification-spec](freedesktop-notification-spec.md) |
| 3 | Popup cooldown: a second notification from the same app within ~30 s updates that app's popup in place (count, latest line) rather than stacking; reset after silence. | S | [android-notification-cooldown](android-notification-cooldown.md), [android-bundled-notifications](android-bundled-notifications.md) |
| 4 | "Quiet this app" and "Quiet for 1 h" on the card (keyboard-reachable), stored in settings with a pane to undo. | S | [android-notification-channels](android-notification-channels.md), [macos-notification-center](macos-notification-center.md) |
| 5 | Collapsed group summary line built mechanically (count, distinct titles or senders); anything synthesised is styled apart from sender text. No LLM rewriting. | S | [apple-intelligence-summaries](apple-intelligence-summaries.md), [android-bundled-notifications](android-bundled-notifications.md) |
| 6 | Always show app name and icon on popups; group key `desktop-entry`, fallback `app_name`. | S | [gnome-shell-notifications](gnome-shell-notifications.md) |
| 7 | Overview (roadmap 4) keeps Exposé's continuity and shows every window flat; no per-app stacking, no perspective. | L | [expose-mission-control](expose-mission-control.md), [windows-flip-3d](windows-flip-3d.md) |
| 8 | Super+Tab: first tap goes to the previous workspace, browsing uses the stable workspace order; never reorder by recency. | S | [alt-tab-switchers](alt-tab-switchers.md), [virtual-desktops-spaces](virtual-desktops-spaces.md) |
| 9 | Pins and other peripheral surfaces go achromatic while a window has focus. | S | [macos-desktop-widgets](macos-desktop-widgets.md) |
| 10 | Launcher keeps an arranged, stable pinned grid next to search; no network results without opt-in. | S | [launchpad-removal-tahoe](launchpad-removal-tahoe.md), [unity-shopping-lens](unity-shopping-lens.md) |
| 11 | Workspace pictures and overview frames stay in memory, never on disk. | S | [windows-recall](windows-recall.md) |

**Confirmed rejections.** The record backs the ROADMAP's Rejected list: widgets and feeds ([Dashboard](macos-dashboard.md), [Widgets](windows-11-widgets.md)), a context-dependent bar segment ([Touch Bar](macbook-touch-bar.md)), an unread badge on the bar ([UAC](vista-uac.md)'s fatigue), and clipboard persistence ([Recall](windows-recall.md)). It also argues against a second grouping axis beside workspaces ([Activities](kde-activities.md)), against third-party plugins in the shell ([Plasma widgets](plasma-widgets.md)), and against named multi-Focus profiles before a single context-triggered quiet mode proves itself ([Focus](ios-focus-modes.md)).
