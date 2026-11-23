//! Cross-platform hardware, player-capability, power, and thermal intelligence.
//!
//! Probes Windows, Linux, and macOS once at runtime (cached via `OnceLock`) so Phaenna
//! selects zero-copy hardware decoders, avoids software AV1 CPU burn on non-AV1 silicon,
//! gates modern `mpv`/`libplacebo` options by actual binary support, and exposes live
//! power/thermal telemetry for Studio UI and the Dynamic Governor.

use crate::api::player::find_binary;
use crate::config::CodecPriority;
use std::process::Command;
use std::sync::OnceLock;

/// Cross-platform thermal pressure classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
#[allow(dead_code)]
pub enum ThermalLevel {
    #[default]
    Nominal,
    Fair,
    Serious,
    Critical,
}

#[allow(dead_code)]
impl ThermalLevel {
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Nominal => 0,
            Self::Fair => 1,
            Self::Serious => 2,
            Self::Critical => 3,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Nominal => "Nominal",
            Self::Fair => "Warm",
            Self::Serious => "Hot",
            Self::Critical => "Critical",
        }
    }

    pub const fn short_badge(self) -> &'static str {
        match self {
            Self::Nominal => "COOL",
            Self::Fair => "WARM",
            Self::Serious => "HOT",
            Self::Critical => "CRIT",
        }
    }

    pub fn from_celsius(temp_c: f32) -> Self {
        if temp_c >= 90.0 {
            Self::Critical
        } else if temp_c >= 82.0 {
            Self::Serious
        } else if temp_c >= 70.0 {
            Self::Fair
        } else {
            Self::Nominal
        }
    }
}

/// Primary GPU vendor identified on the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(dead_code)]
pub enum GpuVendor {
    Apple,
    Nvidia,
    Amd,
    Intel,
    #[default]
    Unknown,
}

/// Static hardware & OS decoder capability snapshot detected once per session.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub struct DeviceProfile {
    pub os_label: String,
    pub display_server: String,
    pub gpu_vendor: GpuVendor,
    pub gpu_label: String,
    pub ram_gb: Option<u32>,
    pub hw_av1_decode: bool,
    pub hw_vp9_decode: bool,
    pub hw_hevc_decode: bool,
    pub recommended_hwdec: &'static str,
    pub recommended_gpu_api: &'static str,
    pub is_laptop_or_handheld: bool,
}

#[allow(dead_code)]
impl DeviceProfile {
    /// Returns `true` when this device has dedicated silicon AV1 video decoding.
    pub const fn prefers_av1_codec(&self) -> bool {
        self.hw_av1_decode
    }

    /// Maps detected hardware capabilities to the coolest high-quality codec selector.
    pub const fn recommended_codec_priority(&self) -> CodecPriority {
        if self.hw_av1_decode {
            CodecPriority::Av1First
        } else {
            CodecPriority::Vp9HwFirst
        }
    }

    /// Human-readable hardware codec summary for Studio Page 3 Inspector.
    pub fn decode_summary(&self) -> String {
        let mut caps = Vec::new();
        if self.hw_av1_decode {
            caps.push("AV1");
        }
        if self.hw_vp9_decode {
            caps.push("VP9.2");
        }
        if self.hw_hevc_decode {
            caps.push("HEVC");
        }
        caps.push("H264");
        let av1_note = if self.hw_av1_decode {
            "HW AV1 Ready"
        } else {
            "AV1: SW -> Auto VP9"
        };
        format!(
            "HW {} ({}) [{}]",
            caps.join("+"),
            self.recommended_hwdec,
            av1_note
        )
    }

    /// Compact single-line host & GPU badge for Inspector headers.
    pub fn inspector_header(&self) -> String {
        match self.ram_gb {
            Some(gb) => format!(
                "{} · {} ({}GB) · {}",
                self.os_label, self.gpu_label, gb, self.display_server
            ),
            None => format!("{} · {} · {}", self.os_label, self.gpu_label, self.display_server),
        }
    }
}

/// Probed software capabilities of the installed `mpv` / `libmpv` player binary.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub struct PlayerCapabilities {
    pub binary_name: String,
    pub version_str: String,
    pub has_gpu_next: bool,
    pub has_shader_opts: bool,
    pub has_perceptual_gamut: bool,
    pub has_hdr_contrast_recovery: bool,
    pub has_hdr_peak_percentile: bool,
    pub has_libplacebo_opts: bool,
    pub has_icc_auto: bool,
    pub has_target_colorspace_hint: bool,
    pub has_gpu_shader_cache_dir: bool,
    pub has_vulkan: bool,
    pub has_d3d11: bool,
    pub has_opengl: bool,
}

#[allow(dead_code)]
impl PlayerCapabilities {
    pub fn conservative_fallback() -> Self {
        Self {
            binary_name: "mpv".to_string(),
            version_str: "unknown".to_string(),
            has_gpu_next: false,
            has_shader_opts: false,
            has_perceptual_gamut: false,
            has_hdr_contrast_recovery: false,
            has_hdr_peak_percentile: false,
            has_libplacebo_opts: false,
            has_icc_auto: false,
            has_target_colorspace_hint: false,
            has_gpu_shader_cache_dir: false,
            has_vulkan: false,
            has_d3d11: cfg!(target_os = "windows"),
            has_opengl: true,
        }
    }

    pub const fn effective_vo(&self) -> &'static str {
        if self.has_gpu_next {
            "gpu-next"
        } else {
            "gpu"
        }
    }

    pub fn effective_gpu_api(&self, dev: &DeviceProfile) -> &'static str {
        match dev.recommended_gpu_api {
            "d3d11" if self.has_d3d11 => "d3d11",
            "vulkan" if self.has_vulkan => "vulkan",
            _ if self.has_vulkan => "vulkan",
            _ if self.has_d3d11 => "d3d11",
            _ => "auto",
        }
    }

    pub fn inspector_summary(&self) -> String {
        let vo = self.effective_vo();
        let param_flag = if self.has_shader_opts { "YES" } else { "Static" };
        let plc_flag = if self.has_perceptual_gamut && self.has_hdr_contrast_recovery {
            "libplacebo Full"
        } else if self.has_gpu_next {
            "libplacebo Basic"
        } else {
            "Legacy GL"
        };
        format!(
            "{} {} · vo={} · //!PARAM: {} · {}",
            self.binary_name, self.version_str, vo, param_flag, plc_flag
        )
    }
}

/// Live cross-platform power and thermal snapshot polled periodically during playback.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[allow(dead_code)]
pub struct PowerThermalSnapshot {
    pub on_battery: bool,
    pub battery_pct: Option<u8>,
    pub battery_saver: bool,
    pub temp_celsius: Option<f32>,
    pub thermal_level: ThermalLevel,
}

#[allow(dead_code)]
impl PowerThermalSnapshot {
    pub fn status_badge(&self) -> String {
        let pwr = match (self.on_battery, self.battery_pct, self.battery_saver) {
            (true, Some(pct), true) => format!("BATT {pct}% SAVER"),
            (true, Some(pct), false) => format!("BATT {pct}%"),
            (true, None, _) => "BATTERY".to_string(),
            (false, Some(pct), _) => format!("AC {pct}%"),
            (false, None, _) => "AC".to_string(),
        };
        match self.temp_celsius {
            Some(c) => format!("{pwr} · {c:.0}C {}", self.thermal_level.short_badge()),
            None => format!("{pwr} · {}", self.thermal_level.short_badge()),
        }
    }

    /// Maps battery/thermal telemetry into a governor pressure ceiling (`0..=3`).
    pub fn effective_thermal_tier(&self) -> u8 {
        let mut tier = self.thermal_level.as_u8();
        if self.battery_saver || (self.on_battery && self.battery_pct.is_some_and(|p| p < 25)) {
            tier = tier.max(2);
        } else if self.on_battery {
            tier = tier.max(1);
        }
        tier.min(3)
    }
}

static DEVICE_PROFILE_CACHE: OnceLock<DeviceProfile> = OnceLock::new();
static PLAYER_CAPS_CACHE: OnceLock<PlayerCapabilities> = OnceLock::new();

/// Returns the cached hardware & OS capability profile (probes once per process).
#[allow(dead_code)]
pub fn detect_device_profile() -> &'static DeviceProfile {
    DEVICE_PROFILE_CACHE.get_or_init(probe_device_profile_uncached)
}

/// Returns cached installed player capabilities (`mpv` / `mpv.net` / `celluloid`).
#[allow(dead_code)]
pub fn detect_player_capabilities() -> &'static PlayerCapabilities {
    PLAYER_CAPS_CACHE.get_or_init(probe_player_capabilities_uncached)
}

fn mem_bytes_to_gb(mem_bytes: Option<u64>) -> Option<u32> {
    mem_bytes
        .filter(|&b| b > 0)
        .map(|b| ((b as f64) / (1024.0 * 1024.0 * 1024.0)).round().max(1.0) as u32)
}

/// Parse macOS CPU brand string (`sysctl -n machdep.cpu.brand_string`) into `DeviceProfile`.
pub fn parse_macos_chip_profile(
    brand: &str,
    mem_bytes: Option<u64>,
    has_battery: bool,
) -> DeviceProfile {
    let trimmed = brand.trim();
    let lower = trimmed.to_ascii_lowercase();
    let apple_gen = parse_apple_m_generation(&lower);
    let is_apple = apple_gen.is_some() || lower.contains("apple");
    let hw_av1 = apple_gen.is_some_and(|g| g >= 3);

    let gpu_label = if trimmed.is_empty() {
        "Apple Graphics".to_string()
    } else if is_apple {
        trimmed.to_string()
    } else {
        format!("Intel Mac ({trimmed})")
    };

    DeviceProfile {
        os_label: "macOS".to_string(),
        display_server: "Metal".to_string(),
        gpu_vendor: if is_apple {
            GpuVendor::Apple
        } else {
            GpuVendor::Intel
        },
        gpu_label,
        ram_gb: mem_bytes_to_gb(mem_bytes),
        hw_av1_decode: hw_av1,
        hw_vp9_decode: true,
        hw_hevc_decode: true,
        recommended_hwdec: "videotoolbox",
        recommended_gpu_api: "vulkan",
        is_laptop_or_handheld: has_battery,
    }
}

fn parse_apple_m_generation(lower: &str) -> Option<u32> {
    let idx = lower.find("apple m")?;
    let rest = &lower[idx + "apple m".len()..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse::<u32>().ok()
}

#[allow(dead_code)]
fn extract_first_model_number(haystack: &str, prefix: &str) -> Option<u32> {
    let idx = haystack.find(prefix)?;
    let rest = &haystack[idx + prefix.len()..];
    let digits: String = rest
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse::<u32>().ok()
}

#[allow(dead_code)]
fn classify_gpu_av1_support(lower: &str, vendor: GpuVendor) -> bool {
    match vendor {
        GpuVendor::Apple => parse_apple_m_generation(lower).is_some_and(|g| g >= 3),
        GpuVendor::Nvidia => {
            if let Some(num) = extract_first_model_number(lower, "rtx ") {
                if num >= 3000 {
                    return true;
                }
            }
            lower.contains("ada")
                || lower.contains("blackwell")
                || lower.contains("rtx a")
        }
        GpuVendor::Amd => {
            if let Some(rx_num) = extract_first_model_number(lower, "rx ") {
                // Navi 24 (RX 6400 / 6500 XT) omitted the VCN AV1 decoder block.
                return rx_num >= 6600 || ((700..1000).contains(&rx_num) && rx_num != 700);
            }
            for apu in ["680m", "740m", "760m", "780m", "880m", "890m"] {
                if lower.contains(apu) {
                    return true;
                }
            }
            lower.contains("rdna3") || lower.contains("rdna 3") || lower.contains("rdna4")
        }
        GpuVendor::Intel => {
            if lower.contains("arc")
                || lower.contains("iris xe")
                || lower.contains("core ultra")
            {
                return true;
            }
            if let Some(uhd_num) = extract_first_model_number(lower, "uhd graphics ") {
                return uhd_num >= 730;
            }
            false
        }
        GpuVendor::Unknown => false,
    }
}

#[allow(dead_code)]
fn infer_gpu_vendor(lower: &str, drm_vendor_hex: Option<&str>) -> GpuVendor {
    if let Some(hex) = drm_vendor_hex.map(|s| s.trim().to_ascii_lowercase()) {
        if hex == "0x10de" {
            return GpuVendor::Nvidia;
        } else if hex == "0x1002" {
            return GpuVendor::Amd;
        } else if hex == "0x8086" {
            return GpuVendor::Intel;
        }
    }
    if lower.contains("nvidia") || lower.contains("geforce") || lower.contains("rtx") || lower.contains("gtx") {
        GpuVendor::Nvidia
    } else if lower.contains("amd") || lower.contains("radeon") || lower.contains("rx ") {
        GpuVendor::Amd
    } else if lower.contains("intel") || lower.contains("iris") || lower.contains("uhd") || lower.contains("arc") {
        GpuVendor::Intel
    } else if lower.contains("apple") {
        GpuVendor::Apple
    } else {
        GpuVendor::Unknown
    }
}

/// Parse Windows GPU Registry `DriverDesc` string into `DeviceProfile`.
#[allow(dead_code)]
pub fn parse_windows_gpu_profile(
    gpu_name: &str,
    mem_bytes: Option<u64>,
    has_battery: bool,
) -> DeviceProfile {
    let trimmed = gpu_name.trim();
    let lower = trimmed.to_ascii_lowercase();
    let vendor = infer_gpu_vendor(&lower, None);
    let hw_av1 = classify_gpu_av1_support(&lower, vendor);
    let hw_vp9 = !lower.contains("basic display") && vendor != GpuVendor::Unknown;
    let hw_hevc = hw_vp9;

    let label = if trimmed.is_empty() {
        "Windows GPU".to_string()
    } else {
        trimmed.to_string()
    };

    DeviceProfile {
        os_label: "Windows".to_string(),
        display_server: "D3D11".to_string(),
        gpu_vendor: vendor,
        gpu_label: label,
        ram_gb: mem_bytes_to_gb(mem_bytes),
        hw_av1_decode: hw_av1,
        hw_vp9_decode: hw_vp9,
        hw_hevc_decode: hw_hevc,
        recommended_hwdec: "d3d11va",
        recommended_gpu_api: "d3d11",
        is_laptop_or_handheld: has_battery,
    }
}

/// Parse Linux `vainfo`, `/sys/class/drm/.../vendor`, `lspci`/Vulkan name, and Wayland flag.
#[allow(dead_code)]
pub fn parse_linux_profile(
    vainfo_out: Option<&str>,
    drm_vendor_hex: Option<&str>,
    gpu_name: Option<&str>,
    is_wayland: bool,
    mem_bytes: Option<u64>,
    has_battery: bool,
) -> DeviceProfile {
    let gpu_raw = gpu_name.unwrap_or("").trim();
    let lower = gpu_raw.to_ascii_lowercase();
    let vendor = infer_gpu_vendor(&lower, drm_vendor_hex);

    let (hw_av1, hw_vp9, hw_hevc) = if let Some(va) = vainfo_out {
        let has_va_profile = |needle: &str| {
            va.lines().any(|l| {
                l.contains(needle) && l.contains("VAEntrypointVLD")
            })
        };
        let av1 = has_va_profile("VAProfileAV1");
        let vp9 = has_va_profile("VAProfileVP9");
        let hevc = has_va_profile("VAProfileHEVC");
        (av1 || classify_gpu_av1_support(&lower, vendor), vp9 || vendor != GpuVendor::Unknown, hevc || vendor != GpuVendor::Unknown)
    } else {
        (
            classify_gpu_av1_support(&lower, vendor),
            vendor != GpuVendor::Unknown,
            vendor != GpuVendor::Unknown,
        )
    };

    let (hwdec, gpu_api) = match vendor {
        GpuVendor::Nvidia => ("nvdec", "vulkan"),
        GpuVendor::Amd | GpuVendor::Intel => ("vaapi", "vulkan"),
        GpuVendor::Apple => ("auto-safe", "vulkan"),
        GpuVendor::Unknown => ("auto-safe", "auto"),
    };

    let default_gpu_label = match vendor {
        GpuVendor::Nvidia => "NVIDIA GPU",
        GpuVendor::Amd => "AMD Radeon GPU",
        GpuVendor::Intel => "Intel Graphics",
        _ => "Linux GPU",
    };

    DeviceProfile {
        os_label: "Linux".to_string(),
        display_server: if is_wayland {
            "Wayland".to_string()
        } else {
            "X11".to_string()
        },
        gpu_vendor: vendor,
        gpu_label: if gpu_raw.is_empty() {
            default_gpu_label.to_string()
        } else {
            gpu_raw.to_string()
        },
        ram_gb: mem_bytes_to_gb(mem_bytes),
        hw_av1_decode: hw_av1,
        hw_vp9_decode: hw_vp9,
        hw_hevc_decode: hw_hevc,
        recommended_hwdec: hwdec,
        recommended_gpu_api: gpu_api,
        is_laptop_or_handheld: has_battery,
    }
}

/// Parse `mpv --version`, `--vo=help`, `--gpu-api=help`, and `--list-options` into `PlayerCapabilities`.
pub fn parse_mpv_capabilities(
    binary_name: &str,
    version_out: &str,
    vo_help: &str,
    gpu_api_help: &str,
    list_options: &str,
) -> PlayerCapabilities {
    let version_str = version_out
        .lines()
        .find_map(|line| {
            let trimmed = line.trim();
            trimmed
                .strip_prefix("mpv ")
                .and_then(|rest| rest.split_whitespace().next())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "unknown".to_string());

    let has_opt = |flag: &str| list_options.contains(flag);
    let has_word = |hay: &str, token: &str| {
        hay.lines().any(|line| {
            line.split_whitespace()
                .any(|w| w.trim_end_matches(':') == token)
        })
    };

    PlayerCapabilities {
        binary_name: binary_name.to_string(),
        version_str,
        has_gpu_next: has_word(vo_help, "gpu-next"),
        has_shader_opts: has_opt("--glsl-shader-opts"),
        has_perceptual_gamut: has_opt("--gamut-mapping-mode"),
        has_hdr_contrast_recovery: has_opt("--hdr-contrast-recovery"),
        has_hdr_peak_percentile: has_opt("--hdr-peak-percentile"),
        has_libplacebo_opts: has_opt("--libplacebo-opts"),
        has_icc_auto: has_opt("--icc-profile-auto"),
        has_target_colorspace_hint: has_opt("--target-colorspace-hint"),
        has_gpu_shader_cache_dir: has_opt("--gpu-shader-cache-dir"),
        has_vulkan: has_word(gpu_api_help, "vulkan"),
        has_d3d11: has_word(gpu_api_help, "d3d11"),
        has_opengl: has_word(gpu_api_help, "opengl"),
    }
}

/// Parse `pmset -g batt` stdout into `(on_battery, battery_pct)`.
pub fn parse_macos_pmset_batt(out: &str) -> (bool, Option<u8>) {
    let lower = out.to_ascii_lowercase();
    let on_battery = lower.contains("'battery power'");
    let pct = out.lines().find_map(|line| {
        if !line.contains('%') {
            return None;
        }
        line.split(|c: char| !c.is_ascii_digit() && c != '%')
            .find_map(|tok| tok.strip_suffix('%').and_then(|d| d.parse::<u8>().ok()))
            .map(|p| p.min(100))
    });
    (on_battery, pct)
}

/// Parse `pmset -g therm` stdout into `ThermalLevel`.
pub fn parse_macos_pmset_therm(out: &str) -> ThermalLevel {
    let lower = out.to_ascii_lowercase();
    if lower.contains("no thermal warning") {
        return ThermalLevel::Nominal;
    }
    if lower.contains("critical") {
        return ThermalLevel::Critical;
    }
    if lower.contains("serious") || lower.contains("heavy") {
        return ThermalLevel::Serious;
    }
    if lower.contains("fair") || lower.contains("moderate") || lower.contains("cpu_speed_limit") {
        for line in lower.lines() {
            if line.contains("cpu_speed_limit") {
                if let Some(val) = line
                    .split('=')
                    .nth(1)
                    .and_then(|s| s.trim().parse::<u32>().ok())
                {
                    if val <= 65 {
                        return ThermalLevel::Critical;
                    } else if val <= 85 {
                        return ThermalLevel::Serious;
                    } else if val < 100 {
                        return ThermalLevel::Fair;
                    }
                }
            }
        }
        if lower.contains("fair") || lower.contains("moderate") {
            return ThermalLevel::Fair;
        }
    }
    ThermalLevel::Nominal
}

/// Parse Linux `/sys/class/power_supply` and `/sys/class/thermal` data cleanly.
#[allow(dead_code)]
pub fn parse_linux_sysfs_power_thermal(
    ac_online: Option<&str>,
    bat_status: Option<&str>,
    bat_cap: Option<&str>,
    thermal_millic: &[i64],
    platform_profile: Option<&str>,
) -> PowerThermalSnapshot {
    let ac_is_online = ac_online.map(|s| s.trim() == "1");
    let status_discharging = bat_status.is_some_and(|s| s.trim().eq_ignore_ascii_case("discharging"));
    let on_battery = match ac_is_online {
        Some(false) => true,
        Some(true) => false,
        None => status_discharging,
    };
    let battery_pct = bat_cap.and_then(|s| s.trim().parse::<u8>().ok()).map(|p| p.min(100));
    let battery_saver = platform_profile
        .is_some_and(|p| p.trim().eq_ignore_ascii_case("low-power"));

    let max_temp_c = thermal_millic
        .iter()
        .copied()
        .filter(|&m| (5_000..=150_000).contains(&m))
        .map(|m| (m as f32) / 1000.0)
        .reduce(f32::max);

    let thermal_level = max_temp_c
        .map(ThermalLevel::from_celsius)
        .unwrap_or(ThermalLevel::Nominal);

    PowerThermalSnapshot {
        on_battery,
        battery_pct,
        battery_saver,
        temp_celsius: max_temp_c,
        thermal_level,
    }
}

/// Convert raw Windows `SYSTEM_POWER_STATUS` fields into `PowerThermalSnapshot`.
#[allow(dead_code)]
pub fn parse_windows_power_status(
    ac_line_status: u8,
    battery_pct: u8,
    battery_saver_flag: u8,
) -> PowerThermalSnapshot {
    let on_battery = ac_line_status == 0;
    let pct = if battery_pct <= 100 {
        Some(battery_pct)
    } else {
        None
    };
    PowerThermalSnapshot {
        on_battery,
        battery_pct: pct,
        battery_saver: battery_saver_flag != 0,
        temp_celsius: None,
        thermal_level: ThermalLevel::Nominal,
    }
}

fn run_cmd_stdout(bin: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(bin).args(args).output().ok()?;
    let mut txt = String::from_utf8_lossy(&out.stdout).into_owned();
    if txt.trim().is_empty() && !out.stderr.is_empty() {
        txt = String::from_utf8_lossy(&out.stderr).into_owned();
    }
    Some(txt)
}

fn probe_device_profile_uncached() -> DeviceProfile {
    #[cfg(target_os = "macos")]
    {
        let brand = run_cmd_stdout("sysctl", &["-n", "machdep.cpu.brand_string"]).unwrap_or_default();
        let mem_bytes = run_cmd_stdout("sysctl", &["-n", "hw.memsize"])
            .and_then(|s| s.trim().parse::<u64>().ok());
        let batt = run_cmd_stdout("pmset", &["-g", "batt"]).unwrap_or_default();
        let has_battery = batt.to_ascii_lowercase().contains("internalbattery");
        parse_macos_chip_profile(&brand, mem_bytes, has_battery)
    }

    #[cfg(target_os = "windows")]
    {
        let reg_out = run_cmd_stdout(
            "reg",
            &[
                "query",
                r"HKLM\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}\0000",
                "/v",
                "DriverDesc",
            ],
        )
        .unwrap_or_default();
        let gpu_name = reg_out
            .lines()
            .find_map(|l| {
                let idx = l.find("REG_SZ")?;
                Some(l[idx + "REG_SZ".len()..].trim().to_string())
            })
            .unwrap_or_default();
        parse_windows_gpu_profile(&gpu_name, None, false)
    }

    #[cfg(target_os = "linux")]
    {
        let vainfo_out = run_cmd_stdout("vainfo", &[]);
        let drm_vendor = (0..4).find_map(|i| {
            std::fs::read_to_string(format!("/sys/class/drm/card{i}/device/vendor")).ok()
        });
        let gpu_name = run_cmd_stdout("nvidia-smi", &["--query-gpu=name", "--format=csv,noheader"])
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                run_cmd_stdout("lspci", &[]).and_then(|out| {
                    out.lines()
                        .find(|l| {
                            l.contains("VGA compatible controller")
                                || l.contains("3D controller")
                                || l.contains("Display controller")
                        })
                        .and_then(|l| l.split(": ").nth(1))
                        .map(|s| s.trim().to_string())
                })
            });
        let mem_bytes = std::fs::read_to_string("/proc/meminfo").ok().and_then(|txt| {
            txt.lines().find_map(|l| {
                let kb = l
                    .strip_prefix("MemTotal:")?
                    .trim()
                    .strip_suffix("kB")?
                    .trim()
                    .parse::<u64>()
                    .ok()?;
                Some(kb * 1024)
            })
        });
        let is_wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty());
        let has_battery = std::fs::read_dir("/sys/class/power_supply")
            .ok()
            .into_iter()
            .flatten()
            .flatten()
            .any(|e| {
                e.file_name()
                    .to_string_lossy()
                    .to_ascii_uppercase()
                    .starts_with("BAT")
            });
        parse_linux_profile(
            vainfo_out.as_deref(),
            drm_vendor.as_deref(),
            gpu_name.as_deref(),
            is_wayland,
            mem_bytes,
            has_battery,
        )
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        DeviceProfile {
            os_label: std::env::consts::OS.to_string(),
            display_server: "Unknown".to_string(),
            gpu_vendor: GpuVendor::Unknown,
            gpu_label: "Generic GPU".to_string(),
            ram_gb: None,
            hw_av1_decode: false,
            hw_vp9_decode: false,
            hw_hevc_decode: false,
            recommended_hwdec: "auto-safe",
            recommended_gpu_api: "auto",
            is_laptop_or_handheld: false,
        }
    }
}

fn probe_player_capabilities_uncached() -> PlayerCapabilities {
    let bin_path = find_binary("mpv")
        .or_else(|| find_binary("mpvnet"))
        .or_else(|| find_binary("mpv.net"));
    let Some(path) = bin_path else {
        return PlayerCapabilities::conservative_fallback();
    };
    let bin = path.to_string_lossy().into_owned();
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "mpv".to_string());

    let ver = run_cmd_stdout(&bin, &["--version"]).unwrap_or_default();
    let vo_help = run_cmd_stdout(&bin, &["--vo=help"]).unwrap_or_default();
    let api_help = run_cmd_stdout(&bin, &["--vo=gpu-next", "--gpu-api=help"])
        .filter(|s| !s.is_empty())
        .or_else(|| run_cmd_stdout(&bin, &["--gpu-api=help"]))
        .unwrap_or_default();
    let list_opts = run_cmd_stdout(&bin, &["--list-options"]).unwrap_or_default();

    parse_mpv_capabilities(&name, &ver, &vo_help, &api_help, &list_opts)
}

/// Non-blocking periodic host power & thermal monitor for the Dynamic Governor IPC loop.
#[allow(dead_code)]
pub async fn poll_host_power_thermal() -> PowerThermalSnapshot {
    #[cfg(target_os = "macos")]
    {
        let batt_out = tokio::process::Command::new("pmset")
            .args(["-g", "batt"])
            .output()
            .await
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        let therm_out = tokio::process::Command::new("pmset")
            .args(["-g", "therm"])
            .output()
            .await
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        let (on_battery, battery_pct) = parse_macos_pmset_batt(&batt_out);
        let thermal_level = parse_macos_pmset_therm(&therm_out);
        PowerThermalSnapshot {
            on_battery,
            battery_pct,
            battery_saver: false,
            temp_celsius: None,
            thermal_level,
        }
    }

    #[cfg(target_os = "linux")]
    {
        let mut ac_online = None;
        let mut bat_status = None;
        let mut bat_cap = None;
        if let Ok(mut rd) = tokio::fs::read_dir("/sys/class/power_supply").await {
            while let Ok(Some(entry)) = rd.next_entry().await {
                let name = entry.file_name().to_string_lossy().into_owned();
                let base = entry.path();
                if name.starts_with("AC") || name.starts_with("ADP") {
                    if let Ok(v) = tokio::fs::read_to_string(base.join("online")).await {
                        ac_online = Some(v);
                    }
                } else if name.starts_with("BAT") && bat_cap.is_none() {
                    bat_status = tokio::fs::read_to_string(base.join("status")).await.ok();
                    bat_cap = tokio::fs::read_to_string(base.join("capacity")).await.ok();
                }
            }
        }
        let mut temps = Vec::new();
        if let Ok(mut rd) = tokio::fs::read_dir("/sys/class/thermal").await {
            while let Ok(Some(entry)) = rd.next_entry().await {
                if let Ok(raw) = tokio::fs::read_to_string(entry.path().join("temp")).await {
                    if let Ok(millic) = raw.trim().parse::<i64>() {
                        temps.push(millic);
                    }
                }
            }
        }
        let plat = tokio::fs::read_to_string("/sys/firmware/acpi/platform_profile")
            .await
            .ok();
        parse_linux_sysfs_power_thermal(
            ac_online.as_deref(),
            bat_status.as_deref(),
            bat_cap.as_deref(),
            &temps,
            plat.as_deref(),
        )
    }

    #[cfg(target_os = "windows")]
    {
        let wmic_out = tokio::process::Command::new("wmic")
            .args([
                "path",
                "Win32_Battery",
                "get",
                "BatteryStatus,EstimatedChargeRemaining",
                "/format:value",
            ])
            .output()
            .await
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        let mut ac_line = 1u8;
        let mut pct = 255u8;
        for line in wmic_out.lines() {
            if let Some(v) = line.strip_prefix("BatteryStatus=") {
                if v.trim() == "1" {
                    ac_line = 0;
                }
            } else if let Some(v) = line.strip_prefix("EstimatedChargeRemaining=") {
                if let Ok(p) = v.trim().parse::<u8>() {
                    pct = p;
                }
            }
        }
        parse_windows_power_status(ac_line, pct, 0)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        PowerThermalSnapshot::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_macos_silicon_m1_m2_vs_m3_m4_and_intel() {
        let m2 = parse_macos_chip_profile("Apple M2", Some(16 * 1024 * 1024 * 1024), true);
        assert_eq!(m2.gpu_vendor, GpuVendor::Apple);
        assert!(!m2.hw_av1_decode);
        assert!(m2.hw_vp9_decode);
        assert_eq!(m2.recommended_hwdec, "videotoolbox");
        assert_eq!(m2.recommended_codec_priority(), CodecPriority::Vp9HwFirst);
        assert_eq!(m2.ram_gb, Some(16));

        let m3 = parse_macos_chip_profile("Apple M3 Pro", Some(18 * 1024 * 1024 * 1024), true);
        assert!(m3.hw_av1_decode);
        assert_eq!(m3.recommended_codec_priority(), CodecPriority::Av1First);

        let m4 = parse_macos_chip_profile("Apple M4 Max", Some(64 * 1024 * 1024 * 1024), true);
        assert!(m4.hw_av1_decode);

        let intel = parse_macos_chip_profile("Intel(R) Core(TM) i9-9980HK", None, true);
        assert_eq!(intel.gpu_vendor, GpuVendor::Intel);
        assert!(!intel.hw_av1_decode);
        assert_eq!(intel.recommended_hwdec, "videotoolbox");
    }

    #[test]
    fn test_parse_windows_nvidia_amd_intel_av1_matrix() {
        let rtx4070 = parse_windows_gpu_profile("NVIDIA GeForce RTX 4070 Laptop GPU", None, true);
        assert_eq!(rtx4070.gpu_vendor, GpuVendor::Nvidia);
        assert!(rtx4070.hw_av1_decode);
        assert_eq!(rtx4070.recommended_hwdec, "d3d11va");
        assert_eq!(rtx4070.recommended_gpu_api, "d3d11");

        let rtx2060 = parse_windows_gpu_profile("NVIDIA GeForce RTX 2060", None, false);
        assert!(!rtx2060.hw_av1_decode);
        assert!(rtx2060.hw_vp9_decode);

        let rx7900 = parse_windows_gpu_profile("AMD Radeon RX 7900 XTX", None, false);
        assert_eq!(rx7900.gpu_vendor, GpuVendor::Amd);
        assert!(rx7900.hw_av1_decode);

        let rx6500 = parse_windows_gpu_profile("AMD Radeon RX 6500 XT", None, false);
        assert!(!rx6500.hw_av1_decode);

        let radeon780m = parse_windows_gpu_profile("AMD Radeon 780M Graphics", None, true);
        assert!(radeon780m.hw_av1_decode);

        let arc = parse_windows_gpu_profile("Intel(R) Arc(TM) A770 Graphics", None, false);
        assert_eq!(arc.gpu_vendor, GpuVendor::Intel);
        assert!(arc.hw_av1_decode);

        let uhd630 = parse_windows_gpu_profile("Intel(R) UHD Graphics 630", None, false);
        assert!(!uhd630.hw_av1_decode);
        assert!(uhd630.hw_vp9_decode);
    }

    #[test]
    fn test_parse_linux_vainfo_wayland_and_nvidia_x11() {
        let va = "libva info: VA-API version 1.21.0\n\
                  VAProfileVP9Profile2             : VAEntrypointVLD\n\
                  VAProfileHEVCMain10              : VAEntrypointVLD\n\
                  VAProfileAV1Profile0             : VAEntrypointVLD\n";
        let amd = parse_linux_profile(
            Some(va),
            Some("0x1002\n"),
            Some("AMD Radeon 780M"),
            true,
            Some(32 * 1024 * 1024 * 1024),
            true,
        );
        assert_eq!(amd.display_server, "Wayland");
        assert_eq!(amd.gpu_vendor, GpuVendor::Amd);
        assert!(amd.hw_av1_decode);
        assert!(amd.hw_vp9_decode);
        assert_eq!(amd.recommended_hwdec, "vaapi");
        assert_eq!(amd.recommended_gpu_api, "vulkan");

        let nv = parse_linux_profile(
            None,
            Some("0x10de"),
            Some("NVIDIA GeForce RTX 3080"),
            false,
            None,
            false,
        );
        assert_eq!(nv.display_server, "X11");
        assert_eq!(nv.recommended_hwdec, "nvdec");
        assert!(nv.hw_av1_decode);
    }

    #[test]
    fn test_parse_mpv_capabilities_modern_vs_legacy() {
        let modern = parse_mpv_capabilities(
            "mpv",
            "mpv v0.42.0 Copyright ...",
            "Valid values for option vo are:\n  gpu\n  gpu-next\n",
            "Available GPU APIs:\n  auto\n  vulkan\n  opengl\n",
            " --glsl-shader-opts\n --gamut-mapping-mode\n --hdr-contrast-recovery\n --hdr-peak-percentile\n --libplacebo-opts\n --icc-profile-auto\n --target-colorspace-hint\n --gpu-shader-cache-dir\n",
        );
        assert_eq!(modern.version_str, "v0.42.0");
        assert!(modern.has_gpu_next);
        assert!(modern.has_shader_opts);
        assert!(modern.has_perceptual_gamut);
        assert!(modern.has_hdr_contrast_recovery);
        assert!(modern.has_libplacebo_opts);
        assert_eq!(modern.effective_vo(), "gpu-next");

        let legacy = parse_mpv_capabilities(
            "mpv",
            "mpv 0.34.1 Copyright ...",
            "  gpu\n",
            "  opengl\n",
            " --gamut-mapping-mode\n",
        );
        assert_eq!(legacy.version_str, "0.34.1");
        assert!(!legacy.has_gpu_next);
        assert!(!legacy.has_shader_opts);
        assert!(!legacy.has_hdr_contrast_recovery);
        assert_eq!(legacy.effective_vo(), "gpu");
    }

    #[test]
    fn test_parse_power_and_thermal_across_macos_linux_windows() {
        let (batt, pct) = parse_macos_pmset_batt(
            "Now drawing from 'Battery Power'\n -InternalBattery-0\t42%; discharging; 3:10 remaining\n",
        );
        assert!(batt);
        assert_eq!(pct, Some(42));
        assert_eq!(
            parse_macos_pmset_therm("Note: No thermal warning level has been recorded"),
            ThermalLevel::Nominal
        );
        assert_eq!(
            parse_macos_pmset_therm("CPU_Scheduler_Limit = 100\nCPU_Speed_Limit = 78"),
            ThermalLevel::Serious
        );

        let lin = parse_linux_sysfs_power_thermal(
            Some("0\n"),
            Some("Discharging\n"),
            Some("20\n"),
            &[48_000, 84_500],
            Some("low-power\n"),
        );
        assert!(lin.on_battery);
        assert_eq!(lin.battery_pct, Some(20));
        assert!(lin.battery_saver);
        assert_eq!(lin.thermal_level, ThermalLevel::Serious);
        assert_eq!(lin.effective_thermal_tier(), 2);

        let win = parse_windows_power_status(0, 18, 1);
        assert!(win.on_battery);
        assert_eq!(win.battery_pct, Some(18));
        assert!(win.battery_saver);
        assert_eq!(win.effective_thermal_tier(), 2);
    }

    #[test]
    fn test_live_host_probe_returns_valid_profile() {
        let dev = detect_device_profile();
        assert!(!dev.os_label.is_empty());
        assert!(!dev.gpu_label.is_empty());
        assert!(!dev.decode_summary().is_empty());
        let caps = detect_player_capabilities();
        assert!(!caps.inspector_summary().is_empty());
    }
}
