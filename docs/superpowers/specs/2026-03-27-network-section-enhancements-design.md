# Network Section Enhancements

## Summary

Expand the network section from basic WiFi/VPN management into a comprehensive network control panel with 8 new features, while refactoring the monolithic `network.rs` (~1700 lines) into a clean module structure.

## Features

### 1. WiFi Radio Toggle
- Switch at top of detail area, label "WiFi", icon toggles between 󰤨 (on) and 󰤭 (off)
- `nmcli radio wifi on` / `nmcli radio wifi off`
- When off: hide available networks list, hotspot section, power saving toggle

### 2. Airplane Mode Toggle
- Switch next to WiFi radio toggle, label "Airplane"
- `nmcli radio all off` / `nmcli radio all on`
- When on: WiFi radio switch becomes insensitive (greyed out), WiFi scan hidden
- Toggling off restores previous radio states (NM handles natively)

### 3. Connectivity State Indicator
- Detail area: label showing "Connected" / "Limited — No internet" / "Captive portal" / "Disconnected"
- When portal: "Open portal" button runs `xdg-open` on NM's connectivity check URL
- Summary row: append badge " · ⚠ Portal" or " · ⚠ Limited" when not `Full`
- Source: `nmcli networking connectivity check`

### 4. DNS Info Display
- Read-only label below existing IP/gateway: "DNS: 1.1.1.1, 8.8.8.8"
- Source: `nmcli -t -f IP4.DNS device show <dev>`
- Hidden when no active connection

### 5. Connect to Hidden Network
- Button at bottom of available networks list: "Connect to hidden network"
- Click reveals: SSID entry + password entry + Connect button
- `nmcli device wifi connect <ssid> password <pass> hidden yes`
- Same feedback pattern (spinner + status label) as other connect flows

### 6. WiFi Hotspot Toggle
- Section below power saving, only visible when WiFi adapter supports AP mode
- Check AP support: `nmcli -t -f WIFI-PROPERTIES.AP device show <wifi-dev>`
- Off state: settings revealer with SSID field (default: hostname) + password field (default: random 8-char)
- Enable: `nmcli device wifi hotspot ssid <ssid> password <pass>`
- On state: show current SSID + password + "Copy password" button + "Stop" button
- Disable: `nmcli connection down <hotspot-conn-name>` (NM names it "Hotspot" by default; read actual name from `nmcli -t -f NAME,TYPE connection show --active` where type is `802-11-wireless` and device is in AP mode)

### 7. WiFi Power Management Toggle
- Single row with switch, label "WiFi Power Saving"
- Only visible when active connection is WiFi
- Read: `nmcli -t -f 802-11-wireless.powersave connection show <conn-name>`
- Toggle: `nmcli connection modify <conn> 802-11-wireless.powersave 2` (off) / `3` (on)

### 8. Periodic State Refresh
- `glib::timeout_add_local` at 5-second interval
- Each tick: `get_active_connection()` + `nmcli networking connectivity check` + `nmcli radio wifi`
- Compare with previous state; only trigger UI updates on change
- Does NOT trigger WiFi rescans (those remain manual/on-expand)
- Polls continuously (5s is cheap) so summary row stays current

## Architecture

### Module Structure

```
src/widgets/network/
  mod.rs          — NetworkSection struct, UI assembly, summary row, detail layout, refresh orchestration
  backend.rs      — Data types, all nmcli/ip commands, parsing helpers, NmResult enum
  wifi.rs         — WiFi list building, connect/forget flows, hidden network join form
  interfaces.rs   — Interface list with switches, WiFi radio toggle, airplane mode toggle
  vpn.rs          — VPN list building, connect/disconnect
  hotspot.rs      — Hotspot toggle UI, SSID/password config revealer
  monitor.rs      — 5s periodic state poller, connectivity state checker
```

### Data Types (backend.rs)

Existing types preserved: `WifiNetwork`, `ActiveConnection`, `VpnConnection`, `NetworkInterface`.

New types:

```rust
enum ConnectivityState { Full, Limited, Portal, None, Unknown }

struct HotspotState {
    active: bool,
    ssid: String,
    password: String,
}

enum NmResult {
    Success,
    Failure(String),  // stderr from nmcli
}

struct NetworkState {
    active: ActiveConnection,
    connectivity: ConnectivityState,
    networks: Vec<WifiNetwork>,
    vpns: Vec<VpnConnection>,
    interfaces: Vec<NetworkInterface>,
    hotspot: Option<HotspotState>,
    wifi_radio_enabled: bool,
    wifi_power_saving: bool,
    list_visible: bool,
    show_all: bool,
    scanning: bool,
}
```

### Threading Pattern

All async nmcli calls use `mpsc::channel::<NmResult>()` with `glib::timeout_add_local` + `rx.try_recv()` polling (matching bluetooth.rs pattern). Replaces the `Arc<Mutex<Option<bool>>>` pattern throughout.

### UI Layout (detail area, top to bottom)

1. Radio toggles row — WiFi switch + Airplane mode switch (horizontal bar)
2. Current connection row — icon + SSID + signal/band (existing)
3. Connectivity state row — status label + "Open portal" button when applicable
4. IP info rows — IP + gateway (existing) + DNS servers (new)
5. WiFi power management toggle — switch, WiFi-only
6. Hotspot section — toggle + settings revealer, WiFi AP-capable only
7. Interfaces subsection — per-interface switches (existing)
8. Available Networks toggle + list — existing + hidden network button at bottom
9. VPN subsection — existing

## Error Handling

- `NmResult::Failure(msg)` shows actual nmcli error text in status labels (not just "Failed")
- Status labels auto-hide after 4 seconds
- Hotspot enable failure: show error in-place, re-enable toggle

## Edge Cases

- **No WiFi adapter**: hide WiFi radio toggle, hotspot, power saving, hidden network. Show only interfaces + ethernet + VPN
- **nmcli unavailable**: early return with placeholder (existing, unchanged)
- **Hotspot AP unsupported**: check `WIFI-PROPERTIES.AP` — hide hotspot if "no"
- **Connectivity check not configured**: returns "unknown" — show "Unknown", no portal button
- **Power saving on ethernet**: hide toggle (only shown for active WiFi)
- **Multiple WiFi adapters**: airplane mode affects all. Interface list shows all. Hotspot uses first available
- **Summary row when WiFi radio off**: show 󰤭 icon + "WiFi Off"
- **Summary row when airplane mode on**: show "Airplane Mode"

## Out of Scope

- DNS editing (read-only only)
- Static IP / DHCP toggle
- MAC randomization toggle
- Bandwidth monitoring
- Connection priority management
- Proxy settings
