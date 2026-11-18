//! 「电量岛」面板：无边框圆角窗口，全部内容由 egui 自绘。
//!
//! 颜色与尺寸都从 [`crate::theme`] 取，浅色 / 深色共用同一套布局代码。

use crate::theme::{corners, metrics, Theme};
use crate::{icons, model::*};
use egui::{Align2, Color32, Context, FontId, Id, Painter, Pos2, Rect, Sense, Stroke, Vec2};
use std::sync::mpsc::Receiver;
use std::time::Duration;
use tray_icon::TrayIcon;

pub const WINDOW_W: f32 = metrics::WINDOW_W;
/// 面板最多容纳的卡片数，超出部分不再撑高窗口。
pub const MAX_CARDS: usize = 5;
/// 任务栏之外的可用区域，换算成 egui 的逻辑坐标。
fn work_area_logical(ctx: &Context) -> (f32, f32, f32, f32) {
    let ppp = ctx.pixels_per_point().max(1.0);
    #[cfg(windows)]
    let rect = {
        let (l, t, r, b) = crate::work_area();
        (l / ppp, t / ppp, r / ppp, b / ppp)
    };
    #[cfg(not(windows))]
    let rect = {
        let m = ctx.input(|i| i.viewport().monitor_size.unwrap_or(egui::vec2(1920.0, 1080.0)));
        (0.0, 0.0, m.x, m.y)
    };
    rect
}

/// 按设备数量计算面板高度：单设备时尽量紧凑。
pub fn window_height(count: usize) -> f32 {
    let n = count.clamp(1, MAX_CARDS) as f32;
    metrics::SHELL_INSET * 2.0
        + metrics::HEADER_H
        + metrics::HEADER_GAP
        + n * metrics::CARD_H
        + (n - 1.0) * metrics::CARD_GAP
        + metrics::CARDS_FOOTER_GAP
        + metrics::FOOTER_H
}

fn regular(size: f32) -> FontId {
    FontId::proportional(size)
}
fn bold(size: f32) -> FontId {
    FontId::new(size, egui::FontFamily::Name("Bold".into()))
}

/// 隐藏面板时把窗口挪到屏幕外，而不是 `Visible(false)`。
///
/// 窗口只在启动时 `Visible(true)` 一次，此后常驻「已显示」状态。这样 winit
/// 不会在每次 ShowWindow 时把 WS_CAPTION | WS_BORDER 写回来，也不会在 egui
/// 还没绘制第一帧时先合成一个带标题栏的白色系统窗口——那就是托盘弹出时
/// 肉眼看到的「闪一下」。
fn place_offscreen(ctx: &Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(Pos2::new(
        -32000.0, -32000.0,
    )));
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Devices,
    Settings,
}

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
    pub theme: Theme,
    worker: Worker,
    ui_rx: Receiver<UiEvent>,
    tray: TrayIcon,
    snapshot: Snapshot,
    visible: bool,
    auto_hide: bool,
    was_focused: bool,
    page: Page,
    theme_pref: String,
    refresh_secs: u64,
    demo: bool,
    last_tray_value: Option<Option<u8>>,
    last_device_count: usize,
    current_h: f32,
    toast: String,
    toast_at: Option<std::time::Instant>,
    /// 已经下发给窗口的高度。隐藏期间改高度可能不生效，重新显示时要按它校验。
    applied_h: f32,
    /// 首帧是否已完成「先改窗口样式、再显示」的初始化。
    booted: bool,
    /// 每次显示后都要补几次样式修正：winit 在 ShowWindow 时会把边框样式加回来。
    restyle_queue: Vec<std::time::Instant>,
    last_tray_click: Option<std::time::Instant>,
}

impl BatteryApp {
    pub fn new(
        cfg: Config,
        worker: Worker,
        ui_rx: Receiver<UiEvent>,
        tray: TrayIcon,
        theme: Theme,
    ) -> Self {
        let visible = cfg.show_on_start;
        let auto_hide = cfg.auto_hide;
        let refresh_secs = cfg.refresh_secs.max(5);
        let demo = cfg.demo;
        let theme_pref = cfg.theme.clone();
        Self {
            cfg,
            theme,
            worker,
            ui_rx,
            tray,
            snapshot: Snapshot::default(),
            visible,
            auto_hide,
            was_focused: false,
            page: Page::Devices,
            theme_pref,
            refresh_secs,
            demo,
            last_tray_value: None,
            last_device_count: 0,
            current_h: window_height(if demo { 3 } else { 1 }),
            toast: String::new(),
            toast_at: None,
            last_tray_click: None,
            applied_h: 0.0,
            booted: false,
            restyle_queue: Vec::new(),
        }
    }

    /// 把窗口高度同步到目标值；`force` 时无条件重发（显示前用，防止隐藏期间的改动丢失）。
    fn apply_size(&mut self, ctx: &Context, h: f32, force: bool) {
        self.current_h = h;
        if !force && (h - self.applied_h).abs() < 0.5 {
            return;
        }
        self.applied_h = h;
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(WINDOW_W, h)));
        #[cfg(windows)]
        crate::apply_round_region(WINDOW_W as i32, h as i32);
        ctx.request_repaint();
    }

    fn hide(&mut self, ctx: &Context) {
        self.visible = false;
        self.was_focused = false;
        // 收起后下次打开回到设备页，避免一开面板就停在设置里。
        self.page = Page::Devices;
        place_offscreen(ctx);
    }

    fn set_page(&mut self, ctx: &Context, page: Page) {
        self.page = page;
        let h = match page {
            Page::Settings => metrics::SETTINGS_H,
            Page::Devices => window_height(self.snapshot.devices.len()),
        };
        if (h - self.current_h).abs() > 0.5 {
            self.apply_size(ctx, h, false);
            // 高度变了要重新贴边，否则从设置页切回设备页会留下空档。
            self.place(ctx);
        }
    }

    fn apply_theme(&mut self, ctx: &Context) {
        self.theme = crate::theme::resolve(&self.theme_pref);
        crate::theme::apply_to_egui(ctx, &self.theme);
        self.cfg.theme = self.theme_pref.clone();
        let _ = self.cfg.save();
        ctx.request_repaint();
    }

    fn show(&mut self, ctx: &Context, _at: Option<(f64, f64)>) {
        self.visible = true;
        // 重新显示时先不认为「曾经获得焦点」，否则会被失焦自动隐藏立刻收起。
        self.was_focused = false;

        // 收起时页面会回到设备页，但窗口高度可能还停在设置页的 432，
        // 直接用旧高度弹出来就会多出一大块空白，所以显示前先按当前页面复位。
        let target_h = match self.page {
            Page::Settings => metrics::SETTINGS_H,
            Page::Devices => window_height(self.snapshot.devices.len()),
        };
        // 窗口常驻可见后尺寸变更都会生效，只有高度真的不同才重发，
        // 避免每次弹出都触发一次无谓的窗口重设（会闪）。
        self.apply_size(ctx, target_h, false);

        // 窗口一直处于显示状态，这里只需要把它从屏幕外挪回来。
        self.place(ctx);
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        ctx.request_repaint();
    }

    /// 固定贴工作区右下角。
    ///
    /// 旧实现按托盘点击坐标算 `x - 宽 + 46`：图标落进「隐藏图标」溢出区时
    /// 面板会整体左移一百多像素，与首次启动的右下角不一致；窗口移动又会在
    /// 透明窗口上留下旧位置的残影。统一到右下角后每次弹出位置都相同。
    fn place(&self, ctx: &Context) {
        let h = self.current_h;
        let (wl, wt, wr, wb) = work_area_logical(ctx);
        let left = (wr - WINDOW_W - 12.0).max(wl + 8.0);
        let top = (wb - h - 12.0).max(wt + 8.0);
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(Pos2::new(left, top)));
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
        self.cfg.theme = self.theme_pref.clone();
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
        let r = Rect::from_center_size(center, Vec2::splat(28.0));
        let resp = ui.interact(r, Id::unique(id), Sense::click());
        let t = &self.theme;
        if resp.hovered() {
            p.rect_filled(r, corners::CONTROL, t.hover);
        }
        let color = if resp.hovered() { t.text } else { t.muted };
        draw(p, center, color);
        resp.clicked()
    }

    /// 单张设备卡片。
    fn paint_card(&self, ui: &mut egui::Ui, p: &Painter, card: Rect, d: &DeviceState) {
        let t = &self.theme;
        p.rect_filled(card, corners::CARD, t.card);
        p.rect_stroke(
            card,
            corners::CARD,
            Stroke::new(1.0, t.card_stroke),
            egui::StrokeKind::Inside,
        );

        let badge = Rect::from_min_size(
            Pos2::new(card.left() + 14.0, card.top() + 14.0),
            Vec2::splat(36.0),
        );
        icons::device_badge(p, badge, d.kind, t.muted, t.badge);

        p.text(
            Pos2::new(card.left() + 62.0, card.top() + 15.0),
            Align2::LEFT_TOP,
            &d.name,
            bold(14.0),
            t.text,
        );
        p.text(
            Pos2::new(card.left() + 62.0, card.top() + 34.0),
            Align2::LEFT_TOP,
            &d.model,
            regular(11.0),
            t.muted,
        );

        let value = match d.battery {
            Some(v) if d.connected => format!("{v}%"),
            _ => "—".to_string(),
        };
        let value_color = if d.connected { t.text } else { t.faint };
        p.text(
            Pos2::new(card.right() - 14.0, card.top() + 28.0),
            Align2::RIGHT_CENTER,
            value,
            bold(22.0),
            value_color,
        );

        // 进度条
        let bar_cy = card.top() + 68.0;
        let bar_right = card.right() - 14.0 - 54.0;
        let bar = Rect::from_min_max(
            Pos2::new(card.left() + 14.0, bar_cy - 2.0),
            Pos2::new(bar_right, bar_cy + 2.0),
        );
        p.rect_filled(bar, corners::CONTROL, t.track);
        if let Some(v) = d.battery.filter(|_| d.connected) {
            let w = bar.width() * (v.min(100) as f32 / 100.0);
            if w > 0.5 {
                p.rect_filled(
                    Rect::from_min_size(bar.min, Vec2::new(w, bar.height())),
                    corners::CONTROL,
                    t.battery_color(v, d.charging),
                );
            }
        }

        // 状态：小圆点 + 文字，右对齐到卡片内边距
        let (dot, label) = if !d.connected {
            (t.faint, d.status_text())
        } else if d.charging {
            (t.charging, d.status_text())
        } else {
            (t.accent, d.status_text())
        };
        let galley = p.layout_no_wrap(label, regular(11.0), t.muted);
        let tw = galley.size().x;
        let x_right = card.right() - 14.0;
        p.galley(
            Pos2::new(x_right - tw, bar_cy - galley.size().y / 2.0),
            galley,
            t.muted,
        );
        p.circle_filled(Pos2::new(x_right - tw - 9.0, bar_cy), 2.5, dot);

        // 悬停显示协议诊断，同时给一点视觉反馈
        let hover_id = Id::unique(("card", d.key.as_str()));
        let resp = ui.interact(card, hover_id, Sense::hover());
        if resp.hovered() {
            p.rect_stroke(
                card,
                corners::CARD,
                Stroke::new(1.0, t.faint),
                egui::StrokeKind::Inside,
            );
            if !d.detail.is_empty() {
                let _ = resp.clone().on_hover_text(d.detail.as_str());
            }
        }
    }

    fn paint_cards(&self, ui: &mut egui::Ui, p: &Painter, shell: Rect) {
        let content = Rect::from_min_max(
            Pos2::new(shell.left() + metrics::PAD, shell.top() + metrics::HEADER_H + metrics::HEADER_GAP),
            Pos2::new(shell.right() - metrics::PAD, shell.bottom()),
        );
        let mut y = content.top();
        for d in self.snapshot.devices.iter().take(MAX_CARDS) {
            let card = Rect::from_min_size(
                Pos2::new(content.left(), y),
                Vec2::new(content.width(), metrics::CARD_H),
            );
            self.paint_card(ui, p, card, d);
            y += metrics::CARD_H + metrics::CARD_GAP;
        }
    }

    /// 设置作为面板内的一页绘制，不再弹独立窗口。
    fn paint_settings(&mut self, ui: &mut egui::Ui, ctx: &Context, p: &Painter, shell: Rect) {
        use egui::{RichText, UiBuilder};

        let t = self.theme;
        let card = Rect::from_min_max(
            Pos2::new(shell.left() + metrics::PAD, shell.top() + metrics::HEADER_H + metrics::HEADER_GAP),
            Pos2::new(
                shell.right() - metrics::PAD,
                shell.bottom() - metrics::FOOTER_H - metrics::CARDS_FOOTER_GAP,
            ),
        );
        p.rect_filled(card, corners::CARD, t.card);
        p.rect_stroke(
            card,
            corners::CARD,
            Stroke::new(1.0, t.card_stroke),
            egui::StrokeKind::Inside,
        );

        let inner = card.shrink(metrics::PAD);
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(inner)
                .layout(egui::Layout::top_down(egui::Align::Min)),
            |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(8.0, 10.0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.label(RichText::new("外观").size(11.0).color(t.muted));
                        ui.horizontal(|ui| {
                            for (value, label) in
                                [("auto", "跟随系统"), ("light", "浅色"), ("dark", "深色")]
                            {
                                if ui
                                    .selectable_label(self.theme_pref == value, label)
                                    .clicked()
                                {
                                    self.theme_pref = value.to_string();
                                    self.apply_theme(ctx);
                                }
                            }
                        });

                        ui.add_space(2.0);
                        ui.label(RichText::new("偏好设置").size(11.0).color(t.muted));
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("自动刷新").size(12.5).color(t.text));
                            egui::ComboBox::from_id_salt("refresh-interval")
                                .selected_text(format!("{} 秒", self.refresh_secs))
                                .width(84.0)
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
                            RichText::new("窗口失焦时自动隐藏到托盘").size(12.5),
                        );
                        ui.checkbox(
                            &mut self.demo,
                            RichText::new("演示模式（不读取硬件）").size(12.5),
                        );

                        ui.add_space(2.0);
                        ui.separator();
                        ui.label(
                            RichText::new("迈从 Feature 0x11/0x06 · 狼蛛查询 20 01 · 33 字节")
                                .size(11.0)
                                .color(t.faint),
                        );
                        egui::CollapsingHeader::new(
                            RichText::new("本次扫描诊断").size(11.5).color(t.muted),
                        )
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(self.snapshot.note.as_str())
                                    .size(11.0)
                                    .color(t.faint),
                            );
                        });
                        ui.horizontal(|ui| {
                            if ui.button("保存").clicked() {
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

    fn paint(&mut self, ui: &mut egui::Ui, ctx: &Context) {
        let t = self.theme;
        // 窗口样式已改成 WS_POPUP，客户区与窗口矩形重合，直接按 (0,0) 起绘制。
        // 最顶端 1px 由窗口区域裁掉（系统边框），所以面板从第 1 行开始可见。
        let panel = Rect::from_min_size(Pos2::new(0.0, 0.0), Vec2::new(WINDOW_W, self.current_h));
        let p = ui.painter().with_clip_rect(panel);
        p.rect_filled(panel, corners::SHELL, t.shell);
        let full = panel;
        let _ = full;

        // 面板四周留 SHELL_INSET 作为视觉边距。
        let shell = full.shrink(metrics::SHELL_INSET);
        let header_cy = shell.top() + metrics::HEADER_H / 2.0 + 3.0;

        // ---- 顶栏 ----
        let logo = Rect::from_min_size(
            Pos2::new(shell.left() + metrics::PAD, shell.top() + 13.0),
            Vec2::splat(32.0),
        );
        p.rect_filled(logo, corners::BADGE, t.card);
        icons::logo(&p, logo.center(), t.accent);

        p.text(
            Pos2::new(logo.right() + 10.0, logo.top() - 2.0),
            Align2::LEFT_TOP,
            "电量岛",
            bold(17.0),
            t.text,
        );
        let total = self.snapshot.devices.len();
        let online = self.snapshot.devices.iter().filter(|d| d.connected).count();
        let subtitle = if total == 0 {
            "正在扫描设备".to_string()
        } else if online == 0 {
            format!("{total} 台设备 · 均未响应")
        } else {
            format!("{online} / {total} 台在线")
        };
        p.text(
            Pos2::new(logo.right() + 10.0, logo.top() + 21.0),
            Align2::LEFT_TOP,
            subtitle,
            regular(11.0),
            t.muted,
        );

        let gear_center = Pos2::new(shell.right() - metrics::PAD - 14.0, header_cy);
        let close_center = Pos2::new(gear_center.x - 34.0, header_cy);
        if self.icon_button(ui, &p, gear_center, "btn-settings", icons::sliders) {
            let next = match self.page {
                Page::Devices => Page::Settings,
                Page::Settings => Page::Devices,
            };
            self.set_page(ctx, next);
        }
        if self.icon_button(ui, &p, close_center, "btn-close", icons::close) {
            self.hide(ctx);
        }

        // ---- 主体 ----
        match self.page {
            Page::Devices => self.paint_cards(ui, &p, shell),
            Page::Settings => self.paint_settings(ui, ctx, &p, shell),
        }

        // ---- 底栏 ----
        let fy = shell.bottom() - metrics::FOOTER_H;
        let fcy = fy + metrics::FOOTER_H / 2.0;
        p.line_segment(
            [
                Pos2::new(shell.left() + metrics::PAD, fy),
                Pos2::new(shell.right() - metrics::PAD, fy),
            ],
            Stroke::new(1.0, t.divider),
        );
        let stamp = match self.snapshot.updated_at {
            Some(ts) => format!("上次更新 {}", human_ago(&ts)),
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
            Pos2::new(shell.left() + metrics::PAD + 2.0, fcy),
            Align2::LEFT_CENTER,
            label,
            regular(11.0),
            t.muted,
        );
        if self.icon_button(
            ui,
            &p,
            Pos2::new(shell.right() - metrics::PAD - 14.0, fcy),
            "btn-refresh",
            icons::refresh,
        ) {
            self.refresh();
            self.toast.clear();
        }
    }
}

impl eframe::App for BatteryApp {
    /// 完全透明清屏，四角形状由 SetWindowRgn 裁切，圆角外透出桌面。
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    /// eframe 0.37 起，窗口隐藏时不会跑 egui pass，但 `logic` 仍会被调用。
    fn logic(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        #[cfg(windows)]
        {
            if !self.booted {
                // 窗口是隐藏创建的。趁它还没露面，先把样式改成无边框并裁好圆角，
                // 否则用户会看到一帧系统标题栏，而且客户区会带着 8/31px 的偏移。
                self.booted = true;
                // 触发一次尺寸同步，让渲染表面按新客户区重建。
                let h = self.current_h;
                self.apply_size(ctx, h, true);
                // 整个进程只在这里 Visible(true) 一次；之后收起面板只是把窗口
                // 挪到屏幕外，窗口本身一直可见、一直有已渲染好的画面。
                if self.cfg.show_on_start {
                    self.place(ctx);
                } else {
                    place_offscreen(ctx);
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                // 首次 ShowWindow 后 winit 可能回写边框样式，稍后补修一次。
                self.restyle_queue
                    .push(std::time::Instant::now() + Duration::from_millis(120));
                ctx.request_repaint();
            }
            let now = std::time::Instant::now();
            if self.restyle_queue.iter().any(|t| now >= *t) {
                self.restyle_queue.retain(|t| now < *t);
                crate::apply_round_region(WINDOW_W as i32, self.current_h as i32);
            }
        }

        #[cfg(windows)]
        {
            use std::sync::atomic::{AtomicU32, Ordering};
            static APPLIED: AtomicU32 = AtomicU32::new(0);
            let bits = self.current_h.to_bits();
            if APPLIED.swap(bits, Ordering::Relaxed) != bits {
                crate::apply_round_region(WINDOW_W as i32, self.current_h as i32);
            }
        }

        // 「配置已保存」之类的瞬时提示只留 3 秒。
        if let Some(ts) = self.toast_at {
            if ts.elapsed() >= Duration::from_secs(3) {
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
            let n = self.snapshot.devices.len();
            if n != self.last_device_count {
                self.last_device_count = n;
                // 设置页有自己的固定高度，别被设备数改写的窗口尺寸打断。
                if self.page == Page::Devices {
                    self.apply_size(ctx, window_height(n), false);
                    if self.visible {
                        self.place(ctx);
                    }
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
                        .map(|ts| now.duration_since(ts) < Duration::from_millis(320))
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
        // 不再套 CentralPanel：它会在 root Ui 里留出内边距，
        // 导致所有自绘内容整体右下偏移（实测约 (8, 31)）。
        let ctx = ui.ctx().clone();
        self.paint(ui, &ctx);
    }
}
