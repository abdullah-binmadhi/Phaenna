//! Linux Wayland compositor Picture-in-Picture (PiP) & Fullscreen controller.
//!
//! macOS, Windows, and X11 honor `mpv`'s `ontop`, `border=no`, `window-scale`, and `geometry`
//! properties directly. Under native Wayland, `xdg-shell` forbids clients from positioning
//! themselves or pinning above other windows, and tiling compositors (Hyprland, Sway) tile new
//! windows by default and block fullscreen on pinned windows (`binds:allow_pin_fullscreen=false`).
//! This module bridges Phaenna's PiP and fullscreen state to Hyprland (`hyprctl`) and Sway
//! (`swaymsg`) at runtime by matching the exact `mpv` PID tree and window address so sibling
//! terminal windows are never touched.

use serde::Deserialize;
use std::collections::HashSet;
use std::sync::OnceLock;

/// Wayland `app_id` / X11 class assigned to Phaenna-managed `mpv` windows on Linux.
pub const PHAENNA_WAYLAND_APP_ID: &str = "phaenna-mpv";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipAnchor {
    BottomRight,
    TopRight,
    BottomLeft,
    TopLeft,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PipCornerPreset {
    pub label: &'static str,
    pub scale: f64,
    pub anchor: PipAnchor,
    pub margin: i32,
    pub mpv_geometry: &'static str,
}

/// Unified 6-preset PiP table shared across `mpv` flags, IPC commands, Lua overlay, and Wayland.
pub const PIP_PRESETS: [PipCornerPreset; 6] = [
    PipCornerPreset {
        label: "Bottom-Right 30%",
        scale: 0.30,
        anchor: PipAnchor::BottomRight,
        margin: 24,
        mpv_geometry: "30%x30%-24-24",
    },
    PipCornerPreset {
        label: "Top-Right 30%",
        scale: 0.30,
        anchor: PipAnchor::TopRight,
        margin: 24,
        mpv_geometry: "30%x30%-24+24",
    },
    PipCornerPreset {
        label: "Bottom-Left 30%",
        scale: 0.30,
        anchor: PipAnchor::BottomLeft,
        margin: 24,
        mpv_geometry: "30%x30%+24-24",
    },
    PipCornerPreset {
        label: "Top-Left 30%",
        scale: 0.30,
        anchor: PipAnchor::TopLeft,
        margin: 24,
        mpv_geometry: "30%x30%+24+24",
    },
    PipCornerPreset {
        label: "Compact 22%",
        scale: 0.22,
        anchor: PipAnchor::BottomRight,
        margin: 20,
        mpv_geometry: "22%x22%-20-20",
    },
    PipCornerPreset {
        label: "Theater 38%",
        scale: 0.38,
        anchor: PipAnchor::BottomRight,
        margin: 28,
        mpv_geometry: "38%x38%-28-28",
    },
];

pub const PIP_CORNER_LABELS: [&str; 6] = [
    "Bottom-Right 30%",
    "Top-Right 30%",
    "Bottom-Left 30%",
    "Top-Left 30%",
    "Compact 22%",
    "Theater 38%",
];

pub fn pip_preset(idx: usize) -> &'static PipCornerPreset {
    &PIP_PRESETS[idx % PIP_PRESETS.len()]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonitorGeometry {
    pub x: i32,
    pub y: i32,
    pub phys_width: i32,
    pub phys_height: i32,
    pub scale: f64,
    pub transform: i32,
    /// `[left, top, right, bottom]` reserved panel insets in logical pixels.
    pub reserved: [i32; 4],
}

/// Compute logical 16:9 floating PiP coordinates respecting HiDPI scaling, rotation, and panels.
pub fn compute_pip_rect(mon: MonitorGeometry, preset: &PipCornerPreset) -> PipRect {
    let scale = if mon.scale.is_finite() && mon.scale > 0.1 {
        mon.scale
    } else {
        1.0
    };
    let (raw_w, raw_h) = if mon.transform.rem_euclid(2) == 1 {
        (mon.phys_height, mon.phys_width)
    } else {
        (mon.phys_width, mon.phys_height)
    };
    let logical_w = ((raw_w as f64) / scale).round() as i32;
    let logical_h = ((raw_h as f64) / scale).round() as i32;

    let [res_left, res_top, res_right, res_bottom] = mon.reserved;
    let usable_x = mon.x + res_left.max(0);
    let usable_y = mon.y + res_top.max(0);
    let usable_w = (logical_w - res_left.max(0) - res_right.max(0)).max(320);
    let usable_h = (logical_h - res_top.max(0) - res_bottom.max(0)).max(180);

    let mut pip_w = ((usable_w as f64) * preset.scale).round() as i32;
    let mut pip_h = ((pip_w as f64) * 9.0 / 16.0).round() as i32;

    // On portrait/tall monitors, cap PiP height to 65% of usable height while keeping 16:9.
    let max_h = ((usable_h as f64) * 0.65).round() as i32;
    if pip_h > max_h && max_h >= 120 {
        pip_h = max_h;
        pip_w = ((pip_h as f64) * 16.0 / 9.0).round() as i32;
    }

    pip_w = pip_w.clamp(240, usable_w.saturating_sub(16).max(240));
    pip_h = pip_h.clamp(135, usable_h.saturating_sub(16).max(135));

    let margin = preset
        .margin
        .min((usable_w - pip_w) / 2)
        .min((usable_h - pip_h) / 2)
        .max(0);
    let (x, y) = match preset.anchor {
        PipAnchor::BottomRight => (
            usable_x + usable_w - pip_w - margin,
            usable_y + usable_h - pip_h - margin,
        ),
        PipAnchor::BottomLeft => (
            usable_x + margin,
            usable_y + usable_h - pip_h - margin,
        ),
        PipAnchor::TopRight => (
            usable_x + usable_w - pip_w - margin,
            usable_y + margin,
        ),
        PipAnchor::TopLeft => (usable_x + margin, usable_y + margin),
    };

    PipRect {
        x,
        y,
        width: pip_w,
        height: pip_h,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinuxPipBackend {
    Hyprland,
    Sway,
    UnsupportedWayland(String),
    X11OrNative,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipApplyOutcome {
    NotNeeded,
    Applied,
    WindowNotFound,
    UnsupportedCompositor(String),
    Failed(String),
}

pub fn classify_linux_pip_backend(
    has_hypr_sig: bool,
    has_hyprctl: bool,
    has_swaysock: bool,
    has_swaymsg: bool,
    has_wayland_display: bool,
    desktop_name: &str,
) -> LinuxPipBackend {
    if has_hypr_sig && has_hyprctl {
        LinuxPipBackend::Hyprland
    } else if has_swaysock && has_swaymsg {
        LinuxPipBackend::Sway
    } else if has_wayland_display {
        let label = if desktop_name.trim().is_empty() {
            "Wayland".to_string()
        } else {
            desktop_name.trim().to_string()
        };
        LinuxPipBackend::UnsupportedWayland(label)
    } else {
        LinuxPipBackend::X11OrNative
    }
}

pub fn detect_linux_pip_backend() -> LinuxPipBackend {
    #[cfg(not(target_os = "linux"))]
    {
        LinuxPipBackend::X11OrNative
    }
    #[cfg(target_os = "linux")]
    {
        let has_hypr_sig = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")
            .is_some_and(|v| !v.is_empty());
        let has_swaysock = std::env::var_os("SWAYSOCK").is_some_and(|v| !v.is_empty());
        let has_wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty());
        let has_hyprctl = has_hypr_sig && crate::api::player::find_binary("hyprctl").is_some();
        let has_swaymsg = has_swaysock && crate::api::player::find_binary("swaymsg").is_some();
        let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
        classify_linux_pip_backend(
            has_hypr_sig,
            has_hyprctl,
            has_swaysock,
            has_swaymsg,
            has_wayland,
            &desktop,
        )
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct HyprWorkspaceRef {
    #[serde(default)]
    pub id: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct HyprClient {
    pub address: String,
    #[serde(default)]
    pub mapped: bool,
    #[serde(default)]
    pub floating: bool,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub fullscreen: i32,
    #[serde(default)]
    pub monitor: i64,
    #[serde(default)]
    pub workspace: HyprWorkspaceRef,
    #[serde(default)]
    pub class: String,
    #[serde(default)]
    pub pid: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HyprMonitor {
    pub id: i64,
    pub width: i32,
    pub height: i32,
    #[serde(default)]
    pub x: i32,
    #[serde(default)]
    pub y: i32,
    #[serde(default = "default_scale")]
    pub scale: f64,
    #[serde(default)]
    pub transform: i32,
    #[serde(default)]
    pub focused: bool,
    #[serde(default)]
    pub reserved: [i32; 4],
}

fn default_scale() -> f64 {
    1.0
}

impl From<&HyprMonitor> for MonitorGeometry {
    fn from(m: &HyprMonitor) -> Self {
        Self {
            x: m.x,
            y: m.y,
            phys_width: m.width,
            phys_height: m.height,
            scale: m.scale,
            transform: m.transform,
            reserved: m.reserved,
        }
    }
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn collect_pid_and_descendants(root_pid: u32) -> HashSet<u32> {
    let mut pids = HashSet::new();
    pids.insert(root_pid);
    #[cfg(target_os = "linux")]
    {
        let mut queue = vec![root_pid];
        while let Some(pid) = queue.pop() {
            let task_dir = format!("/proc/{pid}/task");
            let Ok(entries) = std::fs::read_dir(task_dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let children_path = entry.path().join("children");
                let Ok(txt) = std::fs::read_to_string(children_path) else {
                    continue;
                };
                for tok in txt.split_whitespace() {
                    if let Ok(child) = tok.parse::<u32>() {
                        if pids.insert(child) {
                            queue.push(child);
                        }
                    }
                }
            }
        }
    }
    pids
}

/// Match the `mpv` window strictly by PID tree (or Phaenna's Wayland app_id), never touching terminals.
pub fn find_hypr_target_client<'a>(
    clients: &'a [HyprClient],
    pids: &HashSet<u32>,
) -> Option<&'a HyprClient> {
    clients
        .iter()
        .find(|c| c.mapped && !c.address.is_empty() && pids.contains(&c.pid))
        .or_else(|| {
            clients.iter().find(|c| {
                c.mapped
                    && !c.address.is_empty()
                    && (c.class.eq_ignore_ascii_case(PHAENNA_WAYLAND_APP_ID)
                        || c.class.eq_ignore_ascii_case("phaenna-pip"))
            })
        })
}

pub fn resolve_hypr_monitor_geometry(
    monitors: &[HyprMonitor],
    monitor_id: i64,
) -> Option<MonitorGeometry> {
    monitors
        .iter()
        .find(|m| m.id == monitor_id)
        .or_else(|| monitors.iter().find(|m| m.focused))
        .or_else(|| monitors.first())
        .map(MonitorGeometry::from)
}

/// Build the Lua script for `hyprctl eval` on Hyprland >= 0.57 using exact string address selectors.
pub fn build_hypr_lua_script(
    client: &HyprClient,
    enabled: bool,
    rect: Option<PipRect>,
) -> Option<String> {
    let addr = format!("address:{}", client.address);
    let mut stmts: Vec<String> = Vec::new();
    if enabled {
        let r = rect?;
        if client.fullscreen != 0 {
            stmts.push(format!(
                "hl.dispatch(hl.dsp.window.fullscreen({{ window = '{addr}', action = 'unset' }}))"
            ));
        }
        if !client.floating {
            stmts.push(format!(
                "hl.dispatch(hl.dsp.window.float({{ window = '{addr}', action = 'set' }}))"
            ));
        }
        stmts.push(format!(
            "hl.dispatch(hl.dsp.window.resize({{ window = '{addr}', x = {}, y = {}, relative = false }}))",
            r.width, r.height
        ));
        stmts.push(format!(
            "hl.dispatch(hl.dsp.window.move({{ window = '{addr}', x = {}, y = {}, relative = false }}))",
            r.x, r.y
        ));
        if !client.pinned {
            stmts.push(format!(
                "hl.dispatch(hl.dsp.window.pin({{ window = '{addr}', action = 'set' }}))"
            ));
        }
    } else {
        if client.pinned {
            stmts.push(format!(
                "hl.dispatch(hl.dsp.window.pin({{ window = '{addr}', action = 'unset' }}))"
            ));
        }
        if client.floating {
            stmts.push(format!(
                "hl.dispatch(hl.dsp.window.float({{ window = '{addr}', action = 'unset' }}))"
            ));
        }
    }

    if stmts.is_empty() {
        None
    } else {
        Some(stmts.join("; "))
    }
}

/// Build Lua script for `hyprctl eval` to cleanly unpin/unfloat before entering or leaving fullscreen.
pub fn build_hypr_fullscreen_lua_script(client: &HyprClient, enable: bool) -> Option<String> {
    let addr = format!("address:{}", client.address);
    let mut stmts: Vec<String> = Vec::new();
    if enable {
        if client.pinned {
            stmts.push(format!(
                "hl.dispatch(hl.dsp.window.pin({{ window = '{addr}', action = 'unset' }}))"
            ));
        }
        if client.floating {
            stmts.push(format!(
                "hl.dispatch(hl.dsp.window.float({{ window = '{addr}', action = 'unset' }}))"
            ));
        }
        if client.fullscreen == 0 {
            stmts.push(format!(
                "hl.dispatch(hl.dsp.window.fullscreen({{ window = '{addr}', action = 'set' }}))"
            ));
        }
    } else if client.fullscreen != 0 {
        stmts.push(format!(
            "hl.dispatch(hl.dsp.window.fullscreen({{ window = '{addr}', action = 'unset' }}))"
        ));
    }

    if stmts.is_empty() {
        None
    } else {
        Some(stmts.join("; "))
    }
}

/// Build legacy `hyprctl --batch` command string for Hyprland < 0.57.
pub fn build_hypr_legacy_batch(
    client: &HyprClient,
    enabled: bool,
    rect: Option<PipRect>,
) -> Option<String> {
    let addr = format!("address:{}", client.address);
    let mut cmds: Vec<String> = Vec::new();
    if enabled {
        let r = rect?;
        if client.fullscreen != 0 {
            cmds.push(format!("dispatch fullscreen 0,{addr}"));
        }
        if !client.floating {
            cmds.push(format!("dispatch setfloating {addr}"));
        }
        cmds.push(format!(
            "dispatch resizewindowpixel exact {} {},{addr}",
            r.width, r.height
        ));
        cmds.push(format!(
            "dispatch movewindowpixel exact {} {},{addr}",
            r.x, r.y
        ));
        if !client.pinned {
            cmds.push(format!("dispatch pin {addr}"));
        }
    } else {
        if client.pinned {
            cmds.push(format!("dispatch pin {addr}"));
        }
        if client.floating {
            cmds.push(format!("dispatch settiled {addr}"));
        }
    }

    if cmds.is_empty() {
        None
    } else {
        Some(cmds.join("; "))
    }
}

/// Build legacy `hyprctl --batch` command string for Hyprland < 0.57 fullscreen toggling.
pub fn build_hypr_legacy_fullscreen_batch(client: &HyprClient, enable: bool) -> Option<String> {
    let addr = format!("address:{}", client.address);
    let mut cmds: Vec<String> = Vec::new();
    if enable {
        if client.pinned {
            cmds.push(format!("dispatch pin {addr}"));
        }
        if client.floating {
            cmds.push(format!("dispatch settiled {addr}"));
        }
        if client.fullscreen == 0 {
            cmds.push(format!("dispatch fullscreen 0,{addr}"));
        }
    } else if client.fullscreen != 0 {
        cmds.push(format!("dispatch fullscreen 0,{addr}"));
    }

    if cmds.is_empty() {
        None
    } else {
        Some(cmds.join("; "))
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SwayNode {
    pub id: i64,
    #[serde(default)]
    pub pid: Option<u32>,
    #[serde(default)]
    pub app_id: Option<String>,
    #[serde(default)]
    pub fullscreen_mode: i32,
    #[serde(default)]
    pub nodes: Vec<SwayNode>,
    #[serde(default)]
    pub floating_nodes: Vec<SwayNode>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SwayOutputRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SwayOutput {
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub focused: bool,
    pub rect: SwayOutputRect,
}

pub fn find_sway_target_con_id(node: &SwayNode, pids: &HashSet<u32>) -> Option<i64> {
    if node.pid.is_some_and(|p| pids.contains(&p)) {
        return Some(node.id);
    }
    if node
        .app_id
        .as_deref()
        .is_some_and(|a| {
            a.eq_ignore_ascii_case(PHAENNA_WAYLAND_APP_ID)
                || a.eq_ignore_ascii_case("phaenna-pip")
        })
    {
        return Some(node.id);
    }
    for child in node.floating_nodes.iter().chain(node.nodes.iter()) {
        if let Some(id) = find_sway_target_con_id(child, pids) {
            return Some(id);
        }
    }
    None
}

pub fn resolve_sway_monitor_geometry(outputs: &[SwayOutput]) -> Option<MonitorGeometry> {
    outputs
        .iter()
        .find(|o| o.active && o.focused)
        .or_else(|| outputs.iter().find(|o| o.active))
        .or_else(|| outputs.first())
        .map(|o| MonitorGeometry {
            x: o.rect.x,
            y: o.rect.y,
            phys_width: o.rect.width,
            phys_height: o.rect.height,
            scale: 1.0,
            transform: 0,
            reserved: [0, 0, 0, 0],
        })
}

pub fn build_sway_pip_command(con_id: i64, enabled: bool, rect: Option<PipRect>) -> Option<String> {
    if enabled {
        let r = rect?;
        Some(format!(
            "[con_id={con_id}] fullscreen disable, floating enable, sticky enable, resize set {} px {} px, move position {} px {} px",
            r.width, r.height, r.x, r.y
        ))
    } else {
        Some(format!(
            "[con_id={con_id}] sticky disable, floating disable"
        ))
    }
}

pub fn build_sway_fullscreen_command(con_id: i64, enable: bool) -> String {
    if enable {
        format!("[con_id={con_id}] sticky disable, floating disable, fullscreen enable")
    } else {
        format!("[con_id={con_id}] fullscreen disable")
    }
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
static HYPR_USES_LUA_EVAL: OnceLock<bool> = OnceLock::new();

#[cfg(target_os = "linux")]
async fn hypr_supports_lua_eval() -> bool {
    if let Some(&cached) = HYPR_USES_LUA_EVAL.get() {
        return cached;
    }
    let supported = match tokio::process::Command::new("hyprctl")
        .args(["eval", "return 1"])
        .output()
        .await
    {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            out.status.success() && stdout.trim() == "ok"
        }
        Err(_) => false,
    };
    let _ = HYPR_USES_LUA_EVAL.set(supported);
    supported
}

#[cfg(target_os = "linux")]
async fn hyprctl_json<T: for<'de> Deserialize<'de>>(subcmd: &str) -> Result<T, String> {
    let out = tokio::process::Command::new("hyprctl")
        .args(["-j", subcmd])
        .output()
        .await
        .map_err(|e| format!("Failed to invoke hyprctl -j {subcmd}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "hyprctl -j {subcmd} exited with {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("Failed to parse hyprctl -j {subcmd}: {e}"))
}

#[cfg(target_os = "linux")]
async fn find_mapped_hypr_client(
    player_pid: Option<u32>,
    wait_for_map: bool,
) -> Option<HyprClient> {
    let max_attempts = if wait_for_map { 28 } else { 3 };
    for attempt in 0..max_attempts {
        let pids = player_pid
            .map(collect_pid_and_descendants)
            .unwrap_or_default();
        if let Ok(clients) = hyprctl_json::<Vec<HyprClient>>("clients").await {
            if let Some(found) = find_hypr_target_client(&clients, &pids) {
                return Some(found.clone());
            }
        }
        if attempt + 1 < max_attempts {
            tokio::time::sleep(std::time::Duration::from_millis(90)).await;
        }
    }
    None
}

#[cfg(target_os = "linux")]
async fn exec_hypr_script_or_batch(
    lua_script: Option<String>,
    legacy_batch: Option<String>,
) -> PipApplyOutcome {
    if hypr_supports_lua_eval().await {
        let Some(script) = lua_script else {
            return PipApplyOutcome::Applied;
        };
        let out = tokio::process::Command::new("hyprctl")
            .args(["eval", &script])
            .output()
            .await;
        match out {
            Ok(o) if o.status.success() => {
                let stdout = String::from_utf8_lossy(&o.stdout);
                if stdout.trim_start().starts_with("error:") {
                    PipApplyOutcome::Failed(stdout.trim().to_string())
                } else {
                    PipApplyOutcome::Applied
                }
            }
            Ok(o) => PipApplyOutcome::Failed(format!(
                "hyprctl eval failed: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            )),
            Err(e) => PipApplyOutcome::Failed(format!("Failed to execute hyprctl eval: {e}")),
        }
    } else {
        let Some(batch) = legacy_batch else {
            return PipApplyOutcome::Applied;
        };
        let out = tokio::process::Command::new("hyprctl")
            .args(["--batch", &batch])
            .output()
            .await;
        match out {
            Ok(o) if o.status.success() => PipApplyOutcome::Applied,
            Ok(o) => PipApplyOutcome::Failed(format!(
                "hyprctl --batch failed: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            )),
            Err(e) => PipApplyOutcome::Failed(format!("Failed to execute hyprctl --batch: {e}")),
        }
    }
}

#[cfg(target_os = "linux")]
async fn apply_hyprland_pip(
    enabled: bool,
    corner_idx: usize,
    player_pid: Option<u32>,
    wait_for_map: bool,
) -> PipApplyOutcome {
    let Some(client) = find_mapped_hypr_client(player_pid, wait_for_map).await else {
        return PipApplyOutcome::WindowNotFound;
    };

    let rect = if enabled {
        let monitors = match hyprctl_json::<Vec<HyprMonitor>>("monitors").await {
            Ok(m) => m,
            Err(e) => return PipApplyOutcome::Failed(e),
        };
        let Some(mon) = resolve_hypr_monitor_geometry(&monitors, client.monitor) else {
            return PipApplyOutcome::Failed("No active Hyprland monitor found".to_string());
        };
        Some(compute_pip_rect(mon, pip_preset(corner_idx)))
    } else {
        None
    };

    exec_hypr_script_or_batch(
        build_hypr_lua_script(&client, enabled, rect),
        build_hypr_legacy_batch(&client, enabled, rect),
    )
    .await
}

#[cfg(target_os = "linux")]
async fn apply_hyprland_fullscreen(
    enable: bool,
    player_pid: Option<u32>,
    wait_for_map: bool,
) -> PipApplyOutcome {
    let Some(client) = find_mapped_hypr_client(player_pid, wait_for_map).await else {
        return PipApplyOutcome::WindowNotFound;
    };

    exec_hypr_script_or_batch(
        build_hypr_fullscreen_lua_script(&client, enable),
        build_hypr_legacy_fullscreen_batch(&client, enable),
    )
    .await
}

#[cfg(target_os = "linux")]
async fn swaymsg_json<T: for<'de> Deserialize<'de>>(msg_type: &str) -> Result<T, String> {
    let out = tokio::process::Command::new("swaymsg")
        .args(["-t", msg_type])
        .output()
        .await
        .map_err(|e| format!("Failed to invoke swaymsg -t {msg_type}: {e}"))?;
    if !out.status.success() {
        return Err(format!("swaymsg -t {msg_type} exited with {}", out.status));
    }
    serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("Failed to parse swaymsg -t {msg_type}: {e}"))
}

#[cfg(target_os = "linux")]
async fn find_sway_con(player_pid: Option<u32>, wait_for_map: bool) -> Option<i64> {
    let max_attempts = if wait_for_map { 28 } else { 3 };
    for attempt in 0..max_attempts {
        let pids = player_pid
            .map(collect_pid_and_descendants)
            .unwrap_or_default();
        if let Ok(tree) = swaymsg_json::<SwayNode>("get_tree").await {
            if let Some(id) = find_sway_target_con_id(&tree, &pids) {
                return Some(id);
            }
        }
        if attempt + 1 < max_attempts {
            tokio::time::sleep(std::time::Duration::from_millis(90)).await;
        }
    }
    None
}

#[cfg(target_os = "linux")]
async fn apply_sway_pip(
    enabled: bool,
    corner_idx: usize,
    player_pid: Option<u32>,
    wait_for_map: bool,
) -> PipApplyOutcome {
    let Some(con_id) = find_sway_con(player_pid, wait_for_map).await else {
        return PipApplyOutcome::WindowNotFound;
    };

    let rect = if enabled {
        let outputs = match swaymsg_json::<Vec<SwayOutput>>("get_outputs").await {
            Ok(o) => o,
            Err(e) => return PipApplyOutcome::Failed(e),
        };
        let Some(mon) = resolve_sway_monitor_geometry(&outputs) else {
            return PipApplyOutcome::Failed("No active Sway output found".to_string());
        };
        Some(compute_pip_rect(mon, pip_preset(corner_idx)))
    } else {
        None
    };

    let Some(cmd) = build_sway_pip_command(con_id, enabled, rect) else {
        return PipApplyOutcome::Applied;
    };

    match tokio::process::Command::new("swaymsg")
        .arg(&cmd)
        .output()
        .await
    {
        Ok(o) if o.status.success() => PipApplyOutcome::Applied,
        Ok(o) => PipApplyOutcome::Failed(format!(
            "swaymsg failed: {}",
            String::from_utf8_lossy(&o.stderr).trim()
        )),
        Err(e) => PipApplyOutcome::Failed(format!("Failed to execute swaymsg: {e}")),
    }
}

#[cfg(target_os = "linux")]
async fn apply_sway_fullscreen(
    enable: bool,
    player_pid: Option<u32>,
    wait_for_map: bool,
) -> PipApplyOutcome {
    let Some(con_id) = find_sway_con(player_pid, wait_for_map).await else {
        return PipApplyOutcome::WindowNotFound;
    };
    let cmd = build_sway_fullscreen_command(con_id, enable);
    match tokio::process::Command::new("swaymsg")
        .arg(&cmd)
        .output()
        .await
    {
        Ok(o) if o.status.success() => PipApplyOutcome::Applied,
        Ok(o) => PipApplyOutcome::Failed(format!(
            "swaymsg failed: {}",
            String::from_utf8_lossy(&o.stderr).trim()
        )),
        Err(e) => PipApplyOutcome::Failed(format!("Failed to execute swaymsg: {e}")),
    }
}

/// Apply or remove Picture-in-Picture window state on Linux Wayland compositors.
pub async fn apply_linux_pip(
    enabled: bool,
    corner_idx: usize,
    player_pid: Option<u32>,
    wait_for_map: bool,
) -> PipApplyOutcome {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (enabled, corner_idx, player_pid, wait_for_map);
        PipApplyOutcome::NotNeeded
    }
    #[cfg(target_os = "linux")]
    {
        match detect_linux_pip_backend() {
            LinuxPipBackend::Hyprland => {
                apply_hyprland_pip(enabled, corner_idx, player_pid, wait_for_map).await
            }
            LinuxPipBackend::Sway => {
                apply_sway_pip(enabled, corner_idx, player_pid, wait_for_map).await
            }
            LinuxPipBackend::UnsupportedWayland(de) => {
                PipApplyOutcome::UnsupportedCompositor(de)
            }
            LinuxPipBackend::X11OrNative => PipApplyOutcome::NotNeeded,
        }
    }
}

/// Find any mapped non-`mpv` window currently fullscreen/maximized on the same workspace/monitor
/// (such as a fullscreened terminal launched via `Super+V`) that would trap new windows behind it.
pub fn find_obstructing_fullscreen_client<'a>(
    clients: &'a [HyprClient],
    mpv: &HyprClient,
) -> Option<&'a HyprClient> {
    clients.iter().find(|c| {
        c.mapped
            && !c.address.is_empty()
            && c.address != mpv.address
            && c.fullscreen != 0
            && (mpv.workspace.id == 0 || c.workspace.id == mpv.workspace.id || c.monitor == mpv.monitor)
    })
}

/// Map Hyprland JSON `fullscreen` mode (`1` = maximize, `2` = true fullscreen) to `dispatch fullscreen <arg>`.
pub const fn hypr_fullscreen_dispatcher_arg(json_mode: i32) -> i32 {
    if json_mode == 1 {
        1
    } else {
        0
    }
}

/// Build Lua script to cleanly unset fullscreen on an obstructing window under Hyprland >= 0.57.
pub fn build_hypr_unset_fullscreen_lua(client: &HyprClient) -> Option<String> {
    if client.fullscreen == 0 || client.address.is_empty() {
        return None;
    }
    Some(format!(
        "hl.dispatch(hl.dsp.window.fullscreen({{ window = 'address:{}', action = 'unset' }}))",
        client.address
    ))
}

/// Build legacy `--batch` command to un-fullscreen an obstructing client matching its exact mode (`0` or `1`).
pub fn build_hypr_unset_fullscreen_legacy_batch(client: &HyprClient) -> Option<String> {
    if client.fullscreen == 0 || client.address.is_empty() {
        return None;
    }
    let arg = hypr_fullscreen_dispatcher_arg(client.fullscreen);
    Some(format!(
        "dispatch focuswindow address:{}; dispatch fullscreen {arg}",
        client.address
    ))
}

/// Build batch command to focus and raise the `mpv` window cleanly to the top of the stack.
pub fn build_hypr_focus_front_batch(mpv: &HyprClient) -> String {
    format!(
        "dispatch focuswindow address:{}; dispatch alterzorder top,address:{}; dispatch bringactivetotop",
        mpv.address, mpv.address
    )
}

/// Find any fullscreened non-`mpv` Sway node obstructing focus.
pub fn find_obstructing_sway_fullscreen_con(node: &SwayNode, mpv_con_id: i64) -> Option<i64> {
    if node.id != mpv_con_id && node.fullscreen_mode != 0 && node.pid.is_some() {
        return Some(node.id);
    }
    for child in node.floating_nodes.iter().chain(node.nodes.iter()) {
        if let Some(found) = find_obstructing_sway_fullscreen_con(child, mpv_con_id) {
            return Some(found);
        }
    }
    None
}

/// Bring a newly launched visible `mpv` window in front of fullscreen terminals and apply PiP/fullscreen.
/// Returns `Some((window_id, previous_mode))` if an obstructing terminal fullscreen was temporarily yielded.
pub async fn bring_player_to_front(
    player_pid: Option<u32>,
    corner_idx: usize,
    auto_pip: bool,
    auto_fullscreen: bool,
) -> (PipApplyOutcome, Option<(String, i32)>) {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (player_pid, corner_idx, auto_pip, auto_fullscreen);
        (PipApplyOutcome::NotNeeded, None)
    }
    #[cfg(target_os = "linux")]
    {
        match detect_linux_pip_backend() {
            LinuxPipBackend::Hyprland => {
                let Some(mpv) = find_mapped_hypr_client(player_pid, true).await else {
                    return (PipApplyOutcome::WindowNotFound, None);
                };

                let mut yielded: Option<(String, i32)> = None;
                if let Ok(clients) = hyprctl_json::<Vec<HyprClient>>("clients").await {
                    if let Some(obs) = find_obstructing_fullscreen_client(&clients, &mpv) {
                        let lua = build_hypr_unset_fullscreen_lua(obs);
                        let legacy = build_hypr_unset_fullscreen_legacy_batch(obs);
                        let _ = exec_hypr_script_or_batch(lua, legacy).await;
                        yielded = Some((obs.address.clone(), obs.fullscreen));
                    }
                }

                let focus_batch = build_hypr_focus_front_batch(&mpv);
                let _ = tokio::process::Command::new("hyprctl")
                    .args(["--batch", &focus_batch])
                    .output()
                    .await;

                let outcome = if auto_pip {
                    apply_hyprland_pip(true, corner_idx, player_pid, false).await
                } else if auto_fullscreen {
                    apply_hyprland_fullscreen(true, player_pid, false).await
                } else {
                    PipApplyOutcome::Applied
                };
                (outcome, yielded)
            }
            LinuxPipBackend::Sway => {
                let Some(con_id) = find_sway_con(player_pid, true).await else {
                    return (PipApplyOutcome::WindowNotFound, None);
                };

                let mut yielded: Option<(String, i32)> = None;
                if let Ok(tree) = swaymsg_json::<SwayNode>("get_tree").await {
                    if let Some(obs_id) = find_obstructing_sway_fullscreen_con(&tree, con_id) {
                        let cmd = format!("[con_id={obs_id}] fullscreen disable");
                        let _ = tokio::process::Command::new("swaymsg")
                            .arg(&cmd)
                            .output()
                            .await;
                        yielded = Some((format!("sway:{obs_id}"), 1));
                    }
                }

                let focus_cmd = format!("[con_id={con_id}] focus");
                let _ = tokio::process::Command::new("swaymsg")
                    .arg(&focus_cmd)
                    .output()
                    .await;

                let outcome = if auto_pip {
                    apply_sway_pip(true, corner_idx, player_pid, false).await
                } else if auto_fullscreen {
                    apply_sway_fullscreen(true, player_pid, false).await
                } else {
                    PipApplyOutcome::Applied
                };
                (outcome, yielded)
            }
            LinuxPipBackend::UnsupportedWayland(de) => {
                (PipApplyOutcome::UnsupportedCompositor(de), None)
            }
            LinuxPipBackend::X11OrNative => (PipApplyOutcome::NotNeeded, None),
        }
    }
}

/// Restore a previously fullscreened/maximized terminal window when playback exits.
pub async fn restore_yielded_fullscreen(address: &str, prev_mode: i32) {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (address, prev_mode);
    }
    #[cfg(target_os = "linux")]
    {
        if address.is_empty() || prev_mode == 0 {
            return;
        }
        if let Some(sway_id) = address.strip_prefix("sway:") {
            if let Ok(cid) = sway_id.parse::<i64>() {
                let cmd = format!("[con_id={cid}] fullscreen enable, focus");
                let _ = tokio::process::Command::new("swaymsg")
                    .arg(&cmd)
                    .output()
                    .await;
            }
            return;
        }
        match detect_linux_pip_backend() {
            LinuxPipBackend::Hyprland => {
                if let Ok(clients) = hyprctl_json::<Vec<HyprClient>>("clients").await {
                    // Only restore if the terminal window still exists and is currently un-fullscreened
                    if let Some(term) = clients.iter().find(|c| c.mapped && c.address == address) {
                        if term.fullscreen == 0 {
                            let arg = hypr_fullscreen_dispatcher_arg(prev_mode);
                            let batch = format!(
                                "dispatch focuswindow address:{address}; dispatch fullscreen {arg}"
                            );
                            let _ = tokio::process::Command::new("hyprctl")
                                .args(["--batch", &batch])
                                .output()
                                .await;
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// Unpin/unfloat if needed and toggle compositor fullscreen on Linux Wayland compositors.
pub async fn apply_linux_fullscreen(
    enable: bool,
    player_pid: Option<u32>,
    wait_for_map: bool,
) -> PipApplyOutcome {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (enable, player_pid, wait_for_map);
        PipApplyOutcome::NotNeeded
    }
    #[cfg(target_os = "linux")]
    {
        match detect_linux_pip_backend() {
            LinuxPipBackend::Hyprland => {
                apply_hyprland_fullscreen(enable, player_pid, wait_for_map).await
            }
            LinuxPipBackend::Sway => {
                apply_sway_fullscreen(enable, player_pid, wait_for_map).await
            }
            LinuxPipBackend::UnsupportedWayland(de) => {
                PipApplyOutcome::UnsupportedCompositor(de)
            }
            LinuxPipBackend::X11OrNative => PipApplyOutcome::NotNeeded,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_pip_rect_hidpi_with_reserved_topbar() {
        let mon = MonitorGeometry {
            x: 0,
            y: 0,
            phys_width: 2560,
            phys_height: 1600,
            scale: 1.6,
            transform: 0,
            reserved: [0, 40, 0, 0],
        };
        let br = compute_pip_rect(mon, pip_preset(0));
        assert_eq!(
            br,
            PipRect {
                x: 1096,
                y: 706,
                width: 480,
                height: 270
            }
        );

        let tr = compute_pip_rect(mon, pip_preset(1));
        assert_eq!(
            tr,
            PipRect {
                x: 1096,
                y: 64,
                width: 480,
                height: 270
            }
        );

        let bl = compute_pip_rect(mon, pip_preset(2));
        assert_eq!(
            bl,
            PipRect {
                x: 24,
                y: 706,
                width: 480,
                height: 270
            }
        );

        let tl = compute_pip_rect(mon, pip_preset(3));
        assert_eq!(
            tl,
            PipRect {
                x: 24,
                y: 64,
                width: 480,
                height: 270
            }
        );
    }

    #[test]
    fn test_find_hypr_target_client_never_picks_terminal() {
        let clients = vec![
            HyprClient {
                address: "0x111".to_string(),
                mapped: true,
                floating: false,
                pinned: false,
                fullscreen: 0,
                monitor: 0,
                workspace: HyprWorkspaceRef { id: 1 },
                class: "kitty".to_string(),
                pid: 1001,
            },
            HyprClient {
                address: "0x222".to_string(),
                mapped: true,
                floating: false,
                pinned: false,
                fullscreen: 0,
                monitor: 0,
                workspace: HyprWorkspaceRef { id: 1 },
                class: "mpv".to_string(),
                pid: 2002,
            },
        ];
        let pids: HashSet<u32> = [2002].into_iter().collect();
        let found = find_hypr_target_client(&clients, &pids).unwrap();
        assert_eq!(found.address, "0x222");

        // Unmapped or missing mpv must return None instead of grabbing kitty
        let wrong_pids: HashSet<u32> = [9999].into_iter().collect();
        assert!(find_hypr_target_client(&clients, &wrong_pids).is_none());
    }

    #[test]
    fn test_obstructing_fullscreen_terminal_yields_to_mpv_front() {
        let term = HyprClient {
            address: "0xaaa".to_string(),
            mapped: true,
            floating: false,
            pinned: false,
            fullscreen: 1, // Maximize / Super+V
            monitor: 0,
            workspace: HyprWorkspaceRef { id: 2 },
            class: "kitty".to_string(),
            pid: 1001,
        };
        let mpv = HyprClient {
            address: "0xbbb".to_string(),
            mapped: true,
            floating: false,
            pinned: false,
            fullscreen: 0,
            monitor: 0,
            workspace: HyprWorkspaceRef { id: 2 },
            class: PHAENNA_WAYLAND_APP_ID.to_string(),
            pid: 2002,
        };
        let clients = vec![term.clone(), mpv.clone()];
        let obs = find_obstructing_fullscreen_client(&clients, &mpv).unwrap();
        assert_eq!(obs.address, "0xaaa");
        assert_eq!(hypr_fullscreen_dispatcher_arg(1), 1);
        assert_eq!(hypr_fullscreen_dispatcher_arg(2), 0);
        let unset_lua = build_hypr_unset_fullscreen_lua(obs).unwrap();
        assert!(unset_lua.contains("address:0xaaa"));
        assert!(unset_lua.contains("action = 'unset'"));
        let focus_batch = build_hypr_focus_front_batch(&mpv);
        assert!(focus_batch.contains("focuswindow address:0xbbb"));
        assert!(focus_batch.contains("bringactivetotop"));
    }

    #[test]
    fn test_hypr_lua_scripts_use_address_selector_and_valid_actions() {
        let c = HyprClient {
            address: "0xdeadbeef".to_string(),
            mapped: true,
            floating: false,
            pinned: false,
            fullscreen: 2,
            monitor: 0,
            workspace: HyprWorkspaceRef::default(),
            class: PHAENNA_WAYLAND_APP_ID.to_string(),
            pid: 4242,
        };
        let rect = PipRect {
            x: 1096,
            y: 706,
            width: 480,
            height: 270,
        };
        let pip_lua = build_hypr_lua_script(&c, true, Some(rect)).unwrap();
        assert!(pip_lua.contains("window = 'address:0xdeadbeef'"));
        assert!(pip_lua.contains("action = 'unset'"));
        assert!(pip_lua.contains("action = 'set'"));
        assert!(pip_lua.contains("relative = false"));
        assert!(!pip_lua.contains("exact = true"));

        let pinned_c = HyprClient {
            floating: true,
            pinned: true,
            fullscreen: 0,
            ..c
        };
        let fs_lua = build_hypr_fullscreen_lua_script(&pinned_c, true).unwrap();
        assert!(fs_lua.contains("hl.dsp.window.pin({ window = 'address:0xdeadbeef', action = 'unset' })"));
        assert!(fs_lua.contains("hl.dsp.window.float({ window = 'address:0xdeadbeef', action = 'unset' })"));
        assert!(fs_lua.contains("hl.dsp.window.fullscreen({ window = 'address:0xdeadbeef', action = 'set' })"));
    }
}
