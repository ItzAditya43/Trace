//! The see-through stats overlay: where it goes on the screen, what it
//! shows, and the frame-rate feed from MangoHud.

use crate::sys_util::{data_dir, read_num, read_trimmed};
use serde::{Deserialize, Serialize};
use std::fs;
use std::sync::Mutex;

pub const ITEMS: &[&str] = &["fps", "cpu", "gpu", "ram", "temp", "fan", "net", "battery", "clock"];
pub const CORNERS: &[&str] = &["top-left", "top-center", "top-right", "bottom-left", "bottom-center", "bottom-right"];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Config {
    /// Show it whenever Trace starts.
    pub open_at_start: bool,
    /// "auto" (a strip while a game runs, a panel otherwise) | "strip" | "panel"
    pub layout: String,
    pub corner: String,
    /// A monitor name, or "auto" for whichever has the focus.
    pub monitor: String,
    /// How dark the tint behind each number is, 0 (none) to 0.9.
    pub backdrop: f32,
    pub text_scale: f32,
    pub items: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            open_at_start: false,
            layout: "auto".into(),
            corner: "top-right".into(),
            monitor: "auto".into(),
            backdrop: 0.45,
            text_scale: 1.0,
            items: ["fps", "cpu", "gpu", "ram", "temp", "net"].iter().map(|s| s.to_string()).collect(),
        }
    }
}

impl Config {
    /// The same settings with every value forced into what the overlay accepts.
    pub fn cleaned(mut self) -> Self {
        if !["auto", "strip", "panel"].contains(&self.layout.as_str()) {
            self.layout = "auto".into();
        }
        if !CORNERS.contains(&self.corner.as_str()) {
            self.corner = "top-right".into();
        }
        if self.monitor != "auto" && !self.monitor.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            self.monitor = "auto".into();
        }
        self.backdrop = if self.backdrop.is_finite() { self.backdrop.clamp(0.0, 0.9) } else { 0.45 };
        self.text_scale = if self.text_scale.is_finite() { self.text_scale.clamp(0.8, 1.8) } else { 1.0 };
        self.items.retain(|i| ITEMS.contains(&i.as_str()));
        self.items.dedup();
        if self.items.is_empty() {
            self.items = Config::default().items;
        }
        self
    }
}

pub fn config() -> Config {
    fs::read_to_string(data_dir().join("overlay.json")).ok().and_then(|t| serde_json::from_str::<Config>(&t).ok()).unwrap_or_default().cleaned()
}

pub fn save_config(config: &Config) {
    fs::write(data_dir().join("overlay.json"), serde_json::to_string_pretty(config).unwrap_or_default()).ok();
}

/// A monitor as the desktop lays it out, in layout pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Screen {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    /// Space taken by bars: left, top, right, bottom.
    pub reserved: [i32; 4],
    pub focused: bool,
    /// Something is fullscreen on it right now.
    pub fullscreen: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Geometry {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    /// "strip" | "panel"
    pub layout: String,
    pub monitor: String,
    /// The same place counted from the monitor's own top-left corner.
    pub local_x: i32,
    pub local_y: i32,
    /// The corner it is anchored to, e.g. "top-right".
    pub corner: String,
}

impl Geometry {
    /// Where the window itself must sit so that the numbers land at this
    /// place. `window_width`/`window_height` are what the desktop actually
    /// gave the window, which can be more than was asked for: the toolkit
    /// has a smallest window size, and sometimes adds an invisible border
    /// all round. The numbers are anchored to a corner inside the window,
    /// so the window is lined up by that corner.
    pub fn window_origin(&self, window_width: i32, window_height: i32) -> (i32, i32) {
        let inset = Self::border(self.width, self.height, window_width, window_height);
        let x = if self.corner.ends_with("left") {
            self.x - inset
        } else if self.corner.ends_with("center") {
            self.x + self.width / 2 - window_width / 2
        } else {
            self.x + self.width + inset - window_width
        };
        let y = if self.corner.starts_with("top") { self.y - inset } else { self.y + self.height + inset - window_height };
        (x, y)
    }

    /// The toolkit's invisible border, when it added one: exactly that much
    /// extra on both sides in both directions, on top of its smallest size.
    fn border(width: i32, height: i32, window_width: i32, window_height: i32) -> i32 {
        const BORDER: i32 = 24;
        const SMALLEST: i32 = 200;
        let both = 2 * BORDER;
        if window_width - width.max(SMALLEST.min(window_width - both)) == both && window_height - height.max(SMALLEST) == both {
            BORDER
        } else {
            0
        }
    }
}

const MARGIN: i32 = 12;
const STRIP_ITEM_WIDTH: f32 = 92.0;
const STRIP_HEIGHT: f32 = 34.0;
const PANEL_WIDTH: f32 = 188.0;
const PANEL_ROW: f32 = 25.0;

/// Where the overlay goes: inside the chosen monitor's usable area (clear of
/// bars), in the chosen corner, sized from what it shows. A strip that would
/// not fit the monitor's width becomes a panel; a panel taller than the
/// monitor is cut to it.
pub fn geometry(config: &Config, screens: &[Screen], game_running: bool, fps_available: bool) -> Option<Geometry> {
    // A chosen monitor wins. Otherwise: while a game runs, the screen showing
    // something fullscreen (the game), even if the keyboard is on the other one;
    // failing that, the screen in use.
    let screen = screens
        .iter()
        .find(|s| s.name == config.monitor)
        .or_else(|| screens.iter().find(|s| game_running && s.fullscreen && s.focused))
        .or_else(|| screens.iter().find(|s| game_running && s.fullscreen))
        .or_else(|| screens.iter().find(|s| s.focused))
        .or(screens.first())?;
    let [left, top, right, bottom] = screen.reserved;
    let (ux, uy) = (screen.x + left, screen.y + top);
    let (uw, uh) = (screen.width - left - right, screen.height - top - bottom);

    let visible: Vec<&String> = config.items.iter().filter(|i| i.as_str() != "fps" || fps_available).collect();
    let shown = visible.len().max(1) as f32;
    // In a strip, the network and clock readings are wider than a percentage.
    let strip_units: f32 = visible.iter().map(|i| match i.as_str() { "net" => 1.7, "clock" => 1.35, _ => 1.0 }).sum::<f32>().max(1.0);
    let scale = config.text_scale;
    let mut layout = match config.layout.as_str() {
        "strip" => "strip",
        "panel" => "panel",
        _ if game_running => "strip",
        _ => "panel",
    };
    let strip_width = (strip_units * STRIP_ITEM_WIDTH * scale + 10.0) as i32;
    if layout == "strip" && strip_width > uw - 2 * MARGIN {
        layout = "panel";
    }
    let (width, height) = if layout == "strip" {
        (strip_width, (STRIP_HEIGHT * scale) as i32)
    } else {
        ((PANEL_WIDTH * scale) as i32, (shown * PANEL_ROW * scale + 12.0) as i32)
    };
    let width = width.min(uw - 2 * MARGIN).max(40);
    let height = height.min(uh - 2 * MARGIN).max(20);

    let x = if config.corner.ends_with("left") {
        ux + MARGIN
    } else if config.corner.ends_with("center") {
        ux + (uw - width) / 2
    } else {
        ux + uw - width - MARGIN
    };
    let y = if config.corner.starts_with("top") { uy + MARGIN } else { uy + uh - height - MARGIN };
    Some(Geometry { x, y, width, height, layout: layout.into(), monitor: screen.name.clone(), local_x: x - screen.x, local_y: y - screen.y, corner: config.corner.clone() })
}

/// Monitors as Hyprland reports them. Sizes are divided by the scale and
/// swapped for sideways rotations, which is how windows are positioned.
pub fn screens() -> Vec<Screen> {
    let out = crate::sys_util::run_lossy(std::process::Command::new("hyprctl").args(["monitors", "-j"]));
    let Ok(serde_json::Value::Array(list)) = serde_json::from_str(&out) else {
        return Vec::new();
    };
    let workspaces = crate::sys_util::run_lossy(std::process::Command::new("hyprctl").args(["workspaces", "-j"]));
    let fullscreen_workspaces: Vec<i64> = serde_json::from_str::<serde_json::Value>(&workspaces)
        .ok()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter(|w| w["hasfullscreen"].as_bool().unwrap_or(false))
        .filter_map(|w| w["id"].as_i64())
        .collect();
    list.iter()
        .filter(|m| !m["disabled"].as_bool().unwrap_or(false))
        .map(|m| {
            let scale = m["scale"].as_f64().unwrap_or(1.0).max(0.25);
            let (mut w, mut h) = (m["width"].as_f64().unwrap_or(1920.0) / scale, m["height"].as_f64().unwrap_or(1080.0) / scale);
            if m["transform"].as_u64().unwrap_or(0) % 2 == 1 {
                std::mem::swap(&mut w, &mut h);
            }
            let reserved = m["reserved"].as_array().map(|r| r.iter().map(|v| v.as_i64().unwrap_or(0) as i32).collect::<Vec<_>>()).unwrap_or_default();
            Screen {
                name: m["name"].as_str().unwrap_or_default().to_string(),
                x: m["x"].as_i64().unwrap_or(0) as i32,
                y: m["y"].as_i64().unwrap_or(0) as i32,
                width: w as i32,
                height: h as i32,
                reserved: [reserved.first().copied().unwrap_or(0), reserved.get(1).copied().unwrap_or(0), reserved.get(2).copied().unwrap_or(0), reserved.get(3).copied().unwrap_or(0)],
                focused: m["focused"].as_bool().unwrap_or(false),
                fullscreen: m["activeWorkspace"]["id"].as_i64().is_some_and(|id| fullscreen_workspaces.contains(&id)),
            }
        })
        .collect()
}

// ---- what it shows ----

#[derive(Serialize, Default, Clone)]
pub struct Data {
    pub cpu_percent: f32,
    pub ram_percent: f32,
    pub ram_used_gb: f32,
    pub temp_c: Option<f32>,
    pub gpu_percent: Option<f32>,
    pub fan_rpm: Option<u32>,
    pub net_down: f64,
    pub net_up: f64,
    pub battery_percent: Option<u32>,
    pub charging: bool,
    pub fps: Option<f32>,
    pub frametime_ms: Option<f32>,
    pub game: Option<String>,
}

struct Previous {
    cpu: (u64, u64),
    net: (u64, u64),
    at: Option<std::time::Instant>,
    game: Option<String>,
    game_checked: u32,
}

static PREVIOUS: Mutex<Previous> = Mutex::new(Previous { cpu: (0, 0), net: (0, 0), at: None, game: None, game_checked: 0 });

fn cpu_times() -> (u64, u64) {
    let text = fs::read_to_string("/proc/stat").unwrap_or_default();
    let v: Vec<u64> = text.lines().next().unwrap_or("").split_whitespace().skip(1).filter_map(|x| x.parse().ok()).collect();
    (v.iter().sum(), v.get(3).copied().unwrap_or(0) + v.get(4).copied().unwrap_or(0))
}

/// Bytes received and sent on real network sockets (not bridges, tunnels or containers).
fn net_bytes() -> (u64, u64) {
    let text = fs::read_to_string("/proc/net/dev").unwrap_or_default();
    text.lines()
        .skip(2)
        .filter_map(|l| {
            let (name, rest) = l.split_once(':')?;
            let name = name.trim();
            if name == "lo" || ["docker", "br-", "veth", "tun", "tap", "wg", "virbr"].iter().any(|p| name.starts_with(p)) {
                return None;
            }
            let f: Vec<u64> = rest.split_whitespace().filter_map(|v| v.parse().ok()).collect();
            Some((*f.first()?, *f.get(8)?))
        })
        .fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1))
}

fn hwmon(name_is: &[&str]) -> Option<String> {
    crate::tweaks::expand_globs(&["/sys/class/hwmon/hwmon*"]).into_iter().find(|dir| read_trimmed(&format!("{dir}/name")).is_some_and(|n| name_is.contains(&n.as_str())))
}

fn cpu_temp() -> Option<f32> {
    let dir = hwmon(&["k10temp", "coretemp", "zenpower", "cpu_thermal"])?;
    read_num::<f32>(&format!("{dir}/temp1_input")).map(|t| t / 1000.0)
}

fn fan_rpm() -> Option<u32> {
    crate::tweaks::expand_globs(&["/sys/class/hwmon/hwmon*/fan*_input"]).iter().filter_map(|f| read_num::<u32>(f)).max()
}

fn gpu_busy() -> Option<f32> {
    crate::tweaks::expand_globs(&["/sys/class/drm/card*/device/gpu_busy_percent"]).iter().filter_map(|f| read_num::<f32>(f)).reduce(f32::max)
}

pub fn fps_folder() -> std::path::PathBuf {
    dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("trace-fps")
}

/// The launch option that makes a game report its frame rate to Trace:
/// MangoHud runs inside the game, draws nothing, and writes a line twice a second.
pub fn fps_launch_option() -> String {
    // MangoHud writes nothing if the folder isn't there.
    fs::create_dir_all(fps_folder()).ok();
    format!("MANGOHUD_CONFIG=alpha=0.0,background_alpha=0.0,autostart_log=1,log_interval=500,output_folder={} mangohud %command%", fps_folder().display())
}

/// Frame rate and frame time from the last line of a log (fps is the first column).
fn parse_fps_line(line: &str) -> Option<(f32, f32)> {
    let mut f = line.split(',');
    let fps: f32 = f.next()?.trim().parse().ok()?;
    let frametime: f32 = f.next()?.trim().parse().ok()?;
    (fps.is_finite() && fps > 0.0 && fps < 2000.0).then_some((fps, frametime))
}

/// The newest reading, if a game wrote one in the last three seconds.
pub fn fps_now() -> Option<(f32, f32)> {
    let now = std::time::SystemTime::now();
    let newest = fs::read_dir(fps_folder())
        .ok()?
        .flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.ends_with(".csv") && !name.ends_with("_summary.csv")
        })
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .max_by_key(|(modified, _)| *modified)?;
    if now.duration_since(newest.0).ok()?.as_secs() > 3 {
        return None;
    }
    // Only the tail matters, and these files grow for as long as the game runs.
    use std::io::{Read, Seek, SeekFrom};
    let mut file = fs::File::open(&newest.1).ok()?;
    let len = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(600))).ok()?;
    let mut tail = String::new();
    file.read_to_string(&mut tail).ok()?;
    tail.lines().rev().find_map(parse_fps_line)
}

/// Removes frame-rate logs from finished games (older than a day).
pub fn prune_fps_logs() {
    fs::create_dir_all(fps_folder()).ok();
    let Ok(dir) = fs::read_dir(fps_folder()) else {
        return;
    };
    for entry in dir.flatten() {
        let old = entry.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).is_some_and(|age| age.as_secs() > 86_400);
        if old && entry.file_name().to_string_lossy().ends_with(".csv") {
            fs::remove_file(entry.path()).ok();
        }
    }
}

/// One reading for the overlay. Cheap enough to take every second: a
/// handful of small files, no programs started.
pub fn sample() -> Data {
    let mut prev = PREVIOUS.lock().unwrap();
    let now = std::time::Instant::now();
    let secs = prev.at.map(|t| now.duration_since(t).as_secs_f64()).unwrap_or(0.0);

    let cpu = cpu_times();
    let cpu_percent = if prev.cpu.0 > 0 && cpu.0 > prev.cpu.0 {
        (1.0 - (cpu.1 - prev.cpu.1) as f32 / (cpu.0 - prev.cpu.0) as f32) * 100.0
    } else {
        0.0
    };
    let net = net_bytes();
    let (net_down, net_up) = if secs > 0.2 && prev.net.0 > 0 {
        (net.0.saturating_sub(prev.net.0) as f64 / secs, net.1.saturating_sub(prev.net.1) as f64 / secs)
    } else {
        (0.0, 0.0)
    };

    // Looking for a game means reading every process's command line, so do it less often.
    if prev.game_checked == 0 {
        prev.game = crate::batch5::running_game();
    }
    prev.game_checked = (prev.game_checked + 1) % 5;

    let meminfo = fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let kb = |key: &str| meminfo.lines().find(|l| l.starts_with(key)).and_then(|l| l.split_whitespace().nth(1)).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
    let (total, available) = (kb("MemTotal:"), kb("MemAvailable:"));
    let battery = crate::batch4::battery_now();
    let fps = fps_now();

    prev.cpu = cpu;
    prev.net = net;
    prev.at = Some(now);
    Data {
        cpu_percent: cpu_percent.clamp(0.0, 100.0),
        ram_percent: if total > 0.0 { (total - available) / total * 100.0 } else { 0.0 },
        ram_used_gb: (total - available) / 1048576.0,
        temp_c: cpu_temp(),
        gpu_percent: gpu_busy(),
        fan_rpm: fan_rpm(),
        net_down,
        net_up,
        battery_percent: battery.as_ref().map(|b| b.percent),
        charging: battery.as_ref().is_some_and(|b| !b.discharging),
        fps: fps.map(|f| f.0),
        frametime_ms: fps.map(|f| f.1),
        game: prev.game.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(name: &str, x: i32, w: i32, h: i32, focused: bool) -> Screen {
        Screen { name: name.into(), x, y: 0, width: w, height: h, reserved: [0, 45, 0, 0], focused, fullscreen: false }
    }

    fn inside(g: &Geometry, s: &Screen) -> bool {
        g.x >= s.x + s.reserved[0] && g.y >= s.y + s.reserved[1] && g.x + g.width <= s.x + s.width - s.reserved[2] && g.y + g.height <= s.y + s.height - s.reserved[3]
    }

    #[test]
    fn stays_inside_the_usable_area_in_every_corner_at_every_size() {
        let screens = [screen("eDP-1", 0, 1920, 1080, false), screen("HDMI-A-1", 1920, 1280, 720, true), screen("tiny", 3200, 640, 480, false)];
        for corner in CORNERS {
            for layout in ["auto", "strip", "panel"] {
                for scale in [0.8, 1.0, 1.8] {
                    for monitor in ["auto", "eDP-1", "tiny"] {
                        for game in [false, true] {
                            let config = Config { corner: corner.to_string(), layout: layout.into(), text_scale: scale, monitor: monitor.into(), items: ITEMS.iter().map(|s| s.to_string()).collect(), ..Default::default() };
                            let g = geometry(&config, &screens, game, game).unwrap();
                            let s = screens.iter().find(|s| s.name == g.monitor).unwrap();
                            assert!(inside(&g, s), "{corner} {layout} {scale} {monitor}: {g:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn follows_the_focused_monitor_and_clears_the_bar() {
        let screens = [screen("eDP-1", 0, 1920, 1080, false), screen("HDMI-A-1", 1920, 1920, 1080, true)];
        let g = geometry(&Config::default(), &screens, false, false).unwrap();
        assert_eq!(g.monitor, "HDMI-A-1");
        assert_eq!(g.y, 45 + MARGIN, "below the 45-pixel bar");
        assert_eq!(g.x + g.width, 1920 + 1920 - MARGIN, "against the right edge");
        let pinned = geometry(&Config { monitor: "eDP-1".into(), ..Default::default() }, &screens, false, false).unwrap();
        assert_eq!(pinned.monitor, "eDP-1");
        let gone = geometry(&Config { monitor: "DP-9".into(), ..Default::default() }, &screens, false, false).unwrap();
        assert_eq!(gone.monitor, "HDMI-A-1", "an unplugged monitor falls back to the focused one");
    }

    #[test]
    fn a_fullscreen_game_on_the_other_screen_gets_the_overlay() {
        let mut screens = [screen("laptop", 0, 1920, 1080, true), screen("tv", 1920, 1920, 1080, false)];
        screens[1].fullscreen = true;
        assert_eq!(geometry(&Config::default(), &screens, true, false).unwrap().monitor, "tv", "the game's screen, though the keyboard is on the laptop");
        assert_eq!(geometry(&Config::default(), &screens, false, false).unwrap().monitor, "laptop", "a fullscreen video is not a game");
        assert_eq!(geometry(&Config { monitor: "laptop".into(), ..Default::default() }, &screens, true, false).unwrap().monitor, "laptop", "a chosen monitor always wins");
    }

    #[test]
    fn a_game_gets_the_strip_and_a_narrow_screen_gets_the_panel() {
        let wide = [screen("a", 0, 1920, 1080, true)];
        assert_eq!(geometry(&Config::default(), &wide, true, true).unwrap().layout, "strip");
        assert_eq!(geometry(&Config::default(), &wide, false, false).unwrap().layout, "panel");
        let narrow = [screen("a", 0, 400, 800, true)];
        let all = Config { layout: "strip".into(), items: ITEMS.iter().map(|s| s.to_string()).collect(), ..Default::default() };
        assert_eq!(geometry(&all, &narrow, true, true).unwrap().layout, "panel", "a strip that can't fit turns into a panel");
        // Without a frame-rate feed the fps slot takes no room.
        let with = geometry(&Config { layout: "strip".into(), ..Default::default() }, &wide, true, true).unwrap();
        let without = geometry(&Config { layout: "strip".into(), ..Default::default() }, &wide, true, false).unwrap();
        assert!(without.width < with.width);
    }

    #[test]
    fn a_window_larger_than_asked_is_lined_up_by_its_corner() {
        let screens = [screen("a", 0, 1920, 1080, true)];
        // Asked for 188 x 137; the desktop made the window 200 x 200, no border.
        let top_right = geometry(&Config::default(), &screens, false, false).unwrap();
        assert_eq!((top_right.width, top_right.height), (188, 137));
        let (x, y) = top_right.window_origin(200, 200);
        assert_eq!((x + 200, y), (top_right.x + top_right.width, top_right.y), "right and top edges meet");
        // The same window with the invisible 24-pixel border all round: 236 x 248.
        let (x, y) = top_right.window_origin(236, 248);
        assert_eq!((x + 236 - 24, y + 24), (top_right.x + top_right.width, top_right.y), "the edges inside the border meet");
        let bottom_left = geometry(&Config { corner: "bottom-left".into(), ..Default::default() }, &screens, false, false).unwrap();
        let (x, y) = bottom_left.window_origin(200, 200);
        assert_eq!((x, y + 200), (bottom_left.x, bottom_left.y + bottom_left.height), "left and bottom edges meet");
        let (x, y) = bottom_left.window_origin(236, 248);
        assert_eq!((x + 24, y + 248 - 24), (bottom_left.x, bottom_left.y + bottom_left.height));
        let centre = geometry(&Config { corner: "top-center".into(), ..Default::default() }, &screens, false, false).unwrap();
        assert_eq!(centre.window_origin(200, 200).0 + 100, centre.x + centre.width / 2);
        // A wide strip: as asked, and with the border.
        let strip = geometry(&Config { layout: "strip".into(), ..Default::default() }, &screens, true, true).unwrap();
        assert_eq!(strip.window_origin(strip.width, 200), (strip.x, strip.y));
        assert_eq!(strip.window_origin(strip.width + 48, 248), (strip.x - 24, strip.y - 24));
    }

    #[test]
    fn settings_are_forced_into_range() {
        let odd = Config { layout: "spiral".into(), corner: "middle".into(), monitor: "x\"; os.exit()".into(), backdrop: 7.0, text_scale: f32::NAN, items: vec!["cpu".into(), "nonsense".into()], open_at_start: true }.cleaned();
        assert_eq!((odd.layout.as_str(), odd.corner.as_str(), odd.monitor.as_str()), ("auto", "top-right", "auto"));
        assert_eq!((odd.backdrop, odd.text_scale), (0.9, 1.0));
        assert_eq!(odd.items, ["cpu"]);
    }

    #[test]
    fn reads_frame_rate_lines_and_ignores_headers() {
        assert_eq!(parse_fps_line("59.5786,16.7846,10.8,25.9,8,65"), Some((59.5786, 16.7846)));
        assert_eq!(parse_fps_line("fps,frametime,cpu_load"), None);
        assert_eq!(parse_fps_line("EndeavourOS,AMD Ryzen 7"), None);
        assert_eq!(parse_fps_line(""), None);
        assert!(fps_launch_option().contains("alpha=0.0") && fps_launch_option().ends_with("mangohud %command%"));
    }
}
