# Theming, colour, materials and settings: prior art

**swaypplet's generator is ahead of every shipping desktop on contrast guarantees. Its gap is reach: nothing outside the shell learns its mode, accent or palette.** 50 entries, researched 2026-09-26. Each file marks its sources [verified] or [memory].

## Landscape

Four currents meet here.

- **Generated colour.** Material You ([material-you-dynamic-color](material-you-dynamic-color.md), [hct-cam16](hct-cam16.md)), Leonardo ([adobe-leonardo](adobe-leonardo.md)) and Radix ([radix-colors](radix-colors.md)) replaced hand-picked palettes with functions: a hue in, roles with known contrast out. The perceptual spaces ([oklab-oklch](oklab-oklch.md)) and contrast models ([apca](apca.md)) under them are now mainstream, but APCA has no standing in WCAG.
- **Materials.** Apple ([apple-vibrancy-materials](apple-vibrancy-materials.md), [apple-liquid-glass](apple-liquid-glass.md)) and Fluent ([fluent-mica](fluent-mica.md), [fluent-acrylic](fluent-acrylic.md)) each learned that translucency needs an opaque escape hatch and a placement rule.
- **Linux theming.** It went from arbitrary stylesheets ([gtk-theme-era](gtk-theme-era.md)) through a backlash ([stop-theming-my-app](stop-theming-my-app.md)) to a few supported inputs: [libadwaita](libadwaita.md), [gnome-accent-colors](gnome-accent-colors.md), the [portal keys](xdg-portal-appearance.md). Meanwhile ricers built wallpaper pipelines ([pywal](pywal.md), [wallust](wallust.md), [matugen](matugen.md)) and Nix got declarative theming ([stylix](stylix.md), [base16](base16-tinted-theming.md)).
- **Settings apps.** They swing between minimal ([gnome-settings](gnome-settings.md)) and exhaustive ([kde-system-settings](kde-system-settings.md)). The redesigns that failed did so on findability ([macos-system-settings](macos-system-settings.md), [windows-settings-control-panel](windows-settings-control-panel.md)).

## 10 best ideas

1. **Hue from the image, tone from the role.** Material, KDE and swaypplet all do this ([material-you-dynamic-color](material-you-dynamic-color.md)).
2. **Contrast as the input, colour as the output.** Leonardo and M3 contrast levels solve for tones ([adobe-leonardo](adobe-leonardo.md), [m3-contrast-levels](m3-contrast-levels.md)).
3. **Steps with jobs that hold in both modes** ([radix-colors](radix-colors.md)).
4. **A cross-desktop preference bus** for scheme, accent, contrast and motion ([xdg-portal-appearance](xdg-portal-appearance.md)).
5. **Search that lands on the row**, highlighted, from launcher or settings ([settings-search](settings-search.md)).
6. **Highlight changed settings** against defaults ([kde-system-settings](kde-system-settings.md)).
7. **A switch that waits until you are not looking** ([macos-auto-appearance](macos-auto-appearance.md), [windows-auto-dark-mode](windows-auto-dark-mode.md)).
8. **Colour-vision themes as real modes** ([github-primer](github-primer.md)).
9. **Materials with an opaque fallback and a placement rule**: Mica for long-lived surfaces, Acrylic for transient ones ([fluent-mica](fluent-mica.md)).
10. **One palette, many app templates** ([matugen](matugen.md), [stylix](stylix.md), [base16-tinted-theming](base16-tinted-theming.md)).

## 8 instructive failures

1. pywal takes lightness from the image and assigns slots by order, so red can be green ([pywal](pywal.md)).
2. GTK3 themes broke with every minor release because the stylesheet was the API ([gtk-theme-era](gtk-theme-era.md)).
3. At launch Liquid Glass shipped legibility second, and Reduce Transparency broke in 26.1–26.2 ([apple-liquid-glass](apple-liquid-glass.md)).
4. After a decade, Windows still splits settings between two apps ([windows-settings-control-panel](windows-settings-control-panel.md)).
5. Ventura's settings turned into long flat lists and lost users' spatial memory of the old grid ([macos-system-settings](macos-system-settings.md)).
6. Windows' automatic accent picks muddy colours from dark wallpapers ([windows-accent-from-wallpaper](windows-accent-from-wallpaper.md)).
7. Polaris deferred dark mode until components assumed a light surface ([shopify-polaris](shopify-polaris.md)).
8. Home Manager's dconf overwrites what the user changed at runtime ([gsettings-dconf-home-manager](gsettings-dconf-home-manager.md)).

## Ideas for swaypplet, ranked

| # | Idea | Size | Source |
|---|---|---|---|
| 1 | Publish mode, accent, contrast and reduced motion to gsettings and the portal, in the same deferred step as the token reload. Today `scaling.nix` pins apps to `prefer-dark`, so in light mode the apps stay dark. | S | [xdg-portal-appearance](xdg-portal-appearance.md), [gnome-accent-colors](gnome-accent-colors.md) |
| 2 | Index every settings row by key, title and keywords. The omnibox returns rows and opens the pane on the row, highlighted. | M | [settings-search](settings-search.md) |
| 3 | Export the tokens per mode as base16/matugen JSON, a libadwaita `gtk.css`, a Kvantum/qt6ct palette and ANSI-16, so terminals, Qt and GTK apps follow. | M | [matugen](matugen.md), [stylix](stylix.md), [libadwaita](libadwaita.md) |
| 4 | Solve the text alphas against the glass-aware APCA target instead of tabulating them. The test then cannot fail, and a continuous contrast level comes for free. | M | [adobe-leonardo](adobe-leonardo.md), [m3-contrast-levels](m3-contrast-levels.md) |
| 5 | Mark rows the user file overrides and reset them per row. | S | [kde-system-settings](kde-system-settings.md) |
| 6 | Colour-vision status presets (blue/orange for success/danger). | S | [github-primer](github-primer.md) |
| 7 | A manual mode that holds until the next sun crossing, then returns to auto. A day/night wallpaper pair. | S | [windows-auto-dark-mode](windows-auto-dark-mode.md), [macos-auto-appearance](macos-auto-appearance.md) |
| 8 | A user-facing Reduce Transparency input that selects `.no-glass` everywhere. Glass off on battery saver. | S | [macos-reduce-transparency-increase-contrast](macos-reduce-transparency-increase-contrast.md), [fluent-mica](fluent-mica.md) |
| 9 | Offer 3–4 hues from the palette as accent picks, not only the primary. | S | [material-you-dynamic-color](material-you-dynamic-color.md) |
| 10 | Nix-side locks that show a section read-only; machine-local vs portable sections for "Copy as Nix". | S | [gsettings-dconf-home-manager](gsettings-dconf-home-manager.md), [settings-sync](settings-sync.md) |

## Ahead of and behind the state of the art

**Ahead**

- Contrast is tested *through the material*: 10,440 token sets at APCA thresholds. No shipping system, Apple and Microsoft included, publishes a guarantee over its translucent material.
- Tint rules that are bounded (status ±12°, a neutral cast held at chroma 0.010–0.025) and tested. KDE's tint slider and pywal have neither.
- Auto mode uses a solar-elevation hysteresis band and defers the switch until you are not looking. It shares its location with a night light that ramps in mired. Only macOS matches the deferral.
- A lint enforces the semantic tier; only Atlassian's tooling comes close.
- The settings layers (binary, Nix, user) with "absent means default" avoid Home Manager's overwrite problem.

**Behind**

- **Reach.** Every major desktop exports its preference (portal, gsettings, KDE colour schemes); swaypplet exports only sway borders.
- **Findability.** Android, ChromeOS, GNOME and KDE all search individual settings.
- **Accessibility breadth.** swaypplet lacks forced colours ([windows-contrast-themes](windows-contrast-themes.md)), colour-vision modes and a user-facing transparency switch.
- **Contrast is two tables, not a function.** Material and Leonardo solve for it.
- **No interchange format** ([dtcg-design-tokens](dtcg-design-tokens.md)): the palette cannot leave Rust except as CSS.
