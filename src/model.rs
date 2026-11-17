//! 配置、设备状态模型与后台轮询线程。

use crate::hid::{self, Candidate, ExtraDevice};
use chrono::{DateTime, Local};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[allow(dead_code)]
pub enum DeviceKind {
    #[default]
    Mouse,
    Keyboard,
    Headset,
    Other,
}

#[derive(Clone, Debug)]
pub struct DeviceState {
    pub key: String,
    pub name: String,
    pub model: String,
    pub kind: DeviceKind,
    pub battery: Option<u8>,
    pub charging: bool,
    pub connected: bool,
    pub last_seen: Option<DateTime<Local>>,
    /// 鼠标悬停时可见的协议 / 接口详情。
    pub detail: String,
}

impl DeviceState {
    pub fn status_text(&self) -> String {
        if !self.connected {
            if self.last_seen.is_some() {
                "上次读数".to_string()
            } else {
                "未连接".to_string()
            }
        } else if self.charging {
            "充电中".to_string()
        } else {
            "已连接".to_string()
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub devices: Vec<DeviceState>,
    pub updated_at: Option<DateTime<Local>>,
    /// 最近一次扫描的诊断摘要（设置里可见）。
    pub note: String,
}

impl Snapshot {
    /// 托盘图标优先显示「未充电且电量最低」的设备。
    pub fn tray_percent(&self) -> Option<u8> {
        let mut live: Vec<(u8, bool)> = self
            .devices
            .iter()
            .filter(|d| d.connected)
            .filter_map(|d| d.battery.map(|b| (b, d.charging)))
            .collect();
        live.sort_by_key(|(b, charging)| (*charging, *b));
        live.first().map(|(b, _)| *b)
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(default)]
pub struct Config {
    /// 自动刷新间隔（秒），最小 5 秒。
    pub refresh_secs: u64,
    /// 窗口失焦时自动隐藏到托盘。
    pub auto_hide: bool,
    /// 启动时显示一次面板。
    pub show_on_start: bool,
    /// 演示模式：不访问硬件，用示例数据预览界面。
    pub demo: bool,
    /// 外观：auto（跟随系统）/ light / dark。
    pub theme: String,
    /// 额外的 VID/PID；pid 填 0 表示该 VID 下全部设备。
    #[serde(default)]
    pub extra_devices: Vec<ExtraDevice>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            refresh_secs: 30,
            auto_hide: true,
            show_on_start: true,
            demo: false,
            theme: "auto".to_string(),
            extra_devices: Vec::new(),
        }
    }
}

impl Config {
    pub fn load() -> Self {
        let path = Self::path();
        match std::fs::read_to_string(&path) {
            Ok(text) => match toml::from_str::<Self>(&text) {
                Ok(cfg) => cfg,
                Err(e) => {
                    crate::log_line(&format!("配置文件解析失败（使用默认值）：{e}"));
                    Self::default()
                }
            },
            Err(_) => Self::default(),
        }
    }

    pub fn path() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("config.toml")))
            .unwrap_or_else(|| PathBuf::from("config.toml"))
    }

    pub fn save(&self) -> Result<(), String> {
        let mut text = format!(
            "# 电量岛配置\n\n# 自动刷新间隔（秒，最小 5）\nrefresh_secs = {}\n# 窗口失焦时自动隐藏到托盘\nauto_hide = {}\n# 启动时显示一次面板\nshow_on_start = {}\n# 演示模式：不读取硬件，用示例数据预览界面\ndemo = {}\n# 外观：auto（跟随系统）/ light / dark\ntheme = \"{}\"\n",
            self.refresh_secs, self.auto_hide, self.show_on_start, self.demo, self.theme
        );
        for e in &self.extra_devices {
            text.push_str(&format!(
                "\n[[extra_devices]]\nvid = 0x{:04X}\npid = 0x{:04X}\n",
                e.vid, e.pid
            ));
            if let Some(p) = e.usage_page {
                text.push_str(&format!("usage_page = 0x{p:04X}\n"));
            }
            if let Some(n) = &e.name {
                text.push_str(&format!("name = \"{n}\"\n"));
            }
            if let Some(m) = &e.model {
                text.push_str(&format!("model = \"{m}\"\n"));
            }
        }
        std::fs::write(Self::path(), text).map_err(|e| e.to_string())
    }
}

pub enum WorkerCmd {
    Refresh,
    SetInterval(u64),
    SetDemo(bool),
    Quit,
}

pub struct Worker {
    pub snapshot_rx: Receiver<Snapshot>,
    pub cmd_tx: Sender<WorkerCmd>,
    _handle: Option<JoinHandle<()>>,
}

fn display_for(c: &Candidate) -> (String, String, DeviceKind) {
    if let Some(name) = &c.display_name {
        return (
            name.clone(),
            c.display_model
                .clone()
                .unwrap_or_else(|| format!("{:04X}:{:04X} · 2.4G", c.vid, c.pid)),
            DeviceKind::Mouse,
        );
    }
    let upper = c.product.to_uppercase();
    if upper.contains("L7") || upper.contains("MCHOSE") || upper.contains("MC_") {
        return (
            "迈从 L7 Pro".to_string(),
            "L7 Pro · 2.4G".to_string(),
            DeviceKind::Mouse,
        );
    }
    // 其它设备（含 config 里自定义的）优先显示真实产品名，而不是统一叫「迈从 2.4G 设备」。
    let kind = if upper.contains("KEYBOARD") || upper.contains("KBD") {
        DeviceKind::Keyboard
    } else if upper.contains("HEADSET") || upper.contains("HEADPHONE") || upper.contains("EARPHONE") {
        DeviceKind::Headset
    } else {
        DeviceKind::Mouse
    };
    let product = c.product.trim();
    (
        if product.is_empty() {
            "2.4G 设备".to_string()
        } else {
            product.to_string()
        },
        format!("{:04X}:{:04X} · 2.4G", c.vid, c.pid),
        kind,
    )
}

fn poll_devices(cfg: &Config) -> Snapshot {
    let api = match hidapi::HidApi::new() {
        Ok(a) => a,
        Err(e) => {
            return Snapshot {
                note: format!("hidapi 初始化失败：{e}"),
                ..placeholder_snapshot("hidapi 初始化失败")
            }
        }
    };
    let candidates = hid::find_candidates(&api, &cfg.extra_devices);
    let mut devices = Vec::new();
    let mut notes = Vec::new();
    let mut seen = std::collections::HashSet::new();
    // 同一个接收器会枚举出多个 HID 集合（实机 L7 Pro 有 5 个），
    // 只有 UsagePage 0xFF01 的配置集合能读电量。某组读成功后跳过其余集合，
    // 避免每次刷新都对着 4 个不支持 Feature Report 的接口白跑一趟。
    let mut done_groups = std::collections::HashSet::new();

    for c in &candidates {
        if done_groups.contains(&(c.vid, c.pid)) {
            continue;
        }
        match hid::read_battery(&api, c) {
            Ok(r) => {
                done_groups.insert((c.vid, c.pid));
                let dedup = format!("{:04x}:{:04x}:{:04x}", c.vid, c.pid, r.pid);
                if !seen.insert(dedup) {
                    continue;
                }
                let (name, model, kind) = display_for(c);
                notes.push(format!(
                    "OK  {} → {}%（充电={}）",
                    c.label(),
                    r.battery,
                    r.charging
                ));
                devices.push(DeviceState {
                    key: c.key(),
                    name,
                    model,
                    kind,
                    battery: Some(r.battery),
                    // 状态值约定：2=放电，4=满电，3=充电中。
                    charging: r.charging != 0 && r.charging != 2 && r.charging != 4,
                    connected: true,
                    last_seen: Some(Local::now()),
                    detail: format!(
                        "接口 {:04X}:{:04X} · 无线设备 {:04X}:{:04X}\n固件 u32={} · 标志 0x{:02X}\n报告 0x11/0x06 · 帧长 {}B · 载荷取反={}\n{}",
                        c.vid,
                        c.pid,
                        r.vid,
                        r.pid,
                        r.firmware,
                        r.flags,
                        r.frame_len,
                        r.inverted,
                        r.label
                    ),
                });
            }
            Err(e) => notes.push(format!("--  {} → {e}", c.label())),
        }
    }

    if devices.is_empty() {
        let reason = if candidates.is_empty() {
            "未发现 2.4G 接收器（VID 5253 / 3837，UsagePage FF01/FF0B）".to_string()
        } else {
            "接收器已发现，但设备未响应电量查询（可能已关机 / 休眠 / 被厂商驱动占用）"
                .to_string()
        };
        return Snapshot {
            note: format!("{reason}\n{}", notes.join("\n")),
            ..placeholder_snapshot(&reason)
        };
    }

    Snapshot {
        devices,
        updated_at: Some(Local::now()),
        note: format!("候选接口 {} 个\n{}", candidates.len(), notes.join("\n")),
    }
}

fn placeholder_snapshot(reason: &str) -> Snapshot {
    Snapshot {
        devices: vec![DeviceState {
            key: "mchose-l7-pro".to_string(),
            name: "迈从 L7 Pro".to_string(),
            model: "L7 Pro · 2.4G".to_string(),
            kind: DeviceKind::Mouse,
            battery: None,
            charging: false,
            connected: false,
            last_seen: None,
            detail: format!(
                "{reason}\n\n协议：Feature Report 0x11，命令 0x06，载荷逐字节取反\n\
                 VID 0x5253 / 0x3837，UsagePage 0xFF01 / 0xFF0B\n\
                 可运行 `--dump` 查看原始报告"
            ),
        }],
        // 没有成功读数时不要谎报「刚刚更新」。
        updated_at: None,
        ..Default::default()
    }
}

/// 界面预览数据，仅用于 `--demo` / 配置 `demo = true`。
fn demo_snapshot() -> Snapshot {
    let now = Some(Local::now());
    Snapshot {
        devices: vec![
            DeviceState {
                key: "demo-mouse".into(),
                name: "魔刃 X".into(),
                model: "CHAKRAM X · 2.4G".into(),
                kind: DeviceKind::Mouse,
                battery: Some(90),
                charging: false,
                connected: true,
                last_seen: now,
                detail: "演示数据（--demo）".into(),
            },
            DeviceState {
                key: "demo-keyboard".into(),
                name: "夜魔".into(),
                model: "AZOTH · OMNI".into(),
                kind: DeviceKind::Keyboard,
                battery: Some(100),
                charging: false,
                connected: true,
                last_seen: now,
                detail: "演示数据（--demo）".into(),
            },
            DeviceState {
                key: "demo-headset".into(),
                name: "赛睿耳机".into(),
                model: "Nova Pro Wireless · 2.4G".into(),
                kind: DeviceKind::Headset,
                battery: None,
                charging: false,
                connected: false,
                last_seen: None,
                detail: "演示数据（--demo）".into(),
            },
        ],
        updated_at: now,
        note: "演示模式：未访问硬件，全部为示例数据。".into(),
    }
}

impl Worker {
    /// `ctx` 用于窗口隐藏时唤醒 eframe 的 `App::logic`，否则定时刷新结果没人消费。
    pub fn spawn(cfg: Config, ctx: egui::Context) -> Self {
        let (snapshot_tx, snapshot_rx) = mpsc::channel();
        let (cmd_tx, cmd_rx) = mpsc::channel::<WorkerCmd>();
        let mut interval = Duration::from_secs(cfg.refresh_secs.max(5));
        let mut demo = cfg.demo;

        // 保留最后一次成功读数：设备关机 / 休眠时卡片不消失。
        let mut last_good: Vec<DeviceState> = Vec::new();
        let mut last_good_at: Option<DateTime<Local>> = None;

        let handle = thread::spawn(move || loop {
            let mut snap = if demo {
                demo_snapshot()
            } else {
                poll_devices(&cfg)
            };
            if !demo {
                if snap.devices.iter().any(|d| d.connected) {
                    last_good = snap
                        .devices
                        .iter()
                        .filter(|d| d.connected)
                        .cloned()
                        .collect();
                    last_good_at = snap.updated_at;
                } else if !last_good.is_empty() {
                    snap.devices = last_good
                        .iter()
                        .map(|d| DeviceState {
                            connected: false,
                            ..d.clone()
                        })
                        .collect();
                    // 沿用上次成功读数的时间，底部栏才不会把「没读到」显示成「刚刚更新」。
                    snap.updated_at = last_good_at;
                    snap.note.push_str("\n设备未响应，保留上次读数（可能已休眠或关机）。");
                }
            }
            if snapshot_tx.send(snap).is_err() {
                break;
            }
            ctx.request_repaint();
            match cmd_rx.recv_timeout(interval) {
                Ok(WorkerCmd::Refresh) => continue,
                Ok(WorkerCmd::SetInterval(s)) => {
                    interval = Duration::from_secs(s.max(5));
                    continue;
                }
                Ok(WorkerCmd::SetDemo(v)) => {
                    demo = v;
                    continue;
                }
                Ok(WorkerCmd::Quit) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
            }
        });

        Self {
            snapshot_rx,
            cmd_tx,
            _handle: Some(handle),
        }
    }
}
