// Windows 下不弹出控制台黑窗口（日志写入 %LOCALAPPDATA%\BatteryIsland\app.log）。
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

//! 电量岛 —— 2.4G 无线外设电量小工具
//!
//! Rust + egui，Windows 托盘常驻。默认读取迈从 MCHOSE L7 Pro 2.4G 接收器
//! 的 Feature Report（详见 src/hid.rs 顶部的协议来源说明）。
//!
//! 命令行：
//!   battery-island.exe              正常运行（托盘常驻）
//!   battery-island.exe --demo       界面预览，不访问硬件
//!   battery-island.exe --read       实机读取一次电量并写入报告文件（不弹窗口）
//!   battery-island.exe --dump       导出 HID 接口原始报告诊断
//!   battery-island.exe --help       帮助

mod app;
mod hid;
mod icons;
mod model;

use app::{window_height, BatteryApp, UiEvent, WINDOW_W};
use model::{Config, Worker};
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};

/// 统一圆角半径构造，隔离 egui 版本间的 Rounding/CornerRadius 差异。
pub fn radius(v: u8) -> egui::CornerRadius {
    egui::CornerRadius::same(v)
}

static LOG_FILE: OnceLock<Mutex<Option<std::fs::File>>> = OnceLock::new();

pub fn log_line(msg: &str) {
    let mutex = LOG_FILE.get_or_init(|| {
        let path = std::env::var("LOCALAPPDATA")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir())
            .join("BatteryIsland")
            .join("app.log");
        let file = std::fs::create_dir_all(path.parent().unwrap())
            .ok()
            .and_then(|_| {
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .ok()
            });
        Mutex::new(file)
    });
    if let Ok(mut guard) = mutex.lock() {
        if let Some(f) = guard.as_mut() {
            use std::io::Write;
            let _ = writeln!(f, "[{}] {msg}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"));
        }
    }
}

/// 加载中文字体。egui 自带字体不含 CJK，不加载会显示方块。
fn setup_fonts(ctx: &egui::Context) {
    use egui::{FontData, FontDefinitions, FontFamily};

    const REGULAR: &[&str] = &[
        "C:/Windows/Fonts/Deng.ttf",
        "C:/Windows/Fonts/msyh.ttc",
        "C:/Windows/Fonts/simhei.ttf",
        "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/System/Library/Fonts/PingFang.ttc",
    ];
    const BOLD: &[&str] = &[
        "C:/Windows/Fonts/Dengb.ttf",
        "C:/Windows/Fonts/msyhbd.ttc",
        "C:/Windows/Fonts/simhei.ttf",
    ];

    let load = |list: &[&str]| -> Option<Vec<u8>> {
        list.iter().find_map(|p| std::fs::read(p).ok())
    };

    let mut fonts = FontDefinitions::default();
    let regular = load(REGULAR);
    let bold = load(BOLD).or_else(|| regular.clone());

    if let Some(data) = regular {
        fonts
            .font_data
            .insert("CJK".to_owned(), FontData::from_owned(data).into());
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .push("CJK".to_owned());
        fonts
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .push("CJK".to_owned());
    } else {
        log_line("警告：未找到中文字体，界面中文可能显示为方块");
    }

    match bold {
        Some(data) => {
            fonts
                .font_data
                .insert("CJK-Bold".to_owned(), FontData::from_owned(data).into());
            fonts
                .families
                .insert(FontFamily::Name("Bold".into()), vec!["CJK-Bold".to_owned()]);
        }
        None => {
            // 连粗体（或任何中文字体）都没找到时，把 Bold 指回默认比例字体。
            // 否则 bold() 引用的 family 不存在，标题会渲染成空白。
            let fallback = fonts
                .families
                .get(&FontFamily::Proportional)
                .cloned()
                .unwrap_or_default();
            fonts
                .families
                .insert(FontFamily::Name("Bold".into()), fallback);
        }
    }
    ctx.set_fonts(fonts);
}

fn build_tray(ctx: &egui::Context, tx: mpsc::Sender<UiEvent>) -> Result<tray_icon::TrayIcon, String> {
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let menu = Menu::new();
    let show = MenuItem::new("显示 / 隐藏", true, None);
    let refresh = MenuItem::new("立即刷新", true, None);
    let quit = MenuItem::new("退出电量岛", true, None);
    menu.append(&show).map_err(|e| e.to_string())?;
    menu.append(&refresh).map_err(|e| e.to_string())?;
    menu.append(&PredefinedMenuItem::separator()).map_err(|e| e.to_string())?;
    menu.append(&quit).map_err(|e| e.to_string())?;

    let show_id = show.id().clone();
    let refresh_id = refresh.id().clone();
    let quit_id = quit.id().clone();
    let tx_click = tx.clone();
    let ctx_menu = ctx.clone();
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        let ev = if e.id == show_id {
            Some(UiEvent::Toggle)
        } else if e.id == refresh_id {
            Some(UiEvent::Refresh)
        } else if e.id == quit_id {
            Some(UiEvent::Quit)
        } else {
            None
        };
        if let Some(ev) = ev {
            let _ = tx.send(ev);
            ctx_menu.request_repaint();
        }
    }));

    let ctx_click = ctx.clone();
    TrayIconEvent::set_event_handler(Some(move |e| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            position,
            ..
        } = e
        {
            let _ = tx_click.send(UiEvent::TrayClick { x: position.x, y: position.y });
            ctx_click.request_repaint();
        }
    }));

    let icon = Icon::from_rgba(icons::tray_rgba(None, 64), 64, 64).map_err(|e| e.to_string())?;
    TrayIconBuilder::new()
        .with_icon(icon)
        .with_tooltip("电量岛 · 所有设备，一眼可见")
        .with_menu_on_left_click(false)
        .with_menu(Box::new(menu))
        .build()
        .map_err(|e| e.to_string())
}

/// 不启动 GUI，用与后台线程完全相同的读取路径做一次实机电量探测。
fn probe_battery(cfg: &Config) -> String {
    let mut out = String::new();
    let api = match hidapi::HidApi::new() {
        Ok(a) => a,
        Err(e) => return format!("hidapi 初始化失败：{e}\n"),
    };
    let list = hid::find_candidates(&api, &cfg.extra_devices);
    out.push_str(&format!("候选接口：{} 个\n", list.len()));
    for c in &list {
        out.push_str(&format!("\n接口 {}\n", c.label()));
        match hid::read_battery(&api, c) {
            Ok(r) => out.push_str(&format!(
                "  读取成功：电量 {}%  充电原始值 {}\n  vid={:04X} pid={:04X} 固件={} 标志=0x{:02X} 帧长={}B 载荷取反={}\n  响应帧：{}\n",
                r.battery, r.charging, r.vid, r.pid, r.firmware, r.flags, r.frame_len, r.inverted, r.label
            )),
            Err(e) => out.push_str(&format!("  读取失败：{e}\n")),
        }
    }
    out.push_str("\n===== 原始 Feature Report（单次，无重试）=====\n");
    out.push_str(&hid::dump_candidates(&cfg.extra_devices));
    out
}

/// 用 Win32 窗口区域把无边框窗口裁成圆角。
/// 透明窗口 + 自绘圆角在 Windows 上会被 DWM 的不可见边框裁掉底部两角，
/// 改由系统裁切窗口形状最可靠，也不依赖 Windows 11 的 DWM 圆角属性。
#[cfg(windows)]
pub fn apply_round_region(width: i32, height: i32) {
    use windows_sys::Win32::Graphics::Gdi::{CreateRoundRectRgn, SetWindowRgn};
    use windows_sys::Win32::UI::WindowsAndMessaging::FindWindowW;

    unsafe {
        let title: Vec<u16> = "电量岛\0".encode_utf16().collect();
        let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
        if hwnd.is_null() {
            return;
        }
        // 透明窗口的视口相对窗口矩形有 8px 不可见边框偏移，
        // 区域从 (8,8) 起才能和 egui 的绘制内容对齐。
        let rgn = CreateRoundRectRgn(8, 8, width, height, 24, 24);
        if !rgn.is_null() {
            // SetWindowRgn 成功后系统接管 HRGN，不要 DeleteObject。
            let ok = SetWindowRgn(hwnd, rgn, 1);
            log_line(&format!("窗口圆角区域：{width}x{height} ret={ok}"));
        }
    }
}

fn print_help() {
    let text = "电量岛 · 2.4G 无线外设电量小工具\n\n\
        --demo    界面预览（示例数据，不访问硬件）\n\
        --read    实机读取一次电量，结果写入 battery-island-reading.txt\n\
        --dump    导出 HID 候选接口与原始 Feature Report 到 battery-island-dump.txt\n\
        --help    显示本帮助\n\n\
        配置文件：程序同目录 config.toml\n\
        日志：%LOCALAPPDATA%\\BatteryIsland\\app.log\n";
    println!("{text}");
    let _ = std::fs::write(
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("README-CLI.txt")))
            .unwrap_or_else(|| "README-CLI.txt".into()),
        text,
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return;
    }

    let mut cfg = Config::load();
    if args.iter().any(|a| a == "--demo") {
        cfg.demo = true;
        cfg.show_on_start = true;
    }
    let want_read = args.iter().any(|a| a == "--read");
    let want_dump = args.iter().any(|a| a == "--dump");
    if want_read || want_dump {
        let body = if want_read {
            probe_battery(&cfg)
        } else {
            hid::dump_candidates(&cfg.extra_devices)
        };
        let report = format!(
            "电量岛 HID 诊断 · {}\n模式：{}\n\n{}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            if want_read { "--read（含重试读取）" } else { "--dump（单次原始报告）" },
            body
        );
        let name = if want_read {
            "battery-island-reading.txt"
        } else {
            "battery-island-dump.txt"
        };
        let path = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join(name)))
            .unwrap_or_else(|| name.into());
        let _ = std::fs::write(&path, &report);
        let _ = std::fs::write(
            std::env::temp_dir().join(name),
            &report,
        );
        println!("{report}");
        println!("\n已写入 {}", path.display());
        return;
    }

    log_line(&format!(
        "启动：demo={} refresh={}s auto_hide={}",
        cfg.demo, cfg.refresh_secs, cfg.auto_hide
    ));

    let (ui_tx, ui_rx) = mpsc::channel::<UiEvent>();
    // 真实模式默认只有 L7 Pro 一台；demo 模式展示三台。收到首次扫描结果后还会校准一次。
    let initial_h = window_height(if cfg.demo { 3 } else { 1 });

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([WINDOW_W, initial_h])
            .with_min_inner_size([WINDOW_W, 280.0])
            .with_resizable(false)
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top()
            // 托盘常驻工具不该占任务栏和 Alt-Tab 列表。
            .with_taskbar(false)
            .with_visible(cfg.show_on_start),
        ..Default::default()
    };

    let result = eframe::run_native(
        "电量岛",
        options,
        Box::new(move |cc| {
            setup_fonts(&cc.egui_ctx);
            // 面板是浅色的，egui 默认深色会让「设置」弹窗显得突兀。
            cc.egui_ctx.set_visuals(egui::Visuals::light());
            let worker = Worker::spawn(cfg.clone(), cc.egui_ctx.clone());
            let tray = match build_tray(&cc.egui_ctx, ui_tx) {
                Ok(t) => t,
                Err(e) => {
                    log_line(&format!("托盘创建失败：{e}"));
                    return Err(Box::new(std::io::Error::other(e)));
                }
            };
            Ok(Box::new(BatteryApp::new(cfg, worker, ui_rx, tray)))
        }),
    );

    if let Err(e) = result {
        log_line(&format!("运行失败：{e}"));
        eprintln!("电量岛启动失败：{e}");
    }
}
