//! The curated list of system settings Trace can change.
//!
//! Rules for what belongs here:
//! - it can be changed back (no one-way kernel switches),
//! - a wrong value can't stop the machine from booting (nothing touches the
//!   bootloader, kernel command line, initramfs, sudoers, PAM or disk layout),
//! - the description says what it does and when you would want it.
//!
//! Entries whose file, unit or option doesn't exist on a machine are simply
//! not shown there.

use crate::tweaks::{Custom, DropIn, Kind, Persist, Reader, Src, Tweak};

/// Category name and a one-line description, in display order.
pub const CATEGORIES: &[(&str, &str)] = &[
    ("Power & CPU", "Speed, heat and how hard the processor is allowed to work"),
    ("Laptop & Battery", "Charging, radios, sleep and what the lid and power button do"),
    ("Memory & Swap", "How the kernel juggles RAM, swap and caches"),
    ("Disk & Files", "How data is read from and written to your drives"),
    ("Network", "Connections, DNS and how traffic is handled"),
    ("Desktop & Windows", "Look and behaviour of windows, effects and apps"),
    ("Mouse, Keyboard & Touchpad", "Pointer speed, scrolling, typing and gestures"),
    ("Audio", "Sound quality, latency and the microphone"),
    ("Privacy & Security", "What programs and other machines are allowed to see and do"),
    ("Background Services", "Things that run without a window"),
    ("Developer", "Limits and switches that development tools run into"),
    ("System", "Clock, logs and shutdown behaviour"),
];

const POWER: &str = "Power & CPU";
const LAPTOP: &str = "Laptop & Battery";
const MEMORY: &str = "Memory & Swap";
const DISK: &str = "Disk & Files";
const NETWORK: &str = "Network";
const DESKTOP: &str = "Desktop & Windows";
const INPUT: &str = "Mouse, Keyboard & Touchpad";
const AUDIO: &str = "Audio";
const SECURITY: &str = "Privacy & Security";
const SERVICES: &str = "Background Services";
const DEVELOPER: &str = "Developer";
const SYSTEM: &str = "System";

const DISKS_SCHEDULER: &[&str] = &[
    "/sys/block/nvme*/queue/scheduler",
    "/sys/block/sd*/queue/scheduler",
    "/sys/block/mmcblk*/queue/scheduler",
    "/sys/block/vd*/queue/scheduler",
];
const DISKS_READ_AHEAD: &[&str] = &[
    "/sys/block/nvme*/queue/read_ahead_kb",
    "/sys/block/sd*/queue/read_ahead_kb",
    "/sys/block/mmcblk*/queue/read_ahead_kb",
    "/sys/block/vd*/queue/read_ahead_kb",
];

fn num(min: f64, max: f64, step: f64, unit: &'static str) -> Kind {
    Kind::Number { min, max, step, unit }
}

fn choice(options: &[(&'static str, &'static str)]) -> Kind {
    Kind::Choice(options.to_vec())
}

/// Choices read from the system when the setting is shown.
fn discovered() -> Kind {
    Kind::Choice(Vec::new())
}

fn sysctl(id: &'static str, cat: &'static str, title: &'static str, help: &'static str, kind: Kind) -> Tweak {
    // The id of a single-key sysctl is the key itself.
    let keys: &'static [&'static str] = Box::leak(Box::new([id]));
    Tweak::new(id, cat, title, help, Src::Sysctl(keys), kind)
}

fn sysctls(
    id: &'static str,
    cat: &'static str,
    title: &'static str,
    help: &'static str,
    keys: &'static [&'static str],
    kind: Kind,
) -> Tweak {
    Tweak::new(id, cat, title, help, Src::Sysctl(keys), kind)
}

fn sysfs(
    id: &'static str,
    cat: &'static str,
    title: &'static str,
    help: &'static str,
    globs: &'static [&'static str],
    kind: Kind,
) -> Tweak {
    Tweak::new(id, cat, title, help, Src::Sysfs(globs), kind)
}

fn hypr(id: &'static str, cat: &'static str, title: &'static str, help: &'static str, option: &'static str, kind: Kind) -> Tweak {
    Tweak::new(id, cat, title, help, Src::Hypr(option), kind).onoff("true", "false")
}

fn gtk(id: &'static str, cat: &'static str, title: &'static str, help: &'static str, schema: &'static str, key: &'static str, kind: Kind) -> Tweak {
    Tweak::new(id, cat, title, help, Src::Gsettings(schema, key), kind).onoff("true", "false")
}

fn custom(id: &'static str, cat: &'static str, title: &'static str, help: &'static str, which: Custom, kind: Kind) -> Tweak {
    Tweak::new(id, cat, title, help, Src::Custom(which), kind)
}

/// A system service or timer that is switched on and off as a whole.
fn service(id: &'static str, title: &'static str, help: &'static str, units: &'static [&'static str]) -> Tweak {
    Tweak::new(id, SERVICES, title, help, Src::Unit { units, user: false, now: true }, Kind::Toggle)
}

fn dropin(
    id: &'static str,
    cat: &'static str,
    title: &'static str,
    help: &'static str,
    d: DropIn,
    options: &[(&'static str, &'static str)],
) -> Tweak {
    Tweak::new(id, cat, title, help, Src::DropIn(d), choice(options))
}

const LOGIND_FILE: &str = "/etc/systemd/logind.conf.d/99-trace.conf";
const LID_ACTIONS: &[(&str, &str)] = &[
    ("ignore", "Do nothing"),
    ("lock", "Lock the screen"),
    ("suspend", "Sleep"),
    ("poweroff", "Shut down"),
];

fn logind(id: &'static str, title: &'static str, help: &'static str, key: &'static str) -> Tweak {
    dropin(
        id,
        LAPTOP,
        title,
        help,
        DropIn {
            file: LOGIND_FILE,
            section: "Login",
            key,
            reader: Reader::Logind(key),
            reload: "systemctl reload systemd-logind",
        },
        LID_ACTIONS,
    )
    .careful()
}

pub fn build() -> Vec<Tweak> {
    let mut t: Vec<Tweak> = Vec::new();

    // ---------------------------------------------------------------- Power & CPU
    t.push(custom("power.profile", POWER, "Power profile",
        "The system-wide trade-off between speed, heat and battery life. Your desktop's quick settings use the same switch, and the system remembers it.",
        Custom::PowerProfile, discovered()).persist(Persist::Inherent));
    t.push(sysfs("cpu.boost", POWER, "Turbo boost",
        "Lets cores run above their rated speed in short bursts. Off makes the machine noticeably cooler and quieter and costs peak speed.",
        &["/sys/devices/system/cpu/cpufreq/boost"], Kind::Toggle));
    t.push(sysfs("cpu.turbo_intel", POWER, "Turbo boost",
        "Lets cores run above their rated speed in short bursts. Off makes the machine noticeably cooler and quieter and costs peak speed.",
        &["/sys/devices/system/cpu/intel_pstate/no_turbo"], Kind::Toggle).onoff("0", "1"));
    t.push(custom("cpu.max_freq", POWER, "Cap CPU clock speed",
        "Limits how fast any core may run, as a share of its top speed. The bluntest way to cut heat and fan noise; 100% removes the cap.",
        Custom::CpuMaxFreq, num(20.0, 100.0, 5.0, "%")).persist(Persist::Boot));
    t.push(sysfs("cpu.governor", POWER, "CPU governor",
        "The kernel's rule for picking clock speed: performance holds it high, powersave lets it follow the load. While the power-profile daemon is running, switching power profile can change this back.",
        &["/sys/devices/system/cpu/cpu*/cpufreq/scaling_governor"], discovered())
        .options_from("/sys/devices/system/cpu/cpu0/cpufreq/scaling_available_governors").persist(Persist::Never));
    t.push(sysfs("cpu.energy_preference", POWER, "CPU energy preference",
        "A hint to the processor itself about how eagerly to speed up. Can't be changed while the governor is performance, and the power-profile daemon also sets it.",
        &["/sys/devices/system/cpu/cpu*/cpufreq/energy_performance_preference"], discovered())
        .options_from("/sys/devices/system/cpu/cpu0/cpufreq/energy_performance_available_preferences").persist(Persist::Never));
    t.push(sysfs("platform.profile", POWER, "Fan and thermal profile",
        "Your laptop firmware's own mode: it sets fan curves and power limits together. Quiet trades speed for silence.",
        &["/sys/firmware/acpi/platform_profile"], discovered())
        .options_from("/sys/firmware/acpi/platform_profile_choices"));
    t.push(sysfs("cpu.smt", POWER, "Hyper-threading (SMT)",
        "Each core presents itself as two. Off halves the number of logical cores: slower for heavy multitasking, slightly cooler, and a little safer against some CPU side-channel attacks.",
        &["/sys/devices/system/cpu/smt/control"], Kind::Toggle).onoff("on", "off").careful());
    t.push(sysfs("cpu.amd_pstate", POWER, "AMD P-state driver mode",
        "How Linux talks to an AMD processor about speed. Active lets the CPU decide (the default), guided gives it a range, passive puts the kernel in charge. Only change this to compare behaviour.",
        &["/sys/devices/system/cpu/amd_pstate/status"],
        choice(&[("active", "Active (CPU decides)"), ("guided", "Guided"), ("passive", "Passive (kernel decides)")]))
        .expert().persist(Persist::Never));
    t.push(sysctl("kernel.nmi_watchdog", POWER, "Kernel lock-up detector",
        "A timer that fires many times a second on every core to notice a frozen kernel. Off saves a little power; only kernel developers miss it.",
        Kind::Toggle));
    t.push(sysctl("kernel.sched_autogroup_enabled", POWER, "Group tasks by terminal session",
        "Treats everything started from one terminal as a single competitor for CPU time, so a 32-job build doesn't starve your browser. Keep on for a desktop.",
        Kind::Toggle));
    t.push(sysctl("kernel.split_lock_mitigate", POWER, "Slow down split-lock programs",
        "Some programs (a number of games among them) use a slow memory trick called a split lock; the kernel deliberately throttles them. Turn off if a game stutters and the kernel log mentions split locks.",
        Kind::Toggle));
    t.push(sysfs("pcie.aspm", POWER, "PCIe link power saving",
        "Lets idle links to the SSD, Wi-Fi card and GPU drop into low-power states. More saving means slightly slower wake-ups; a few devices misbehave on the deepest setting.",
        &["/sys/module/pcie_aspm/parameters/policy"], discovered()).careful());
    t.push(sysfs("pci.runtime_pm", POWER, "Let idle PCI devices sleep",
        "Allows every internal device to power down when unused, instead of only the ones the driver opted in. Worth a few watts on battery; undo it if a device starts dropping out.",
        &["/sys/bus/pci/devices/*/power/control"], Kind::Toggle).onoff("auto", "on").careful());
    t.push(sysfs("usb.runtime_pm", POWER, "Let idle USB devices sleep",
        "Allows every USB device to suspend when idle. Saves power; some mice and keyboards take a moment to wake, in which case turn it back off.",
        &["/sys/bus/usb/devices/*/power/control"], Kind::Toggle).onoff("auto", "on").careful());
    t.push(sysfs("usb.autosuspend_delay", POWER, "USB idle time before sleep",
        "Seconds a USB device must be idle before it may suspend. -1 never suspends. Applies to devices plugged in from now on.",
        &["/sys/module/usbcore/parameters/autosuspend"], num(-1.0, 60.0, 1.0, "s")).effect("for newly connected devices"));
    t.push(sysfs("sata.link_power", POWER, "SATA link power saving",
        "How aggressively SATA drives' links power down. The middle settings are safe; the lowest-power one has caused data corruption on a few old drives.",
        &["/sys/class/scsi_host/host*/link_power_management_policy"],
        choice(&[
            ("max_performance", "Maximum performance"),
            ("medium_power", "Medium"),
            ("med_power_with_dipm", "Medium with device-initiated saving (recommended)"),
            ("min_power_with_partial", "Low"),
            ("min_power", "Lowest (risky on old drives)"),
        ])).careful());
    t.push(sysfs("power.mem_sleep", POWER, "Sleep type",
        "What \"sleep\" means. s2idle keeps the machine half-awake for an instant resume; deep powers nearly everything off and saves far more battery, if your laptop wakes from it reliably.",
        &["/sys/power/mem_sleep"], discovered()).careful().effect("the next time it sleeps"));
    t.push(sysfs("fan.full_speed", POWER, "Fans at full speed",
        "Pins every fan at maximum instead of letting the firmware decide. Loud, and the coolest the machine can run. Off hands control back to the firmware.",
        &["/sys/class/hwmon/hwmon*/pwm*_enable"], Kind::Toggle).onoff("0", "2").persist(Persist::Never));
    t.push(custom("power.keep_awake", POWER, "Keep awake",
        "Stops the machine from sleeping or blanking the screen on idle — for a long download, a presentation or a film. Lasts until you turn it off or Trace quits.",
        Custom::KeepAwake, Kind::Toggle));

    // ---------------------------------------------------------------- Laptop & Battery
    t.push(sysfs("battery.charge_limit", LAPTOP, "Stop charging at",
        "The charge level at which the battery stops filling. Holding a battery at 100% ages it fastest; 80% roughly doubles its useful life if you are mostly plugged in.",
        &["/sys/class/power_supply/BAT*/charge_control_end_threshold"], num(50.0, 100.0, 5.0, "%")));
    t.push(custom("laptop.kbd_backlight", LAPTOP, "Keyboard backlight",
        "Brightness of the lights under the keys. 0 is off.",
        Custom::KeyboardBacklight, num(0.0, 3.0, 1.0, "")));
    t.push(custom("wifi.power_save", LAPTOP, "Wi-Fi power saving",
        "Lets the Wi-Fi card doze between packets. Saves battery; turn off if video calls stutter or SSH feels laggy. NetworkManager may switch it back when you reconnect.",
        Custom::WifiPowerSave, Kind::Toggle).persist(Persist::Boot));
    t.push(custom("radio.wifi", LAPTOP, "Wi-Fi radio",
        "The Wi-Fi transmitter itself. Off is what aeroplane mode does for Wi-Fi.",
        Custom::Radio("wlan"), Kind::Toggle).persist(Persist::Inherent));
    t.push(custom("radio.bluetooth", LAPTOP, "Bluetooth radio",
        "The Bluetooth transmitter. Off saves a little power and stops the laptop from being discoverable.",
        Custom::Radio("bluetooth"), Kind::Toggle).persist(Persist::Inherent));
    t.push(sysfs("audio.power_save", LAPTOP, "Sound card idle power-down",
        "Seconds of silence before the sound chip powers down. 0 keeps it always on, which stops the click or pop some speakers make when sound starts.",
        &["/sys/module/snd_hda_intel/parameters/power_save"], num(0.0, 60.0, 1.0, "s")));
    t.push(sysfs("audio.power_save_controller", LAPTOP, "Power down the sound controller too",
        "When the sound chip sleeps, also sleep the controller it hangs off. Saves a little more; off if audio is slow to start.",
        &["/sys/module/snd_hda_intel/parameters/power_save_controller"], Kind::Toggle).onoff("Y", "N"));
    t.push(logind("logind.lid_battery", "When the lid closes on battery",
        "What the system does when you shut the lid while unplugged. If your desktop runs its own idle manager it may act as well.",
        "HandleLidSwitch"));
    t.push(logind("logind.lid_ac", "When the lid closes while plugged in",
        "What the system does when you shut the lid on mains power — \"do nothing\" suits a laptop used with an external monitor.",
        "HandleLidSwitchExternalPower"));
    t.push(logind("logind.power_key", "When the power button is pressed",
        "A short press of the power button. A long press always forces the machine off, whatever this says.",
        "HandlePowerKey"));

    // ---------------------------------------------------------------- Memory & Swap
    t.push(sysctl("vm.swappiness", MEMORY, "Swappiness",
        "How readily the kernel moves idle app memory to swap instead of dropping cached files. Low (10–60) suits a disk swap; high (100–180) suits compressed-RAM swap (zram), where swapping is cheap.",
        num(0.0, 200.0, 5.0, "")));
    t.push(sysctl("vm.vfs_cache_pressure", MEMORY, "File-name cache pressure",
        "How eagerly the kernel forgets where files are on disk. Lower (50) keeps folder listings snappy at the cost of some RAM; 100 is neutral.",
        num(10.0, 500.0, 10.0, "")));
    t.push(sysctl("vm.page-cluster", MEMORY, "Swap read-ahead",
        "When reading one page back from swap, how many neighbours to fetch too (as a power of two). 0 is right for zram and SSDs; 3 only helps spinning disks.",
        num(0.0, 5.0, 1.0, "")));
    t.push(sysctl("vm.watermark_scale_factor", MEMORY, "Start reclaiming memory earlier",
        "How much free RAM the kernel keeps in reserve before it starts cleaning up in the background. Raising it (125 is common with zram) trades a little usable RAM for fewer sudden freezes when memory runs short.",
        num(10.0, 1000.0, 5.0, "")).careful());
    t.push(sysctl("vm.watermark_boost_factor", MEMORY, "Reclaim boost after fragmentation",
        "After memory gets fragmented the kernel temporarily reclaims far more than it needs. With zram that causes needless swapping, so it is usually turned off.",
        choice(&[("0", "Off"), ("15000", "On (kernel default)")])).careful());
    t.push(sysctl("vm.compaction_proactiveness", MEMORY, "Background memory defragmentation",
        "How hard the kernel works in the background to keep large free blocks available. 0 removes its occasional latency spikes (some turn it off for gaming); 20 is the default.",
        num(0.0, 100.0, 5.0, "")));
    t.push(sysctl("vm.overcommit_memory", MEMORY, "Memory over-commit",
        "Whether programs may reserve more memory than exists. \"Never\" makes allocations fail honestly instead of the kernel killing something later — and breaks many desktop apps that reserve far more than they use.",
        choice(&[("0", "Heuristic (default)"), ("1", "Always allow"), ("2", "Never over-commit")])).expert());
    t.push(sysctl("vm.oom_kill_allocating_task", MEMORY, "On out-of-memory, kill whoever asked",
        "When memory runs out, kill the process that made the failing request instead of searching for the biggest offender. Faster, but it can kill an innocent bystander.",
        Kind::Toggle).careful());
    t.push(sysctl("vm.max_map_count", MEMORY, "Memory-map limit per program",
        "How many separate memory regions one program may have. Some games (through Proton) and databases like Elasticsearch need a high value; there is no cost to raising it.",
        choice(&[("65530", "65,530 (old kernel default)"), ("262144", "262,144"), ("1048576", "1,048,576"), ("2147483642", "No practical limit")])));
    t.push(sysfs("thp.enabled", MEMORY, "Transparent huge pages",
        "Lets the kernel back memory with 2 MB pages instead of 4 KB ones. Faster for big-memory programs and games; \"madvise\" only does it for programs that ask.",
        &["/sys/kernel/mm/transparent_hugepage/enabled"], discovered()));
    t.push(sysfs("thp.defrag", MEMORY, "Huge-page defragmentation",
        "How hard the kernel tries to make room for huge pages when a program asks. \"always\" can stall the program while it compacts memory; \"madvise\" and \"defer\" avoid that.",
        &["/sys/kernel/mm/transparent_hugepage/defrag"], discovered()));
    t.push(sysfs("mglru.enabled", MEMORY, "Multi-generation memory reclaim (MGLRU)",
        "The newer algorithm for deciding which memory to evict. It handles memory pressure much better on desktops; turn it off only to compare.",
        &["/sys/kernel/mm/lru_gen/enabled"], Kind::Toggle).onoff("0x0007", "0x0000").write_onoff("y", "n").expert());
    t.push(sysfs("mglru.min_ttl", MEMORY, "Protect recently used memory",
        "Milliseconds of recently used memory the kernel refuses to evict. Set (1000 is typical) the machine kills a process when RAM truly runs out instead of freezing while it thrashes; 0 turns the protection off.",
        &["/sys/kernel/mm/lru_gen/min_ttl_ms"], num(0.0, 10000.0, 250.0, "ms")).careful());
    t.push(sysfs("zswap.enabled", MEMORY, "zswap (compressed cache in front of swap)",
        "Compresses pages in RAM before they would reach the swap disk. Great with a disk swap; redundant if your swap is already zram, where it compresses everything twice.",
        &["/sys/module/zswap/parameters/enabled"], Kind::Toggle).onoff("Y", "N"));
    t.push(sysfs("zswap.compressor", MEMORY, "zswap compressor",
        "zstd squeezes hardest, lz4 is fastest, lzo is in between.",
        &["/sys/module/zswap/parameters/compressor"],
        choice(&[("zstd", "zstd (best ratio)"), ("lz4", "lz4 (fastest)"), ("lzo", "lzo")])));
    t.push(sysfs("zswap.max_pool_percent", MEMORY, "zswap pool size limit",
        "The most RAM zswap's compressed pool may take, as a percentage of total memory.",
        &["/sys/module/zswap/parameters/max_pool_percent"], num(5.0, 50.0, 5.0, "%")));
    t.push(sysfs("zswap.shrinker", MEMORY, "Let zswap shrink under pressure",
        "Allows zswap to push its coldest pages out to the real swap device when memory is tight, instead of only when its pool is full.",
        &["/sys/module/zswap/parameters/shrinker_enabled"], Kind::Toggle).onoff("Y", "N"));
    t.push(sysfs("ksm.run", MEMORY, "Merge identical memory pages (KSM)",
        "A background scanner that merges duplicate pages. Only affects programs that opt in — in practice virtual machines, where it can save a lot.",
        &["/sys/kernel/mm/ksm/run"], Kind::Toggle));
    t.push(sysctl("vm.dirty_ratio", MEMORY, "Unsaved-data limit (hard)",
        "Share of memory that may hold data not yet written to disk before programs that write are made to wait. Lower means smaller, more frequent writes and less lost on a crash.",
        num(5.0, 60.0, 5.0, "%")));
    t.push(sysctl("vm.dirty_background_ratio", MEMORY, "Unsaved-data limit (background)",
        "Share of memory with unwritten data at which the kernel starts writing it out in the background. Should stay below the hard limit.",
        num(1.0, 30.0, 1.0, "%")));
    t.push(sysctl("vm.dirty_writeback_centisecs", MEMORY, "How often unsaved data is flushed",
        "The interval at which the kernel wakes to write out changed data. Longer lets the disk idle (battery); a crash can lose up to that much recent work.",
        choice(&[("500", "Every 5 seconds (default)"), ("1500", "Every 15 seconds"), ("3000", "Every 30 seconds"), ("6000", "Every minute")])));
    t.push(sysctl("vm.dirty_expire_centisecs", MEMORY, "Oldest unsaved data allowed",
        "How long changed data may sit in memory before it must be written. Pairs with the flush interval.",
        choice(&[("1000", "10 seconds"), ("3000", "30 seconds (default)"), ("6000", "1 minute")])));

    // ---------------------------------------------------------------- Disk & Files
    t.push(sysfs("disk.scheduler", DISK, "Disk I/O scheduler",
        "Who goes first when several programs want the disk. \"none\" is best for NVMe SSDs; bfq keeps the desktop responsive during heavy copying on slower drives; mq-deadline is a sound middle ground.",
        DISKS_SCHEDULER, discovered()));
    t.push(sysfs("disk.read_ahead", DISK, "Disk read-ahead",
        "How much extra data the kernel reads past what was asked for, betting it will be needed next. Larger helps big sequential reads (video, backups); smaller suits databases.",
        DISKS_READ_AHEAD, num(0.0, 8192.0, 64.0, "KB")));
    t.push(dropin("journald.max_use", DISK, "System journal size limit",
        "The most disk the system log may occupy. With no limit it grows to 10% of the disk (capped at 4 GB).",
        DropIn { file: "/etc/systemd/journald.conf.d/99-trace.conf", section: "Journal", key: "SystemMaxUse",
                 reader: Reader::CatConfig("systemd/journald.conf"), reload: "systemctl restart systemd-journald" },
        &[("", "Default (up to 4 GB)"), ("100M", "100 MB"), ("500M", "500 MB"), ("1G", "1 GB"), ("2G", "2 GB")]));
    t.push(dropin("coredump.storage", DISK, "Keep crash dumps",
        "When a program crashes, whether its memory image is saved for debugging. \"Log only\" still records that it crashed (Trace's crash log keeps working) without using disk.",
        DropIn { file: "/etc/systemd/coredump.conf.d/99-trace.conf", section: "Coredump", key: "Storage",
                 reader: Reader::CatConfig("systemd/coredump.conf"), reload: "" },
        &[("", "Default (save to disk)"), ("external", "Save to disk"), ("none", "Log only, save nothing")]).effect("for the next crash"));

    // ---------------------------------------------------------------- Network
    t.push(custom("net.dns", NETWORK, "DNS servers",
        "Who turns website names into addresses. \"Automatic\" uses whatever your router hands out (usually your internet provider). Applies to the network you are on now, and is remembered for it.",
        Custom::Dns, discovered()).persist(Persist::Inherent).effect("the next time this network connects"));
    t.push(sysctl("net.ipv4.tcp_congestion_control", NETWORK, "TCP congestion control",
        "How downloads and uploads pace themselves. BBR, from Google, holds speed far better than the default on Wi-Fi and other lossy links.",
        choice(&[("cubic", "CUBIC (default)"), ("bbr", "BBR"), ("reno", "Reno (classic)")]))
        .pre("modprobe tcp_bbr 2>/dev/null").effect("for new connections"));
    t.push(sysctl("net.core.default_qdisc", NETWORK, "Packet queue discipline",
        "How outgoing packets are ordered. fq_codel keeps latency low and is the default; fq pairs best with BBR; cake is the most thorough for a congested link.",
        choice(&[("fq_codel", "fq_codel (default)"), ("fq", "fq"), ("cake", "cake"), ("pfifo_fast", "pfifo_fast (legacy)")]))
        .pre("modprobe sch_fq 2>/dev/null; modprobe sch_cake 2>/dev/null").effect("for interfaces brought up afterwards"));
    t.push(sysctl("net.ipv4.tcp_fastopen", NETWORK, "TCP Fast Open",
        "Lets data ride along with the connection handshake, saving a round trip on repeat visits to a server.",
        choice(&[("0", "Off"), ("1", "Outgoing connections"), ("2", "Incoming connections"), ("3", "Both")])));
    t.push(sysctl("net.ipv4.tcp_mtu_probing", NETWORK, "Work around broken packet-size discovery",
        "Some networks and VPNs silently drop large packets, which shows up as sites that connect and then hang. Probing detects and adapts to that.",
        choice(&[("0", "Off"), ("1", "Only after a connection stalls"), ("2", "Always")])));
    t.push(sysctl("net.ipv4.tcp_slow_start_after_idle", NETWORK, "Restart slowly after a pause",
        "After a connection has been idle, begin again at low speed. Turning this off keeps long-lived connections (streams, SSH tunnels) fast after pauses.",
        Kind::Toggle));
    t.push(sysctl("net.ipv4.tcp_ecn", NETWORK, "Explicit congestion notification",
        "Lets routers signal congestion without dropping packets. Smoother under load; a handful of very old routers mishandle it.",
        choice(&[("0", "Off"), ("1", "Request and accept"), ("2", "Accept only (default)")])));
    t.push(sysctl("net.ipv4.tcp_timestamps", NETWORK, "TCP timestamps",
        "Timing information added to packets. It improves performance on fast links, and also lets a remote server estimate how long your machine has been up.",
        Kind::Toggle));
    t.push(sysctls("net.buffer_max", NETWORK, "Largest network buffer a program may request",
        "Programs moving a lot of data over UDP — QUIC/HTTP3, WireGuard, game streaming — ask for big buffers and are capped by this.",
        &["net.core.rmem_max", "net.core.wmem_max"],
        choice(&[("212992", "208 KB (kernel default)"), ("4194304", "4 MB"), ("7500000", "7.5 MB (what QUIC asks for)"), ("16777216", "16 MB")])));
    t.push(sysctl("net.ipv4.ip_forward", NETWORK, "Forward traffic between networks",
        "Lets this machine act as a router. Docker, virtual machines and Wi-Fi hotspots need it on and normally turn it on themselves.",
        Kind::Toggle).careful());
    t.push(sysctls("net.ipv6", NETWORK, "IPv6",
        "The newer internet addressing. Turning it off can work around a broken IPv6 connection from your provider, or stop traffic leaking around a VPN that only carries IPv4.",
        &["net.ipv6.conf.all.disable_ipv6", "net.ipv6.conf.default.disable_ipv6"], Kind::Toggle).onoff("0", "1").careful());
    t.push(custom("net.connectivity_check", NETWORK, "Internet connectivity check",
        "NetworkManager fetches a tiny page from your distribution's server every few minutes to detect Wi-Fi login pages. Off means one fewer regular outside contact; login pages won't open by themselves.",
        Custom::ConnectivityCheck, Kind::Toggle).persist(Persist::Inherent));
    t.push(dropin("wifi.mac_address", NETWORK, "Wi-Fi hardware address shown to networks",
        "Every Wi-Fi card has a permanent address that lets networks recognise you across visits. \"Stable per network\" gives each network its own made-up address; \"random\" changes it on every connection (which can re-trigger login pages).",
        DropIn { file: "/etc/NetworkManager/conf.d/99-trace.conf", section: "connection", key: "wifi.cloned-mac-address",
                 reader: Reader::NmConfig, reload: "nmcli general reload" },
        &[("", "Default (real address)"), ("permanent", "Real address"), ("stable", "Stable made-up address per network"), ("random", "New random address each time")])
        .careful().effect("the next time you connect"));
    t.push(custom("firewall.zone", NETWORK, "Firewall default zone",
        "How much the firewall trusts the networks you join. Public blocks incoming connections except the few you opened; home and trusted allow progressively more; drop silently ignores everything incoming.",
        Custom::FirewallZone,
        choice(&[("public", "Public"), ("home", "Home"), ("work", "Work"), ("trusted", "Trusted (allow everything)"), ("block", "Block (reject incoming)"), ("drop", "Drop (ignore incoming)")]))
        .persist(Persist::Inherent).careful());

    // ---------------------------------------------------------------- Desktop & Windows
    t.push(gtk("gtk.color_scheme", DESKTOP, "App colour scheme",
        "Whether apps that follow the system preference use their dark or light look.",
        "org.gnome.desktop.interface", "color-scheme",
        choice(&[("default", "No preference"), ("prefer-dark", "Dark"), ("prefer-light", "Light")])));
    t.push(gtk("gtk.animations", DESKTOP, "Animations inside apps",
        "Fades and slides within GTK apps (menus, dialogs). Off feels snappier and saves a little power.",
        "org.gnome.desktop.interface", "enable-animations", Kind::Toggle));
    t.push(gtk("gtk.text_scale", DESKTOP, "Text size in apps",
        "Scales text in GTK apps without changing the size of everything else.",
        "org.gnome.desktop.interface", "text-scaling-factor", num(0.8, 1.6, 0.05, "×")));
    t.push(gtk("gtk.clock_format", DESKTOP, "Clock format",
        "12-hour or 24-hour time in apps that follow the system setting.",
        "org.gnome.desktop.interface", "clock-format", choice(&[("12h", "12-hour"), ("24h", "24-hour")])));
    t.push(hypr("hypr.animations", DESKTOP, "Window animations",
        "Windows and workspaces slide and fade as they open, close and switch. Off makes everything instant.",
        "animations:enabled", Kind::Toggle));
    t.push(hypr("hypr.blur", DESKTOP, "Blur behind transparent windows",
        "Blurs whatever is behind see-through windows and panels. The most expensive effect for the graphics chip; off helps battery and older GPUs.",
        "decoration:blur:enabled", Kind::Toggle));
    t.push(hypr("hypr.blur_size", DESKTOP, "Blur strength",
        "How far the blur spreads. Larger looks softer.", "decoration:blur:size", num(1.0, 20.0, 1.0, "")));
    t.push(hypr("hypr.blur_passes", DESKTOP, "Blur quality",
        "How many times the blur is applied. Each pass is smoother and costs more graphics power.",
        "decoration:blur:passes", num(1.0, 4.0, 1.0, "")));
    t.push(hypr("hypr.blur_xray", DESKTOP, "Blur shows only the wallpaper",
        "Floating windows blur the wallpaper rather than the windows underneath them. Cheaper to draw.",
        "decoration:blur:xray", Kind::Toggle));
    t.push(hypr("hypr.shadow", DESKTOP, "Window shadows",
        "A soft shadow around each window.", "decoration:shadow:enabled", Kind::Toggle));
    t.push(hypr("hypr.rounding", DESKTOP, "Rounded corners",
        "Corner radius of windows in pixels. 0 is square.", "decoration:rounding", num(0.0, 30.0, 1.0, "px")));
    t.push(hypr("hypr.active_opacity", DESKTOP, "Focused window opacity",
        "1 is solid; lower lets the wallpaper show through the window you are working in.",
        "decoration:active_opacity", num(0.5, 1.0, 0.05, "")));
    t.push(hypr("hypr.inactive_opacity", DESKTOP, "Background window opacity",
        "Opacity of windows that don't have focus.", "decoration:inactive_opacity", num(0.5, 1.0, 0.05, "")));
    t.push(hypr("hypr.dim_inactive", DESKTOP, "Dim background windows",
        "Darkens windows that don't have focus so the active one stands out.", "decoration:dim_inactive", Kind::Toggle));
    t.push(hypr("hypr.dim_strength", DESKTOP, "Dimming amount",
        "How much background windows are darkened.", "decoration:dim_strength", num(0.0, 0.6, 0.05, "")));
    t.push(hypr("hypr.gaps_in", DESKTOP, "Gap between windows",
        "Empty space between neighbouring windows, in pixels.", "general:gaps_in", num(0.0, 30.0, 1.0, "px")));
    t.push(hypr("hypr.gaps_out", DESKTOP, "Gap at screen edges",
        "Empty space between windows and the edge of the screen, in pixels.", "general:gaps_out", num(0.0, 40.0, 1.0, "px")));
    t.push(hypr("hypr.border_size", DESKTOP, "Window border thickness",
        "Width of the coloured line around windows.", "general:border_size", num(0.0, 6.0, 1.0, "px")));
    t.push(hypr("hypr.resize_on_border", DESKTOP, "Resize by dragging window edges",
        "Drag a window's border or the gap beside it to resize, without holding a modifier key.",
        "general:resize_on_border", Kind::Toggle));
    t.push(hypr("hypr.allow_tearing", DESKTOP, "Allow screen tearing",
        "Lets games that ask for it show frames the instant they are ready instead of waiting for the screen. Lowest input delay, at the cost of visible tear lines.",
        "general:allow_tearing", Kind::Toggle));
    t.push(hypr("hypr.vrr", DESKTOP, "Adaptive sync (VRR / FreeSync / G-Sync)",
        "Lets the screen refresh in step with what is being drawn, which smooths games. On the desktop it can make brightness flicker on some panels, hence the full-screen-only choices.",
        "misc:vrr", choice(&[("0", "Off"), ("1", "Always"), ("2", "Full-screen windows only"), ("3", "Full-screen games and video only")])));
    t.push(hypr("hypr.direct_scanout", DESKTOP, "Direct scan-out for full-screen windows",
        "Hands a full-screen window straight to the display, skipping the compositor. Lower delay in games; occasionally causes glitches.",
        "render:direct_scanout", choice(&[("0", "Off"), ("1", "On"), ("2", "Games only")])));
    t.push(hypr("hypr.focus_on_activate", DESKTOP, "Let apps take focus",
        "When an app asks for attention (a link opening in the browser, say) jump to it instead of only marking it urgent.",
        "misc:focus_on_activate", Kind::Toggle));
    t.push(hypr("hypr.swallow", DESKTOP, "Terminal swallows the app it launched",
        "Start a graphical program from a terminal and the terminal window hides until the program closes.",
        "misc:enable_swallow", Kind::Toggle));
    t.push(hypr("hypr.wake_on_mouse", DESKTOP, "Mouse movement wakes the screen",
        "Moving the mouse turns a blanked screen back on.", "misc:mouse_move_enables_dpms", Kind::Toggle));
    t.push(hypr("hypr.wake_on_key", DESKTOP, "Key press wakes the screen",
        "Pressing a key turns a blanked screen back on.", "misc:key_press_enables_dpms", Kind::Toggle));
    t.push(hypr("hypr.animate_resize", DESKTOP, "Animate manual resizing",
        "Windows animate while you drag to resize them. Prettier, and noticeably heavier.",
        "misc:animate_manual_resizes", Kind::Toggle));
    t.push(hypr("hypr.animate_drag", DESKTOP, "Animate dragging windows",
        "Windows animate while being dragged with the mouse.", "misc:animate_mouse_windowdragging", Kind::Toggle));
    t.push(hypr("hypr.workspace_back_forth", DESKTOP, "Workspace key toggles back",
        "Pressing the shortcut for the workspace you are already on returns to the previous one.",
        "binds:workspace_back_and_forth", Kind::Toggle));
    t.push(hypr("hypr.preserve_split", DESKTOP, "Keep window split direction",
        "A split between two tiled windows stays vertical or horizontal as you resize, instead of flipping to fit.",
        "dwindle:preserve_split", Kind::Toggle));
    t.push(hypr("hypr.xwayland_zero_scale", DESKTOP, "Sharp older (X11) apps on scaled screens",
        "Older apps render at full resolution instead of being stretched and blurry on a scaled display. They may then look small unless they scale themselves.",
        "xwayland:force_zero_scaling", Kind::Toggle));
    t.push(hypr("hypr.hardware_cursor", DESKTOP, "Mouse cursor rendering",
        "The hardware cursor is smoothest. If the pointer disappears, flickers or leaves trails (common with NVIDIA), switch to software.",
        "cursor:no_hardware_cursors", choice(&[("0", "Hardware"), ("1", "Software"), ("2", "Automatic")])));
    t.push(hypr("hypr.cursor_timeout", DESKTOP, "Hide the cursor when idle",
        "Seconds without movement before the pointer hides. 0 never hides it.", "cursor:inactive_timeout", num(0.0, 30.0, 1.0, "s")));
    t.push(hypr("hypr.cursor_hide_typing", DESKTOP, "Hide the cursor while typing",
        "The pointer disappears when you press a key and returns when you move the mouse.",
        "cursor:hide_on_key_press", Kind::Toggle));
    t.push(hypr("hypr.zoom", DESKTOP, "Screen magnifier",
        "Zooms the whole screen around the pointer. 1 is normal size.", "cursor:zoom_factor", num(1.0, 4.0, 0.25, "×")).persist(Persist::Never));
    t.push(hypr("hypr.nvidia_anti_flicker", DESKTOP, "NVIDIA anti-flicker",
        "A workaround for flicker on NVIDIA cards that costs a little performance. Has no effect on other GPUs.",
        "opengl:nvidia_anti_flicker", Kind::Toggle));
    t.push(hypr("hypr.fps_overlay", DESKTOP, "Frame-time overlay",
        "Draws frames-per-second and frame-time graphs in the corner of every monitor. Useful for checking whether an effect is costing you smoothness.",
        "debug:overlay", Kind::Toggle).persist(Persist::Never));
    t.push(hypr("hypr.damage_blink", DESKTOP, "Flash redrawn areas",
        "Flashes every region of the screen as it is redrawn. A debugging aid that also shows which app keeps repainting; unpleasant to leave on.",
        "debug:damage_blink", Kind::Toggle).persist(Persist::Never).expert());
    t.push(hypr("hypr.no_update_news", DESKTOP, "Hide Hyprland update news",
        "Stops the pop-up shown after Hyprland is updated.", "ecosystem:no_update_news", Kind::Toggle));
    t.push(hypr("hypr.no_donation_nag", DESKTOP, "Hide Hyprland donation reminders",
        "Stops the occasional donation reminder.", "ecosystem:no_donation_nag", Kind::Toggle));
    t.push(hypr("hypr.disable_autoreload", DESKTOP, "Don't reload config on save",
        "Normally Hyprland re-reads its config the moment a file changes, which also undoes session-only changes made here. On means it only reloads when asked.",
        "misc:disable_autoreload", Kind::Toggle));

    // ---------------------------------------------------------------- Mouse, Keyboard & Touchpad
    t.push(hypr("input.sensitivity", INPUT, "Pointer speed",
        "0 is the device's own speed; negative is slower, positive faster.", "input:sensitivity", num(-1.0, 1.0, 0.05, "")));
    t.push(hypr("input.accel_profile", INPUT, "Pointer acceleration",
        "Adaptive moves the pointer further when you move the mouse quickly; flat is strictly proportional, which most people prefer for games.",
        "input:accel_profile", choice(&[("", "Device default"), ("adaptive", "Adaptive"), ("flat", "Flat (no acceleration)")])));
    t.push(hypr("input.follow_mouse", INPUT, "Focus follows the mouse",
        "Whether pointing at a window is enough to give it keyboard focus.",
        "input:follow_mouse", choice(&[("0", "Click to focus"), ("1", "Focus whatever is under the pointer"), ("2", "Pointer moves freely, click to focus"), ("3", "Keyboard and pointer focus are separate")])));
    t.push(hypr("input.natural_scroll", INPUT, "Natural scrolling (mouse)",
        "The page moves the way your fingers do on the wheel, as on a phone.", "input:natural_scroll", Kind::Toggle));
    t.push(hypr("input.scroll_factor", INPUT, "Mouse scroll speed",
        "Multiplier for how far one notch of the wheel scrolls.", "input:scroll_factor", num(0.25, 3.0, 0.25, "×")));
    t.push(hypr("input.left_handed", INPUT, "Left-handed mouse",
        "Swaps the left and right buttons.", "input:left_handed", Kind::Toggle));
    t.push(hypr("input.repeat_rate", INPUT, "Key repeat speed",
        "How many times a second a held key repeats.", "input:repeat_rate", num(10.0, 80.0, 5.0, "/s")));
    t.push(hypr("input.repeat_delay", INPUT, "Delay before a key repeats",
        "How long a key must be held before it starts repeating.", "input:repeat_delay", num(150.0, 800.0, 50.0, "ms")));
    t.push(hypr("input.numlock", INPUT, "Num Lock on at start",
        "Turns the number pad on when the session starts.", "input:numlock_by_default", Kind::Toggle));
    t.push(hypr("input.middle_click_paste", INPUT, "Middle-click pastes selection",
        "Whatever text you last selected is pasted by a middle click, without copying it first. Classic on Linux, and a source of accidental pastes.",
        "misc:middle_click_paste", Kind::Toggle));
    t.push(gtk("gtk.primary_paste", INPUT, "Middle-click paste in apps",
        "The same middle-click paste, as far as GTK apps are concerned.",
        "org.gnome.desktop.interface", "gtk-enable-primary-paste", Kind::Toggle));
    t.push(hypr("touchpad.natural_scroll", INPUT, "Natural scrolling (touchpad)",
        "Two-finger scrolling moves the page with your fingers.", "input:touchpad:natural_scroll", Kind::Toggle));
    t.push(hypr("touchpad.tap_to_click", INPUT, "Tap to click",
        "A light tap on the touchpad counts as a click.", "input:touchpad:tap-to-click", Kind::Toggle));
    t.push(hypr("touchpad.tap_and_drag", INPUT, "Tap and drag",
        "Tap, then tap again and hold, to drag without pressing the pad down.", "input:touchpad:tap-and-drag", Kind::Toggle));
    t.push(hypr("touchpad.drag_lock", INPUT, "Drag lock",
        "After a tap-and-drag, lifting your finger briefly doesn't drop what you are dragging.",
        "input:touchpad:drag_lock", choice(&[("0", "Off"), ("1", "Release after a short pause"), ("2", "Stay locked until you tap")])));
    t.push(hypr("touchpad.disable_while_typing", INPUT, "Ignore the touchpad while typing",
        "Stops your palm from moving the pointer or clicking while you type.", "input:touchpad:disable_while_typing", Kind::Toggle));
    t.push(hypr("touchpad.scroll_factor", INPUT, "Touchpad scroll speed",
        "Multiplier for two-finger scrolling.", "input:touchpad:scroll_factor", num(0.1, 2.0, 0.1, "×")));
    t.push(hypr("touchpad.clickfinger", INPUT, "Click with two or three fingers",
        "Pressing with two fingers is a right-click and with three a middle-click, wherever they are on the pad — instead of using the pad's corners.",
        "input:touchpad:clickfinger_behavior", Kind::Toggle));
    t.push(hypr("touchpad.middle_emulation", INPUT, "Left + right click = middle click",
        "Pressing both buttons together sends a middle click.", "input:touchpad:middle_button_emulation", Kind::Toggle));

    // ---------------------------------------------------------------- Audio
    t.push(custom("audio.rate", AUDIO, "Sample rate",
        "The rate the sound system runs at. Automatic follows what is playing; forcing a rate avoids resampling for one kind of material.",
        Custom::AudioRate,
        choice(&[("0", "Automatic"), ("44100", "44.1 kHz (CD)"), ("48000", "48 kHz (video, games)"), ("88200", "88.2 kHz"), ("96000", "96 kHz"), ("192000", "192 kHz")]))
        .persist(Persist::Session));
    t.push(custom("audio.quantum", AUDIO, "Latency (buffer size)",
        "How much audio is processed at a time. Smaller means less delay (calls, instruments, rhythm games) and more risk of crackling; larger is safest. Automatic lets each app ask.",
        Custom::AudioQuantum,
        choice(&[("0", "Automatic"), ("64", "64 — lowest delay"), ("128", "128"), ("256", "256"), ("512", "512"), ("1024", "1024"), ("2048", "2048 — most stable")]))
        .persist(Persist::Session));
    t.push(custom("audio.mic_mute", AUDIO, "Mute the microphone",
        "Mutes the default microphone for every app at once.", Custom::MicMute, Kind::Toggle));
    t.push(gtk("gtk.event_sounds", AUDIO, "App sound effects",
        "Short sounds apps play for alerts and events.", "org.gnome.desktop.sound", "event-sounds", Kind::Toggle));

    // ---------------------------------------------------------------- Privacy & Security
    t.push(custom("privacy.webcam", SECURITY, "Webcam",
        "Loads or unloads the driver for USB webcams (the built-in one included). Off makes the camera vanish for every app until you turn it back on; it can't be turned off while in use.",
        Custom::Webcam, Kind::Toggle).careful());
    t.push(sysfs("usb.authorize_new", SECURITY, "Accept newly plugged-in USB devices",
        "Off means anything plugged in from now on is ignored until you approve it under Control → Display & Devices. Protects an unattended laptop from malicious USB sticks; remember it also ignores a new keyboard.",
        &["/sys/bus/usb/devices/usb*/authorized_default"], Kind::Toggle).careful());
    t.push(gtk("gtk.recent_files", SECURITY, "Remember recently used files",
        "Apps keep a list of files you opened lately, shown in file pickers.",
        "org.gnome.desktop.privacy", "remember-recent-files", Kind::Toggle));
    t.push(sysctl("net.ipv4.icmp_echo_ignore_all", SECURITY, "Ignore pings",
        "The machine stops answering ping, which hides it from casual network scans. It also makes it harder for you to check whether it is reachable.",
        Kind::Toggle));
    t.push(sysctl("kernel.dmesg_restrict", SECURITY, "Kernel log readable by administrators only",
        "The kernel log can reveal memory addresses useful to an attacker. On means `dmesg` needs sudo.",
        Kind::Toggle));
    t.push(sysctl("kernel.kptr_restrict", SECURITY, "Hide kernel memory addresses",
        "Whether kernel addresses are shown in system files. Hiding them makes kernel exploits harder; profiling tools need them visible.",
        choice(&[("0", "Shown"), ("1", "Hidden from normal users"), ("2", "Hidden from everyone")])));
    t.push(sysctl("kernel.yama.ptrace_scope", SECURITY, "Who may inspect running programs",
        "Controls debuggers and tools that read another process's memory. The default lets a program inspect only what it started; attaching gdb or strace to an already-running program needs the open setting.",
        choice(&[("0", "Any of your programs"), ("1", "Only programs it started (default)"), ("2", "Administrators only")])));
    t.push(sysctl("kernel.unprivileged_bpf_disabled", SECURITY, "eBPF for normal programs",
        "eBPF lets programs load small pieces of code into the kernel. Restricting it to administrators closes a frequent source of kernel bugs.",
        choice(&[("0", "Allowed"), ("2", "Administrators only")])));
    t.push(sysctl("kernel.sysrq", SECURITY, "Magic SysRq key",
        "Alt+SysRq key combinations that talk straight to the kernel — the last resort for a frozen machine (the \"REISUB\" sequence). Anyone at the keyboard can use them.",
        choice(&[("0", "Off"), ("16", "Sync disks only"), ("176", "Sync, remount read-only and reboot"), ("1", "Everything")])));
    t.push(sysctls("net.accept_redirects", SECURITY, "Accept route redirects",
        "Lets other machines on the network tell yours to send traffic a different way. Only real routers should, and it is a classic way to intercept traffic on a shared network.",
        &["net.ipv4.conf.all.accept_redirects", "net.ipv4.conf.default.accept_redirects", "net.ipv6.conf.all.accept_redirects", "net.ipv6.conf.default.accept_redirects"],
        Kind::Toggle));
    t.push(sysctls("net.send_redirects", SECURITY, "Send route redirects",
        "Whether this machine tells others about better routes. Only a router needs to.",
        &["net.ipv4.conf.all.send_redirects", "net.ipv4.conf.default.send_redirects"], Kind::Toggle));
    t.push(sysctl("net.ipv4.tcp_syncookies", SECURITY, "SYN-flood protection",
        "Keeps accepting connections during a flood of fake connection attempts. No reason to turn it off.",
        Kind::Toggle));
    t.push(sysctl("net.ipv4.tcp_rfc1337", SECURITY, "Protect closing connections",
        "Ignores a kind of stray packet that can wrongly kill a connection that is closing down.",
        Kind::Toggle));
    t.push(sysctls("net.log_martians", SECURITY, "Log impossible packets",
        "Records packets that claim an address they can't have come from. Useful when hunting a network problem; noisy otherwise.",
        &["net.ipv4.conf.all.log_martians", "net.ipv4.conf.default.log_martians"], Kind::Toggle));
    t.push(sysctls("net.rp_filter", SECURITY, "Reverse-path filtering",
        "Drops packets arriving on an interface they couldn't be answered through. Strict breaks some VPN and multi-network setups; loose is the usual compromise.",
        &["net.ipv4.conf.all.rp_filter", "net.ipv4.conf.default.rp_filter"],
        choice(&[("0", "Off"), ("1", "Strict"), ("2", "Loose")])).expert());
    t.push(sysctl("fs.protected_symlinks", SECURITY, "Restrict symlinks in shared folders",
        "Blocks a classic trick of planting a link in /tmp to make another user's program overwrite the wrong file.",
        Kind::Toggle).expert());
    t.push(sysctl("fs.protected_hardlinks", SECURITY, "Restrict hard links to others' files",
        "Stops users from creating hard links to files they don't own.", Kind::Toggle).expert());
    t.push(sysctl("fs.protected_fifos", SECURITY, "Restrict pipes in shared folders",
        "Protects against planted named pipes in world-writable folders such as /tmp.",
        choice(&[("0", "Off"), ("1", "Protect in sticky folders"), ("2", "Protect in group-writable folders too")])).expert());
    t.push(sysctl("fs.protected_regular", SECURITY, "Restrict files in shared folders",
        "Protects against planted regular files in world-writable folders such as /tmp.",
        choice(&[("0", "Off"), ("1", "Protect in sticky folders"), ("2", "Protect in group-writable folders too")])).expert());
    t.push(sysctl("fs.suid_dumpable", SECURITY, "Crash dumps of privileged programs",
        "Whether programs that run with elevated rights may leave a memory dump when they crash. Such dumps can contain secrets.",
        choice(&[("0", "Never"), ("1", "Always"), ("2", "Only readable by the administrator")])).expert());
    t.push(sysctl("dev.tty.legacy_tiocsti", SECURITY, "Let programs type into your terminal",
        "An old feature that lets a program inject keystrokes into the terminal it runs in — used by a few tools and by privilege-escalation tricks.",
        Kind::Toggle).expert());
    t.push(sysctl("kernel.unprivileged_userns_clone", SECURITY, "Sandboxes for normal programs",
        "Lets programs create their own isolated environments without administrator rights. Browsers, Flatpak, Steam and rootless containers depend on it; it also widens the kernel's attack surface.",
        Kind::Toggle).expert());
    t.push(sysctl("kernel.io_uring_disabled", SECURITY, "io_uring",
        "A fast interface for file and network I/O that has had a long run of security bugs. Restricting it breaks software that relies on it.",
        choice(&[("0", "Allowed for everyone"), ("1", "Administrators only"), ("2", "Off entirely")])).expert());
    t.push(sysctl("vm.unprivileged_userfaultfd", SECURITY, "userfaultfd for normal programs",
        "A memory-handling feature used by virtual machines and exploited in several kernel attacks.",
        Kind::Toggle).expert());
    t.push(sysctl("kernel.randomize_va_space", SECURITY, "Randomise memory layout (ASLR)",
        "Loads programs at unpredictable addresses so attacks can't rely on knowing where things are. Only turn down to reproduce a bug.",
        choice(&[("0", "Off"), ("1", "Partial"), ("2", "Full (default)")])).expert());

    // ---------------------------------------------------------------- Background Services
    t.push(service("svc.bluetooth", "Bluetooth service",
        "The service behind all Bluetooth devices. Off, nothing Bluetooth works; the radio toggle is the lighter switch.",
        &["bluetooth.service"]));
    t.push(service("svc.printing", "Printing",
        "CUPS, the print system. Safe to turn off if you never print.", &["cups.socket", "cups.service"]));
    t.push(service("svc.printer_discovery", "Automatic printer discovery",
        "Finds and adds network printers by itself. Has a history of security holes; off if you don't need it.",
        &["cups-browsed.service"]));
    t.push(service("svc.ssh", "SSH server (remote login)",
        "Lets you log in to this machine from another over the network. Off unless you use it.", &["sshd.service"]).careful());
    t.push(service("svc.firewall", "Firewall",
        "Blocks unrequested incoming connections. Should be on for any laptop that joins networks you don't control.",
        &["firewalld.service"]).careful());
    t.push(service("svc.avahi", "Local network discovery (mDNS)",
        "Finds printers, speakers and other machines by name on your network (the .local addresses), and announces this one.",
        &["avahi-daemon.socket", "avahi-daemon.service"]));
    t.push(service("svc.docker", "Docker engine",
        "Runs containers. It holds memory and adds to boot time even with no containers running.",
        &["docker.socket", "docker.service"]));
    t.push(service("svc.libvirt", "Virtual machine manager (libvirt)",
        "Runs and networks virtual machines for apps like virt-manager and GNOME Boxes.",
        &["libvirtd.socket", "libvirtd.service"]));
    t.push(service("svc.tailscale", "Tailscale", "The Tailscale private network.", &["tailscaled.service"]));
    t.push(service("svc.ollama", "Ollama (local AI models)",
        "Serves local language models. It can hold several gigabytes of memory after its last use.",
        &["ollama.service"]));
    t.push(service("svc.postgresql", "PostgreSQL database", "A PostgreSQL server running in the background.", &["postgresql.service"]));
    t.push(service("svc.mariadb", "MariaDB / MySQL database", "A MariaDB server running in the background.", &["mariadb.service"]));
    t.push(service("svc.mysql", "MySQL database", "A MySQL server running in the background.", &["mysqld.service"]));
    t.push(service("svc.redis", "Redis", "A Redis in-memory data store running in the background.", &["redis.service"]));
    t.push(service("svc.valkey", "Valkey", "A Valkey in-memory data store running in the background.", &["valkey.service"]));
    t.push(service("svc.mongodb", "MongoDB database", "A MongoDB server running in the background.", &["mongodb.service"]));
    t.push(service("svc.nginx", "nginx web server", "An nginx web server running in the background.", &["nginx.service"]));
    t.push(service("svc.apache", "Apache web server", "An Apache web server running in the background.", &["httpd.service"]));
    t.push(service("svc.samba", "Windows file sharing (Samba)",
        "Shares folders with Windows and other machines on the network.", &["smb.service", "nmb.service"]));
    t.push(service("svc.cron", "Scheduled jobs (cron)",
        "Runs commands on a schedule from crontab files. Many systems use systemd timers instead and don't need it.",
        &["cronie.service"]));
    t.push(service("svc.modem", "Mobile broadband modems",
        "Manages SIM-card modems. Does nothing if the machine has none.", &["ModemManager.service"]));
    t.push(service("svc.oomd", "Out-of-memory guard (systemd-oomd)",
        "Watches memory pressure and ends the worst-offending app group before the whole desktop freezes.",
        &["systemd-oomd.service"]));
    t.push(service("svc.earlyoom", "Out-of-memory guard (earlyoom)",
        "Ends the largest process when free memory and swap run very low, before the machine locks up.",
        &["earlyoom.service"]));
    t.push(service("svc.irqbalance", "Spread interrupts across cores",
        "Distributes hardware interrupts over CPU cores. Helps busy servers; on a laptop it mostly prevents cores from sleeping.",
        &["irqbalance.service"]));
    t.push(service("svc.ananicy", "Automatic process priorities (ananicy)",
        "Adjusts the priority of known programs so that, for example, compilers yield to your browser.",
        &["ananicy-cpp.service"]));
    t.push(service("svc.thermald", "Intel thermal daemon", "Keeps Intel laptops cool by throttling before the firmware has to.", &["thermald.service"]));
    t.push(service("svc.tlp", "TLP power management",
        "A power-saving tool that sets many of the same switches found here; with it running, changes you make can be overwritten.",
        &["tlp.service"]));
    t.push(service("svc.auto_cpufreq", "auto-cpufreq",
        "Adjusts CPU speed by itself; with it running, CPU settings made here can be overwritten.",
        &["auto-cpufreq.service"]));
    t.push(service("svc.fstrim", "Weekly SSD trim",
        "Tells SSDs once a week which blocks are free, which keeps them fast and long-lived. Should be on if you have an SSD.",
        &["fstrim.timer"]));
    t.push(service("svc.paccache", "Weekly package-cache cleanup",
        "Deletes all but the three newest downloaded versions of each package, once a week.", &["paccache.timer"]));
    t.push(service("svc.reflector", "Refresh download mirrors",
        "Periodically picks the fastest package mirrors.", &["reflector.timer"]));
    t.push(service("svc.locate", "File-name search index",
        "Rebuilds the index behind the `locate` command every day. It reads the whole disk when it runs — a visible burst of disk activity soon after boot.",
        &["plocate-updatedb.timer"]));
    t.push(service("svc.mandb", "Manual-page index",
        "Rebuilds the index used by `man -k` and `apropos` every day. Known for a slow burst of work after boot.",
        &["man-db.timer"]));
    t.push(service("svc.keyring_sync", "Refresh package-signing keys",
        "Updates the keys used to verify packages, weekly. Slow on a poor connection but worth keeping.",
        &["archlinux-keyring-wkd-sync.timer"]));
    t.push(service("svc.pkgfile", "Package file-list updates",
        "Refreshes the database behind \"which package provides this command\".", &["pkgfile-update.timer"]));
    t.push(service("svc.snapper_timeline", "Hourly filesystem snapshots",
        "Takes a snapshot of the system every hour so changes can be rolled back.", &["snapper-timeline.timer"]));
    t.push(Tweak::new("svc.wait_online", SERVICES, "Wait for the network during boot",
        "Holds boot until the network is up. Only needed if something mounts network drives at start; otherwise it just adds seconds to boot.",
        Src::Unit { units: &["NetworkManager-wait-online.service"], user: false, now: false }, Kind::Toggle).effect("at the next boot"));
    t.push(Tweak::new("svc.syncthing", SERVICES, "Syncthing",
        "Keeps folders in sync with your other devices.",
        Src::Unit { units: &["syncthing.service"], user: true, now: true }, Kind::Toggle));
    t.push(Tweak::new("svc.gamemode", SERVICES, "GameMode daemon",
        "Lets games ask for performance settings while they run.",
        Src::Unit { units: &["gamemoded.service"], user: true, now: true }, Kind::Toggle));

    // ---------------------------------------------------------------- Developer
    t.push(sysctl("fs.inotify.max_user_watches", DEVELOPER, "File-watch limit",
        "How many files your programs may watch for changes. Editors, bundlers and sync tools on big projects hit the default — the \"ENOSPC: system limit for number of file watchers reached\" error.",
        choice(&[("8192", "8,192 (old default)"), ("65536", "65,536"), ("524288", "524,288"), ("1048576", "1,048,576")])));
    t.push(sysctl("fs.inotify.max_user_instances", DEVELOPER, "File-watcher instances",
        "How many separate watchers your programs may open. Running many dev servers or containers at once can exhaust the default.",
        choice(&[("128", "128 (default)"), ("1024", "1,024"), ("8192", "8,192")])));
    t.push(sysctl("net.ipv4.ip_unprivileged_port_start", DEVELOPER, "Lowest port a normal program may open",
        "Ports below this need administrator rights. Lowering it lets a dev server listen on 80 or 443 without sudo.",
        choice(&[("1024", "1024 (default)"), ("80", "80 — allow web ports"), ("0", "0 — allow every port")])));
    t.push(sysctl("kernel.perf_event_paranoid", DEVELOPER, "Performance profiling access",
        "How much the `perf` profiler and similar tools may measure without sudo.",
        choice(&[("-1", "Everything"), ("0", "Kernel and CPU events"), ("1", "Kernel and user profiling"), ("2", "Your own programs only (default)")])));
    t.push(sysctl("kernel.task_delayacct", DEVELOPER, "Per-task delay accounting",
        "Tracks how long each process waits for disk, CPU and memory. Tools such as iotop need it; it costs a little overhead.",
        Kind::Toggle));

    // ---------------------------------------------------------------- System
    t.push(custom("time.sync", SYSTEM, "Set the clock from the internet",
        "Keeps the system time correct automatically.", Custom::TimeSync, Kind::Toggle).persist(Persist::Inherent));
    t.push(custom("time.local_rtc", SYSTEM, "Hardware clock in local time",
        "Linux keeps the hardware clock in UTC and Windows keeps it in local time, so a dual-boot machine shows the wrong hour in one of them. On makes Linux follow Windows.",
        Custom::LocalRtc, Kind::Toggle).persist(Persist::Inherent).careful());
    t.push(dropin("systemd.stop_timeout", SYSTEM, "How long shutdown waits for a stuck service",
        "When a service won't stop, the system waits this long before killing it — the \"A stop job is running…\" countdown.",
        DropIn { file: "/etc/systemd/system.conf.d/99-trace.conf", section: "Manager", key: "DefaultTimeoutStopSec",
                 reader: Reader::CatConfig("systemd/system.conf"), reload: "" },
        &[("", "Default (90 seconds)"), ("10s", "10 seconds"), ("30s", "30 seconds"), ("90s", "90 seconds")]).effect("after a reboot"));

    // ================================================================ second batch
    // ---- Power & CPU
    t.push(sysfs("gpu.amd_performance_level", POWER, "AMD graphics power level",
        "How the AMD graphics chip manages its own speed. Automatic is right almost always; low pins it to its slowest state for maximum battery, high to its fastest.",
        &["/sys/class/drm/card*/device/power_dpm_force_performance_level"],
        choice(&[("auto", "Automatic"), ("low", "Low — slowest, coolest"), ("high", "High — fastest")])));
    t.push(sysfs("gpu.amd_panel_power", POWER, "Adaptive backlight (AMD)",
        "Dims the backlight and brightens the picture to match, saving a good share of screen power on battery. Higher levels save more and distort colours more; 0 is off.",
        &["/sys/class/drm/card*-eDP-*/amdgpu/panel_power_savings"], num(0.0, 4.0, 1.0, "")));
    t.push(sysfs("cpu.amd_dynamic_epp", POWER, "Switch energy preference with the charger",
        "Lets the AMD driver itself move between performance on mains and power saving on battery, instead of holding whatever was last set.",
        &["/sys/devices/system/cpu/amd_pstate/dynamic_epp"], Kind::Toggle).onoff("enabled", "disabled").persist(Persist::Never));
    t.push(sysfs("cpu.intel_dynamic_boost", POWER, "Boost when programs wait on disk (Intel)",
        "Briefly raises CPU speed when a task has just been waiting for I/O, which makes disk-heavy work feel snappier at a small power cost.",
        &["/sys/devices/system/cpu/intel_pstate/hwp_dynamic_boost"], Kind::Toggle));
    t.push(sysfs("cpu.intel_max_perf", POWER, "Cap CPU performance (Intel)",
        "The highest performance level the processor may use, as a percentage. Lower it to cut heat and fan noise.",
        &["/sys/devices/system/cpu/intel_pstate/max_perf_pct"], num(20.0, 100.0, 5.0, "%")));

    // ---- Laptop & Battery
    t.push(sysfs("laptop.conservation_mode", LAPTOP, "Battery conservation mode (Lenovo)",
        "Stops charging at about 60% to protect a battery that lives on the charger.",
        &["/sys/bus/platform/drivers/ideapad_acpi/VPC*/conservation_mode"], Kind::Toggle));
    t.push(sysfs("laptop.fn_lock", LAPTOP, "Fn lock (Lenovo)",
        "Swaps the top row between F1–F12 and the media keys printed on them.",
        &["/sys/bus/platform/drivers/ideapad_acpi/VPC*/fn_lock"], Kind::Toggle));
    t.push(sysfs("laptop.usb_charging", LAPTOP, "Charge USB devices while the laptop is off (Lenovo)",
        "Keeps the always-on USB port powered when the laptop sleeps or is shut down.",
        &["/sys/bus/platform/drivers/ideapad_acpi/VPC*/usb_charging"], Kind::Toggle));
    t.push(dropin("logind.kill_user_processes", LAPTOP, "When you log out, your running programs",
        "Whether programs you left running — a terminal multiplexer, a download, a build — survive logging out.",
        DropIn { file: LOGIND_FILE, section: "Login", key: "KillUserProcesses",
                 reader: Reader::Logind("KillUserProcesses"), reload: "systemctl reload systemd-logind" },
        &[("no", "Keep running"), ("yes", "Are ended")]).careful());

    // ---- Network (only while systemd-resolved is the resolver)
    let resolved = |id: &'static str, title: &'static str, help: &'static str, key: &'static str, options: &[(&'static str, &'static str)]| {
        dropin(id, NETWORK, title, help,
            DropIn { file: "/etc/systemd/resolved.conf.d/99-trace.conf", section: "Resolve", key,
                     reader: Reader::CatConfig("systemd/resolved.conf"), reload: "systemctl restart systemd-resolved" },
            options).needs_active("systemd-resolved.service")
    };
    t.push(resolved("dns.over_tls", "Encrypt DNS lookups (DNS over TLS)",
        "Hides which sites you look up from the networks in between. Opportunistic uses it when the server supports it; strict refuses to look anything up otherwise.",
        "DNSOverTLS", &[("", "Default"), ("no", "Off"), ("opportunistic", "When available"), ("yes", "Always (strict)")]));
    t.push(resolved("dns.dnssec", "Verify DNS answers (DNSSEC)",
        "Checks that answers are signed by the domain's owner, so they can't be forged on the way. Strict mode breaks on networks whose DNS server strips the signatures.",
        "DNSSEC", &[("", "Default"), ("no", "Off"), ("allow-downgrade", "When the server supports it"), ("yes", "Always (strict)")]).careful());
    t.push(resolved("dns.llmnr", "Windows-style local name lookup (LLMNR)",
        "Finds machines on the local network by bare name. A well-known way to trick a machine into talking to an impostor on shared networks.",
        "LLMNR", &[("", "Default"), ("yes", "Answer and look up"), ("resolve", "Look up only"), ("no", "Off")]));
    t.push(resolved("dns.mdns", "Local .local name lookup (mDNS)",
        "Finds and announces machines by their .local names. Turn off here if Avahi already does it, or to stop announcing this machine.",
        "MulticastDNS", &[("", "Default"), ("yes", "Answer and look up"), ("resolve", "Look up only"), ("no", "Off")]));

    // ---- Desktop & Windows
    t.push(gtk("gtk.font_antialiasing", DESKTOP, "Text smoothing",
        "How text edges are smoothed. Subpixel is sharpest on ordinary LCDs; greyscale suits OLED, rotated or very high-density screens.",
        "org.gnome.desktop.interface", "font-antialiasing",
        choice(&[("rgba", "Subpixel (sharpest on LCD)"), ("grayscale", "Greyscale"), ("none", "None")])));
    t.push(gtk("gtk.font_hinting", DESKTOP, "Text hinting",
        "How strongly letter shapes are snapped to the pixel grid. More is crisper and less faithful to the font's design.",
        "org.gnome.desktop.interface", "font-hinting",
        choice(&[("none", "None"), ("slight", "Slight"), ("medium", "Medium"), ("full", "Full")])));
    t.push(gtk("gtk.overlay_scrolling", DESKTOP, "Thin scrollbars that appear on hover",
        "Scrollbars stay hidden until you scroll or point at them. Off keeps a full-width bar always visible.",
        "org.gnome.desktop.interface", "overlay-scrolling", Kind::Toggle));
    t.push(gtk("gtk.cursor_blink", DESKTOP, "Blinking text cursor",
        "The text cursor blinks in apps. Off keeps it steady.", "org.gnome.desktop.interface", "cursor-blink", Kind::Toggle));
    t.push(gtk("gtk.files_show_hidden", DESKTOP, "Show hidden files in file pickers",
        "Open and Save dialogs list files whose names start with a dot.",
        "org.gtk.gtk4.Settings.FileChooser", "show-hidden", Kind::Toggle));
    t.push(gtk("gtk.files_folders_first", DESKTOP, "Folders first in file pickers",
        "Open and Save dialogs list folders before files.",
        "org.gtk.gtk4.Settings.FileChooser", "sort-directories-first", Kind::Toggle));
    t.push(gtk("gtk3.files_show_hidden", DESKTOP, "Show hidden files in older apps' file pickers",
        "The same, for apps built on the previous toolkit version (GTK 3).",
        "org.gtk.Settings.FileChooser", "show-hidden", Kind::Toggle));
    t.push(gtk("gtk3.files_folders_first", DESKTOP, "Folders first in older apps' file pickers",
        "The same, for apps built on the previous toolkit version (GTK 3).",
        "org.gtk.Settings.FileChooser", "sort-directories-first", Kind::Toggle));
    t.push(hypr("hypr.blur_popups", DESKTOP, "Blur behind menus and pop-ups",
        "Also blurs behind right-click menus and tooltips. Looks consistent; costs a little more.",
        "decoration:blur:popups", Kind::Toggle));
    t.push(hypr("hypr.blur_special", DESKTOP, "Blur behind the scratchpad workspace",
        "Blurs everything underneath when the special (scratchpad) workspace is shown. Expensive.",
        "decoration:blur:special", Kind::Toggle));
    t.push(hypr("hypr.blur_noise", DESKTOP, "Blur grain",
        "A little noise over blurred areas, which hides colour banding. 0 is perfectly smooth.",
        "decoration:blur:noise", num(0.0, 0.3, 0.01, "")));
    t.push(hypr("hypr.blur_vibrancy", DESKTOP, "Blur colour boost",
        "How much colours behind blurred surfaces are saturated.", "decoration:blur:vibrancy", num(0.0, 1.0, 0.05, "")));
    t.push(hypr("hypr.shadow_range", DESKTOP, "Shadow size",
        "How far window shadows spread, in pixels.", "decoration:shadow:range", num(1.0, 60.0, 1.0, "px")));
    t.push(hypr("hypr.snap", DESKTOP, "Floating windows snap together",
        "Floating windows stick to each other's edges and to the screen edge as you drag them.",
        "general:snap:enabled", Kind::Toggle));
    t.push(hypr("hypr.border_grab_area", DESKTOP, "Width of the resize edge",
        "How far outside a window's border you can still grab it to resize.",
        "general:extend_border_grab_area", num(0.0, 40.0, 1.0, "px")));
    t.push(hypr("hypr.force_split", DESKTOP, "Where a new tiled window goes",
        "Whether a new window opens on the side the pointer is on, or always on the same side.",
        "dwindle:force_split", choice(&[("0", "Follow the pointer"), ("1", "Always left / top"), ("2", "Always right / bottom")])));
    t.push(hypr("hypr.smart_split", DESKTOP, "Split towards the pointer's corner",
        "The split direction is chosen by which quarter of the window the pointer is in, so you can aim where a new window lands.",
        "dwindle:smart_split", Kind::Toggle));
    t.push(hypr("hypr.unfocused_fps", DESKTOP, "Frame rate of hidden windows",
        "How often windows on other workspaces are allowed to redraw. Low saves power; raise it if a video or game stutters right after you switch back.",
        "misc:render_unfocused_fps", num(1.0, 60.0, 1.0, "fps")));
    t.push(hypr("hypr.focus_under_fullscreen", DESKTOP, "A window opening under a full-screen one",
        "What happens when something asks for focus while another window is full-screen.",
        "misc:on_focus_under_fullscreen", choice(&[("0", "Stay on the full-screen window"), ("1", "Show the new window, keep full-screen"), ("2", "Leave full-screen")])));
    t.push(hypr("hypr.close_special_on_empty", DESKTOP, "Close the scratchpad when it empties",
        "The special workspace hides itself once its last window is gone.", "misc:close_special_on_empty", Kind::Toggle));
    t.push(hypr("hypr.workspace_cycles", DESKTOP, "Allow workspace back-and-forth loops",
        "Switching to the previous workspace repeatedly alternates between two, instead of walking further back through history.",
        "binds:allow_workspace_cycles", Kind::Toggle));

    // ---- Mouse, Keyboard & Touchpad
    t.push(hypr("input.caps_lock", INPUT, "Caps Lock key",
        "What the Caps Lock key does. Replaces any other keyboard options set in your config for this session.",
        "input:kb_options",
        choice(&[("", "Caps Lock (normal)"), ("caps:escape", "Escape"), ("caps:swapescape", "Swap with Escape"), ("ctrl:nocaps", "Control"), ("caps:super", "Super"), ("caps:none", "Nothing")])));
    t.push(hypr("input.mouse_refocus", INPUT, "Refocus when the pointer is moved by a window change",
        "When a window opens or closes under a still pointer, focus moves to whatever is now beneath it.",
        "input:mouse_refocus", Kind::Toggle));
    t.push(hypr("input.mouse_focuses_monitor", INPUT, "Moving to another screen focuses it",
        "Carrying the pointer onto another monitor makes that monitor the active one.",
        "misc:mouse_move_focuses_monitor", Kind::Toggle));
    t.push(hypr("input.no_warps", INPUT, "Never move the pointer for me",
        "Stops the pointer from jumping to a window when focus changes by keyboard.", "cursor:no_warps", Kind::Toggle));
    t.push(hypr("input.persistent_warps", INPUT, "Remember pointer position per window",
        "Returning to a window by keyboard puts the pointer back where it last was inside it.",
        "cursor:persistent_warps", Kind::Toggle));
    t.push(sysfs("input.mouse_polling", INPUT, "USB mouse polling rate",
        "How often USB mice are asked for their position. 1000 Hz is smoothest for games and uses a little more CPU. Unplug and re-plug the mouse for it to take effect.",
        &["/sys/module/usbhid/parameters/mousepoll"],
        choice(&[("0", "Mouse's own default"), ("8", "125 Hz"), ("4", "250 Hz"), ("2", "500 Hz"), ("1", "1000 Hz")]))
        .effect("after re-plugging the mouse"));

    // ---- Background Services
    t.push(service("svc.fwupd_refresh", "Check for firmware updates",
        "Periodically downloads the list of firmware updates for your hardware. Nothing is installed without you.",
        &["fwupd-refresh.timer"]));
    t.push(service("svc.smartd", "Drive health monitoring",
        "Watches your drives' self-test data in the background and logs a warning when one starts to fail.",
        &["smartd.service"]));
    t.push(service("svc.nfs", "NFS file server", "Shares folders with other Linux and Unix machines.", &["nfs-server.service"]));
    t.push(service("svc.tor", "Tor", "The Tor anonymity network client.", &["tor.service"]));
    t.push(service("svc.jellyfin", "Jellyfin media server", "Streams your media library to other devices.", &["jellyfin.service"]));
    t.push(service("svc.plex", "Plex media server", "Streams your media library to other devices.", &["plexmediaserver.service"]));
    t.push(service("svc.teamviewer", "TeamViewer daemon",
        "Lets TeamViewer accept remote-control connections. It runs all the time once installed, whether or not you use it.",
        &["teamviewerd.service"]));
    t.push(service("svc.anydesk", "AnyDesk daemon",
        "Lets AnyDesk accept remote-control connections. It runs all the time once installed, whether or not you use it.",
        &["anydesk.service"]));
    t.push(service("svc.warp", "Cloudflare WARP", "The Cloudflare WARP VPN daemon.", &["warp-svc.service"]));
    t.push(service("svc.zerotier", "ZeroTier", "The ZeroTier private network daemon.", &["zerotier-one.service"]));
    t.push(service("svc.nordvpn", "NordVPN daemon", "The background service behind the NordVPN app.", &["nordvpnd.service"]));
    t.push(service("svc.mullvad", "Mullvad VPN daemon", "The background service behind the Mullvad app.", &["mullvad-daemon.service"]));
    t.push(service("svc.protonvpn", "Proton VPN daemon", "The background service behind the Proton VPN app.", &["proton.VPN.service"]));
    t.push(service("svc.asusd", "ASUS laptop control (asusd)", "The asusctl daemon: fan curves, keyboard lighting and charge limits on ASUS laptops.", &["asusd.service"]));
    t.push(service("svc.supergfx", "ASUS graphics switching (supergfxd)", "Switches between integrated and dedicated graphics on ASUS laptops.", &["supergfxd.service"]));

    // ---- Developer
    t.push(sysctl("net.core.somaxconn", DEVELOPER, "Pending-connection queue length",
        "How many incoming connections may wait for a server program to accept them. Load tests against a local server hit the old default.",
        choice(&[("128", "128 (old default)"), ("4096", "4,096"), ("65535", "65,535")])));
    t.push(dropin("systemd.open_files", DEVELOPER, "Open-file limit for programs",
        "How many files and sockets one program may have open — the limit behind \"Too many open files\". Raising it costs nothing.",
        DropIn { file: "/etc/systemd/system.conf.d/99-trace.conf", section: "Manager", key: "DefaultLimitNOFILE",
                 reader: Reader::CatConfig("systemd/system.conf"), reload: "" },
        &[("", "Default (1,024, raisable to 524,288)"), ("65536:524288", "65,536"), ("1048576:1048576", "1,048,576")]).effect("after a reboot"));

    // ---- System
    t.push(sysctl("kernel.panic", SYSTEM, "After a kernel crash",
        "Whether the machine restarts by itself after a kernel panic or stays frozen so the message can be read.",
        choice(&[("0", "Stay frozen"), ("10", "Restart after 10 seconds"), ("60", "Restart after a minute")])));

    t
}
