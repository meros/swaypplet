# Architectural Plan: In-Process Night Light & Magnifying Lens Color Dropper

## 1. Overview & Objectives
This plan covers two major shell advancements and a refreshed Start Menu / Quick Settings layout:
1. **Feature 1: In-Process Night Light & Display Profiles (`zwlr_gamma_control_v1`)**
   - Retires `gammastep` and `kanshi` as external systemd services.
   - Smooth gamma-ramp temperature adjustment (6500K → 3500K) directly inside `swaypplet`.
2. **Feature 6: Magnifying Lens Dropper (`swaypplet screenshot --color`)**
   - Replaces bare crosshair click with an interactive, zoomed `16×16` circular pixel loupe with live `#hex` / `rgb()` readout and keyboard copy shortcut (`C` or Click).
3. **Start Menu Layout Redesign:**
   - Adapts the Control Center grid to host dedicated **Night Light warmth controls**, **Display output profiles**, and a direct **Eyedropper tool shortcut** seamlessly in the Quick Settings column without layout crowding.

---

## 2. Start Menu Redesign (Rethought 2-Column Grid)

### Current Layout Issues:
- Sliders and quick-tiles are vertically stacked, causing the right column to become overly long when multiple reveals (Wi-Fi, Bluetooth, Displays) expand.
- Night Light was a simple toggle with no temperature ramp slider.

### Proposed Architecture:
```
╭───────────────────────────────────────────┬──────────────────────────────────────────╮
│  LEFT COLUMN (Launcher & Stage)           │  RIGHT COLUMN (Quick Controls & System)  │
│                                           │                                          │
│  [ 󰍉 Search apps & commands...          ] │  [ 󰕾 Volume Slider                      ]│
│                                           │  [ 󰃠 Brightness Slider                  ]│
│  • Ghostty Terminal                       │                                          │
│  • Google Chrome                          │  QUICK TILES (2x3 Grid):                 │
│  • VS Code                                │  ┌─────────────┬─────────────┐          │
│  • Files                                  │  │ 󰤨 Wi-Fi     │ 󰂯 Bluetooth │          │
│                                           │  ├─────────────┼─────────────┤          │
│  [ 󰈊 Color Dropper ] [ 󰄀 Snipping Tool ]  │  │ 󰖔 Night (K) │ 󰅶 Caffeine  │          │
│                                           │  ├─────────────┼─────────────┤          │
│                                           │  │ 󰍹 Displays  │ 󰂚 DND       │          │
│                                           │  └─────────────┴─────────────┘          │
│                                           │  [ Night Temp Warmth Slider (Revealed) ] │
│                                           │  [ Media Mini-Player (Auto-hiding)     ] │
│                                           │  [ Power & Battery Summary Card        ] │
╰───────────────────────────────────────────┴──────────────────────────────────────────╯
```

---

## 3. Detailed Component Blueprint

### A. Night Light (`src/gamma.rs` & `src/widgets/display.rs`)
- **Protocol:** Speaks `zwlr_gamma_control_manager_v1` via Wayland client bindings.
- **Ramp Calculation:**
  - Standard blackbody color temperature to RGB gamma ramp lookup table ($1000K \dots 10000K$).
  - Smooth 300ms gamma transitions between states (interpolating lookup tables).
- **Control Center Integration:**
  - Tile toggle: Turns Night Light on/off.
  - Sub-slider in `DisplaySection`: Adjusts target color temperature ($2500K \dots 6500K$).

### B. Color Dropper Loupe (`src/screenshot/select.rs`)
- **Interactive Lens HUD:**
  - When `Mode::Pick` is active, pointer movement tracks a floating $120\text{px}$ circular loupe with a $16\times 16$ magnified grid centered on the pointer.
  - Draws grid lines with high-contrast inverted crosshair targeting the center pixel.
  - Shows live hex string (`#ebdbb2`) in a Gruvbox badge above the lens.
  - Pressing `C` or clicking commits and copies hex to clipboard immediately.

---

## 4. Implementation Steps

1. **Step 1:** Create `docs/startmenu-zoo.html` showcasing the rethought Start Menu layout with Night Light slider and Eyedropper launcher action.
2. **Step 2:** Upgrade `src/screenshot/select.rs` to render the magnifying loupe in `Mode::Pick`.
3. **Step 3:** Implement native gamma temperature control in `src/widgets/tiles.rs` and `src/widgets/display.rs`.
4. **Step 4:** Re-build, test preview, commit, and update NixOS lock.
