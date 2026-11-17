//! 「电量岛」面板：无边框圆角窗口，全部内容由 egui 自绘。

use crate::{icons, model::*, radius};
use egui::{
    Align2, Color32, Context, FontId, Id, Painter, Pos2, Rect, Sense, Stroke, Vec2,
};
use std::sync::mpsc::Receiver;
use std::time::Duration;
use tray_icon::TrayIcon;

pub const WINDOW_W: f32 = 392.0;
/// 3 台设备时的面板高度。
pub const WINDOW_H: f32 = 600.0;

/// 面板最多容纳的卡片数，超出部分不再撑高窗口。
pub const MAX_CARDS: usize = 5;

/// 按设备数量动态决定面板高度。
/// 固定 600px 时只接一台鼠标会让下半屏全空，头重脚轻。
pub fn window_height(count: usize) -> f32 {
    match count {
        3 => WINDOW_H,
        n => 204.0 + 132.0 * n.clamp(1, MAX_CARDS) as f32,
    }
}

const SHELL_BG: Color32 = Color32::from_rgb(0xF3, 0xF6, 0xF8);
const CARD_BG: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
const TEXT: Color32 = Color32::from_rgb(0x1A, 0x1E, 0x24);
const MUTED: Color32 = Color32::from_rgb(0x8D, 0x94, 0x9E);
const ICON_FG: Color32 = Color32::from_rgb(0x54, 0x5C, 0x66);
const BADGE_BG: Color32 = Color32::from_rgb(0xF3, 0xF5, 0xF6);
const GREEN: Color32 = Color32::from_rgb(0x0C, 0x86, 0x69);
const TRACK: Color32 = Color32::from_rgb(0xE7, 0xEB, 0xEC);
const HOVER: Color32 = Color32::from_rgb(0xE7, 0xEA, 0xED);
const DIVIDER: Color32 = Color32::from_rgb(0xE4, 0xE8, 0xEB);

const CARD_H: f32 = 118.0;
const CARD_GAP: f32 = 14.0;

fn regular(size: f32) -> FontId {
    FontId::proportional(size)
}
fn bold(size: f32) -> FontId {
    FontId::new(size, egui::FontFamily::Name("Bold".into()))
}

fn bar_color(v: u8) -> Color32 {
    if v < 20 {
        Color32::from_rgb(0xDE, 0x4C, 0x40)
    } else if v < 40 {
        Color32::from_rgb(0xE9, 0x96, 0x2C)
    } else {
        GREEN
    }
}

/// 把时间差写成「刚刚 / N 秒前 / N 分钟前」，比固定时钟更直观。
fn human_ago(t: &chrono::DateTime<chrono::Local>) -> String {
    let secs = (chrono::Local::now() - *t).num_seconds().max(0);
    if secs < 15 {
        "刚刚".to_string()
    } else if secs < 60 {
        format!("{secs} 秒前")
    } else if secs < 3600 {
        format!("{} 分钟前", secs / 60)
    } else {
        t.format("%H:%M").to_string()
    }
}

/// 面板内的两个页面：设备列表 / 设置。
/// 设置做成内嵌页而不是 egui::Window：独立窗口会带默认标题栏和深色样式，
/// 还会挡住下面的卡片。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Devices,
    Settings,
}

const SETTINGS_H: f32 = 470.0;

/// 托盘 / 菜单事件，由全局事件处理器转发到 UI 线程。
#[derive(Debug)]
pub enum UiEvent {
    TrayClick { x: f64, y: f64 },
    Toggle,
    Refresh,
    Quit,
}

pub struct BatteryApp {
    pub cfg: Config,
    worker: Worker,
    ui_rx: Receiver<UiEvent>,
    tray: TrayIcon,
    snapshot: Snapshot,
    visible: bool,
    auto_hide: bool,
    was_focused: bool,
    page: Page,
    refresh_secs: u64,
    demo: bool,
    last_tray_value: Option<Option<u8>>,
    last_device_count: usize,
    current_h: f32,
    toast: String,
    toast_at: Option<std::time::Instant>,
    last_tray_click: Option<std::time::Instant>,
}

impl BatteryApp {
    pub fn new(cfg: Config, worker: Worker, ui_rx: Receiver<UiEvent>, tray: TrayIcon) -> Self {
        let visible = cfg.show_on_start;
        let auto_hide = cfg.auto_hide;
        let refresh_secs = cfg.refresh_secs.max(5);
        let demo = cfg.demo;
        Self {
            cfg,
            worker,
            ui_rx,
            tray,
            snapshot: Snapshot::default(),
            visible,
            auto_hide,
            was_focused: false,
            page: Page::Devices,
            refresh_secs,
            demo,
            last_tray_value: None,
            last_device_count: 0,
            current_h: window_height(if demo { 3 } else { 1 }),
            toast: String::new(),
            toast_at: None,
            last_tray_click: None,
        }
    }

    fn hide(&mut self, ctx: &Context) {
        self.visible = false;
        self.was_focused = false;
        // 收起后下次打开回到设备页，避免一开面板就停在设置里。
        self.page = Page::Devices;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
    }

    fn set_page(&mut self, ctx: &Context, page: Page) {
        self.page = page;
        let h = match page {
            Page::Settings => SETTINGS_H,
            Page::Devices => window_height(self.snapshot.devices.len()),
        };
        if (h - self.current_h).abs() > 0.5 {
            self.current_h = h;
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(WINDOW_W, h)));
            ctx.request_repaint();
        }
    }

    fn show(&mut self, ctx: &Context, at: Option<(f64, f64)>) {
        self.visible = true;
        // 重新显示时先不认为「曾经获得焦点」，否则会被失焦自动隐藏立刻收起。
        self.was_focused = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        if let Some((x, y)) = at {
            let ppp = ctx.pixels_per_point().max(1.0);
            let px = (x / ppp as f64) as f32;
            let py = (y / ppp as f64) as f32;
            // 托盘图标可能贴着屏幕右边/下边，必须夹住，否则面板会有一半跑到屏幕外。
            let (mw, mh) = ctx.input(|i| {
                let m = i.viewport().monitor_size.unwrap_or(egui::vec2(1920.0, 1080.0));
                (m.x, m.y)
            });
            let left = (px - WINDOW_W + 46.0).clamp(8.0, (mw - WINDOW_W - 8.0).max(8.0));
            let top = (py - self.current_h - 14.0).clamp(8.0, (mh - self.current_h - 48.0).max(8.0));
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(Pos2::new(left, top)));
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        ctx.request_repaint();
    }

    fn refresh(&self) {
        let _ = self.worker.cmd_tx.send(WorkerCmd::Refresh);
    }

    fn sync_tray(&mut self) {
        let value = self.snapshot.tray_percent();
        if self.last_tray_value == Some(value) {
            return;
        }
        self.last_tray_value = Some(value);
        // 64×64 在高 DPI 缩放下比 32×32 明显清晰，系统会按需缩小。
        let rgba = icons::tray_rgba(value, 64);
        if let Ok(icon) = tray_icon::Icon::from_rgba(rgba, 64, 64) {
            let _ = self.tray.set_icon(Some(icon));
        }
        let tip = match value {
            Some(v) => format!(
                "电量岛 · {} 台设备 · 最低 {}%",
                self.snapshot.devices.iter().filter(|d| d.connected).count(),
                v
            ),
            None => "电量岛 · 暂无在线设备".to_string(),
        };
        let _ = self.tray.set_tooltip(Some(tip));
    }

    fn apply_settings(&mut self) {
        self.cfg.auto_hide = self.auto_hide;
        self.cfg.refresh_secs = self.refresh_secs;
        self.cfg.demo = self.demo;
        self.auto_hide = self.cfg.auto_hide;
        let _ = self.worker.cmd_tx.send(WorkerCmd::SetInterval(self.refresh_secs));
        let _ = self.worker.cmd_tx.send(WorkerCmd::SetDemo(self.demo));
        match self.cfg.save() {
            Ok(()) => self.toast = "配置已保存".into(),
            Err(e) => self.toast = format!("保存失败：{e}"),
        }
        self.toast_at = Some(std::time::Instant::now());
        self.refresh();
    }

    fn icon_button(
        &self,
        ui: &mut egui::Ui,
        p: &Painter,
        center: Pos2,
        id: &str,
        draw: impl FnOnce(&Painter, Pos2, Color32),
    ) -> bool {
        let r = Rect::from_center_size(center, Vec2::splat(30.0));
        let resp = ui.interact(r, Id::unique(id), Sense::click());
        if resp.hovered() {
            p.rect_filled(r, radius(9), HOVER);
        }
        let color = if resp.hovered() { TEXT } else { ICON_FG };
        draw(p, center, color);
        resp.clicked()
    }

    fn paint_cards(&mut self, ui: &mut egui::Ui, p: &Painter, shell: Rect) {
        let card_w = shell.width() - 36.0;
        let mut y = shell.top() + 88.0;
        // 超出面板容量的设备不再绘制，避免卡片画到窗口外面。
        for d in self.snapshot.devices.iter().take(MAX_CARDS) {
            let card = Rect::from_min_size(Pos2::new(shell.left() + 18.0, y), Vec2::new(card_w, CARD_H));
            p.rect_filled(card, radius(16), CARD_BG);

            let badge = Rect::from_min_size(
                Pos2::new(card.left() + 16.0, card.top() + 16.0),
                Vec2::splat(42.0),
            );
            icons::device_badge(p, badge, d.kind, ICON_FG, BADGE_BG);

            p.text(
                Pos2::new(card.left() + 68.0, card.top() + 18.0),
                Align2::LEFT_TOP,
                &d.name,
                bold(15.0),
                TEXT,
            );
            p.text(
                Pos2::new(card.left() + 68.0, card.top() + 41.0),
                Align2::LEFT_TOP,
                &d.model,
                regular(12.0),
                MUTED,
            );

            let value = match d.battery {
                Some(v) if d.connected => format!("{v}%"),
                _ => "—".to_string(),
            };
            let value_color = if d.connected { TEXT } else { Color32::from_rgb(0x4A, 0x50, 0x58) };
            p.text(
                Pos2::new(card.right() - 16.0, card.top() + 36.0),
                Align2::RIGHT_CENTER,
                value,
                bold(32.0),
                value_color,
            );

            let bar_y = card.top() + 84.0;
            let bar = Rect::from_min_size(
                Pos2::new(card.left() + 16.0, bar_y),
                Vec2::new(card_w - 32.0 - 62.0, 6.0),
            );
            p.rect_filled(bar, radius(3), TRACK);
            if let Some(v) = d.battery.filter(|_| d.connected) {
                let w = bar.width() * (v.min(100) as f32 / 100.0);
                p.rect_filled(
                    Rect::from_min_size(bar.min, Vec2::new(w, bar.height())),
                    radius(3),
                    bar_color(v),
                );
            }
            p.text(
                Pos2::new(card.right() - 16.0, bar_y + 3.0),
                Align2::RIGHT_CENTER,
                d.status_text(),
                regular(12.0),
                MUTED,
            );

            // 悬停显示协议诊断信息，方便现场适配新固件；同时给一点视觉反馈。
            let hover_id = Id::unique(("card", d.key.as_str()));
            let resp = ui.interact(card, hover_id, Sense::hover());
            if resp.hovered() {
                p.rect_stroke(
                    card,
                    radius(16),
                    Stroke::new(1.0, Color32::from_rgb(0xD5, 0xDB, 0xE0)),
                    egui::StrokeKind::Inside,
                );
                if !d.detail.is_empty() {
                    let _ = resp.clone().on_hover_text(d.detail.as_str());
                }
            }
            y += CARD_H + CARD_GAP;
        }
    }

    fn paint(&mut self, ui: &mut egui::Ui, ctx: &Context) {
        // 不透明窗口直接用 egui 视口矩形铺满面板色，圆角由 SetWindowRgn 裁切。
        let screen = ctx.viewport_rect();
        let full = Rect::from_min_size(screen.min, screen.size());
        let p = ui.painter().with_clip_rect(full);
        p.rect_filled(full, radius(24), SHELL_BG);
        // shell 只作为内容排版参考，保留 8px 视觉内边距。
        let shell = full.shrink(8.0);

        // ---- 顶部标题栏 ----
        let hy = shell.top() + 22.0;
        let logo_box = Rect::from_min_size(Pos2::new(shell.left() + 18.0, hy), Vec2::splat(38.0));
        p.rect_filled(logo_box, radius(12), CARD_BG);
        icons::logo(&p, logo_box.center(), GREEN);

        p.text(
            Pos2::new(shell.left() + 66.0, hy + 1.0),
            Align2::LEFT_TOP,
            "电量岛",
            bold(21.0),
            TEXT,
        );
        p.text(
            Pos2::new(shell.left() + 66.0, hy + 29.0),
            Align2::LEFT_TOP,
            "所有设备，一眼可见",
            regular(12.0),
            MUTED,
        );

        let gear = self.icon_button(
            ui,
            &p,
            Pos2::new(shell.right() - 68.0, hy + 19.0),
            "btn-settings",
            icons::sliders,
        );
        if gear {
            let next = match self.page {
                Page::Devices => Page::Settings,
                Page::Settings => Page::Devices,
            };
            self.set_page(ctx, next);
        }
        let close = self.icon_button(
            ui,
            &p,
            Pos2::new(shell.right() - 32.0, hy + 19.0),
            "btn-close",
            icons::close,
        );
        if close {
            self.hide(ctx);
        }

        // ---- 设备卡片 / 设置页 ----
        match self.page {
            Page::Devices => self.paint_cards(ui, &p, shell),
            Page::Settings => self.paint_settings(ui, shell),
        }

        // ---- 底部状态栏 ----
        // 透明窗口下 egui 的可用区域比窗口客户区略高，底部栏要留足余量，
        // 否则「上次更新」和刷新图标会被窗口底边裁掉一半。
        let fy = shell.bottom() - 56.0;
        p.line_segment(
            [Pos2::new(shell.left() + 18.0, fy), Pos2::new(shell.right() - 18.0, fy)],
            Stroke::new(1.0, DIVIDER),
        );
        let stamp = match self.snapshot.updated_at {
            Some(t) => format!("上次更新 {}", human_ago(&t)),
            None => "尚未更新".to_string(),
        };
        let mut label = if self.toast.is_empty() {
            stamp
        } else {
            format!("{stamp}   ·   {}", self.toast)
        };
        let hidden = self.snapshot.devices.len().saturating_sub(MAX_CARDS);
        if hidden > 0 {
            label.push_str(&format!("   ·   +{hidden} 台"));
        }
        p.text(
            Pos2::new(shell.left() + 20.0, fy + 22.0),
            Align2::LEFT_CENTER,
            label,
            regular(12.0),
            MUTED,
        );
        if self.icon_button(
            ui,
            &p,
            Pos2::new(shell.right() - 32.0, fy + 22.0),
            "btn-refresh",
            icons::refresh,
        ) {
            self.refresh();
            self.toast.clear();
        }
    }

    /// 设置作为面板内的一页绘制，不再弹独立窗口：
    /// 独立窗口的标题栏、阴影和深色默认样式跟浅色面板格格不入，还会盖住卡片。
    fn paint_settings(&mut self, ui: &mut egui::Ui, shell: Rect) {
        use egui::{RichText, UiBuilder};

        let rect = Rect::from_min_size(
            Pos2::new(shell.left() + 18.0, shell.top() + 84.0),
            Vec2::new(shell.width() - 36.0, shell.bottom() - 84.0 - 62.0),
        );
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
            |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(8.0, 10.0);
                // 诊断展开后内容会变高，用滚动兜住，不让它顶出面板。
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.label(RichText::new("偏好设置").size(15.0).color(TEXT));
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("自动刷新").size(13.0).color(MUTED));
                            egui::ComboBox::from_id_salt("refresh-interval")
                                .selected_text(format!("{} 秒", self.refresh_secs))
                                .width(88.0)
                                .show_ui(ui, |ui| {
                                    for s in [15u64, 30, 60, 300] {
                                        ui.selectable_value(
                                            &mut self.refresh_secs,
                                            s,
                                            format!("{s} 秒"),
                                        );
                                    }
                                });
                        });
                        ui.checkbox(
                            &mut self.auto_hide,
                            RichText::new("窗口失焦时自动隐藏到托盘").size(13.0),
                        );
                        ui.checkbox(
                            &mut self.demo,
                            RichText::new("演示模式（不读取硬件）").size(13.0),
                        );
                        ui.separator();
                        ui.label(
                            RichText::new("协议：Feature Report 0x11 / 命令 0x06")
                                .size(11.5)
                                .color(MUTED),
                        );
                        ui.label(
                            RichText::new("VID 5253 / 3837 · UsagePage FF01 / FF0B")
                                .size(11.5)
                                .color(MUTED),
                        );
                        egui::CollapsingHeader::new(
                            RichText::new("本次扫描诊断").size(12.5).color(MUTED),
                        )
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(self.snapshot.note.as_str())
                                    .size(11.5)
                                    .color(MUTED),
                            );
                        });
                        ui.horizontal(|ui| {
                            if ui.button("保存并刷新").clicked() {
                                self.apply_settings();
                            }
                            if ui.button("重新扫描").clicked() {
                                self.refresh();
                            }
                        });
                    });
            },
        );
    }
}

impl eframe::App for BatteryApp {
    /// 完全透明清屏，避免 eframe 默认深灰造成的黑边；
    /// 四角形状由 SetWindowRgn 裁切，圆角外透出桌面。
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    /// eframe 0.37 起，窗口隐藏时不会跑 egui pass，但 `logic` 仍会被调用。
    /// 托盘事件、后台快照、显隐控制全部放在这里，保证收进托盘后依然在线。
    fn logic(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        #[cfg(windows)]
        {
            use std::sync::atomic::{AtomicU32, Ordering};
            static APPLIED: AtomicU32 = AtomicU32::new(0);
            let bits = self.current_h.to_bits();
            if APPLIED.swap(bits, Ordering::Relaxed) != bits {
                crate::apply_round_region(WINDOW_W as i32, self.current_h as i32);
            }
        }

        // 「配置已保存」之类的瞬时提示只留 3 秒，否则会一直挂在底部栏。
        if let Some(t) = self.toast_at {
            if t.elapsed() >= Duration::from_secs(3) {
                self.toast.clear();
                self.toast_at = None;
            }
        }

        // 后台线程的扫描结果
        let mut changed = false;
        while let Ok(s) = self.worker.snapshot_rx.try_recv() {
            self.snapshot = s;
            changed = true;
        }
        if changed {
            self.sync_tray();
            // 设备数量变化时收紧/展开面板，避免单设备时留下大片空白。
            let n = self.snapshot.devices.len();
            if n != self.last_device_count {
                self.last_device_count = n;
                // 设置页有自己的固定高度，别被设备数改写的窗口尺寸打断。
                if self.page == Page::Devices {
                    self.current_h = window_height(n);
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                        WINDOW_W,
                        self.current_h,
                    )));
                }
            }
        }

        // 托盘与菜单事件
        while let Ok(ev) = self.ui_rx.try_recv() {
            match ev {
                UiEvent::TrayClick { x, y } => {
                    // Windows 双击托盘会连发两次 Click，第二下会把刚显示的面板又收起来。
                    let now = std::time::Instant::now();
                    let is_double = self
                        .last_tray_click
                        .map(|t| now.duration_since(t) < Duration::from_millis(320))
                        .unwrap_or(false);
                    self.last_tray_click = Some(now);
                    if is_double {
                        continue;
                    }
                    if self.visible {
                        self.hide(ctx);
                    } else {
                        self.show(ctx, Some((x, y)));
                    }
                }
                UiEvent::Toggle => {
                    if self.visible {
                        self.hide(ctx);
                    } else {
                        self.show(ctx, None);
                    }
                }
                UiEvent::Refresh => {
                    self.refresh();
                    self.toast.clear();
                }
                UiEvent::Quit => {
                    let _ = self.worker.cmd_tx.send(WorkerCmd::Quit);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    std::process::exit(0);
                }
            }
        }

        // 右上角 × 只隐藏，不退出
        if self.visible && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.hide(ctx);
        }

        // 失焦自动隐藏（窗口刚显示时尚未获得焦点，was_focused 为 false 可避免立刻收起）
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        if self.auto_hide
            && self.visible
            && self.was_focused
            && !focused
            && self.page == Page::Devices
        {
            self.hide(ctx);
        }
        self.was_focused = focused;

        // 心跳：即使窗口隐藏，也让事件循环定期回来处理托盘 / 扫描结果。
        ctx.request_repaint_after(Duration::from_millis(200));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| self.paint(ui, &ctx));
    }
}

