//! 全部图标均为运行时矢量绘制，不依赖外部图片资源。
//! 托盘图标用 RGBA 像素直接生成，随最低电量设备变色。

use crate::model::DeviceKind;
use crate::radius;
use egui::{Color32, Painter, Pos2, Rect, Stroke, Vec2};

const ICON_STROKE: f32 = 1.7;

/// 圆角浅灰方块 + 设备线性图标（每张卡片左侧）。
pub fn device_badge(p: &Painter, rect: Rect, kind: DeviceKind, fg: Color32, bg: Color32) {
    p.rect_filled(rect, radius(12), bg);
    let c = rect.center();
    match kind {
        DeviceKind::Mouse => mouse(p, c, fg),
        DeviceKind::Keyboard => keyboard(p, c, fg),
        DeviceKind::Headset => headset(p, c, fg),
        DeviceKind::Other => generic(p, c, fg),
    }
}

fn mouse(p: &Painter, c: Pos2, color: Color32) {
    let s = Stroke::new(ICON_STROKE, color);
    let body = Rect::from_center_size(c, Vec2::new(15.0, 22.0));
    p.rect_stroke(body, radius(8), s, egui::StrokeKind::Middle);
    p.line_segment([Pos2::new(c.x, body.top() + 1.0), Pos2::new(c.x, body.top() + 9.0)], s);
    p.rect_filled(
        Rect::from_center_size(Pos2::new(c.x, body.top() + 4.0), Vec2::new(2.2, 5.0)),
        radius(1),
        color,
    );
    p.line_segment(
        [Pos2::new(body.left() + 1.2, body.top() + 11.0), Pos2::new(body.right() - 1.2, body.top() + 11.0)],
        s,
    );
}

fn keyboard(p: &Painter, c: Pos2, color: Color32) {
    let s = Stroke::new(ICON_STROKE, color);
    let body = Rect::from_center_size(c, Vec2::new(23.0, 16.0));
    p.rect_stroke(body, radius(3), s, egui::StrokeKind::Middle);
    let dot = 1.5;
    for row in 0..2 {
        for col in 0..5 {
            let x = body.left() + 3.5 + col as f32 * 4.0;
            let y = body.top() + 4.0 + row as f32 * 4.0;
            p.rect_filled(Rect::from_center_size(Pos2::new(x, y), Vec2::splat(dot)), radius(1), color);
        }
    }
    p.line_segment(
        [Pos2::new(body.left() + 5.0, body.bottom() - 3.2), Pos2::new(body.right() - 5.0, body.bottom() - 3.2)],
        s,
    );
}

fn headset(p: &Painter, c: Pos2, color: Color32) {
    let s = Stroke::new(ICON_STROKE, color);
    // 用折线近似头梁圆弧
    let pts = [
        Pos2::new(c.x - 10.0, c.y + 2.0),
        Pos2::new(c.x - 10.0, c.y - 4.0),
        Pos2::new(c.x - 6.0, c.y - 9.0),
        Pos2::new(c.x, c.y - 10.5),
        Pos2::new(c.x + 6.0, c.y - 9.0),
        Pos2::new(c.x + 10.0, c.y - 4.0),
        Pos2::new(c.x + 10.0, c.y + 2.0),
    ];
    for w in pts.windows(2) {
        p.line_segment([w[0], w[1]], s);
    }
    p.rect_filled(Rect::from_center_size(Pos2::new(c.x - 10.0, c.y + 4.0), Vec2::new(5.0, 11.0)), radius(2), color);
    p.rect_filled(Rect::from_center_size(Pos2::new(c.x + 10.0, c.y + 4.0), Vec2::new(5.0, 11.0)), radius(2), color);
    p.line_segment([Pos2::new(c.x + 10.0, c.y + 9.0), Pos2::new(c.x + 5.5, c.y + 11.0)], s);
}

fn generic(p: &Painter, c: Pos2, color: Color32) {
    let s = Stroke::new(ICON_STROKE, color);
    let body = Rect::from_center_size(c, Vec2::new(20.0, 13.0));
    p.rect_stroke(body, radius(3), s, egui::StrokeKind::Middle);
    p.rect_filled(Rect::from_center_size(c - Vec2::new(4.0, 0.0), Vec2::new(8.0, 6.0)), radius(1), color);
}

/// 顶部绿色 logo：圆环 + 三条高低不等的竖线（电平波形）。
pub fn logo(p: &Painter, c: Pos2, color: Color32) {
    let s = Stroke::new(1.9, color);
    p.circle_stroke(c, 11.0, s);
    let bars = [(-5.0, 5.0), (0.0, 9.0), (5.0, 6.0)];
    for (dx, h) in bars {
        p.line_segment(
            [Pos2::new(c.x + dx, c.y + h / 2.0), Pos2::new(c.x + dx, c.y - h / 2.0)],
            Stroke::new(2.2, color),
        );
    }
}

/// 设置：两条滑杆 + 滑块。
pub fn sliders(p: &Painter, c: Pos2, color: Color32) {
    let s = Stroke::new(1.6, color);
    for (dy, knob) in [(-4.0, 2.5), (4.0, -3.0)] {
        p.line_segment(
            [Pos2::new(c.x - 9.0, c.y + dy), Pos2::new(c.x + 9.0, c.y + dy)],
            s,
        );
        p.circle_filled(Pos2::new(c.x + knob, c.y + dy), 2.6, color);
    }
}

/// 返回箭头（设置页左上角）。
pub fn back(p: &Painter, c: Pos2, color: Color32) {
    let s = Stroke::new(1.9, color);
    p.line_segment([Pos2::new(c.x + 5.0, c.y), Pos2::new(c.x - 5.0, c.y)], s);
    p.line_segment([Pos2::new(c.x - 5.0, c.y), Pos2::new(c.x - 1.0, c.y - 4.0)], s);
    p.line_segment([Pos2::new(c.x - 5.0, c.y), Pos2::new(c.x - 1.0, c.y + 4.0)], s);
}

pub fn close(p: &Painter, c: Pos2, color: Color32) {
    let s = Stroke::new(1.7, color);
    p.line_segment([Pos2::new(c.x - 5.0, c.y - 5.0), Pos2::new(c.x + 5.0, c.y + 5.0)], s);
    p.line_segment([Pos2::new(c.x + 5.0, c.y - 5.0), Pos2::new(c.x - 5.0, c.y + 5.0)], s);
}

/// 刷新：一段开口圆弧 + 箭头。
pub fn refresh(p: &Painter, c: Pos2, color: Color32) {
    use std::f32::consts::PI;
    let s = Stroke::new(1.8, color);
    let r = 8.0;
    let start = 0.28 * PI;
    let end = 1.82 * PI;
    let steps = 18;
    let mut prev = None;
    for i in 0..=steps {
        let a = start + (end - start) * (i as f32 / steps as f32);
        let pt = Pos2::new(c.x + r * a.cos(), c.y + r * a.sin());
        if let Some(q) = prev {
            p.line_segment([q, pt], s);
        }
        prev = Some(pt);
    }
    let tip = Pos2::new(c.x + r * end.cos(), c.y + r * end.sin());
    p.line_segment([tip, tip + Vec2::new(-4.5, -1.0)], s);
    p.line_segment([tip, tip + Vec2::new(-0.5, -4.5)], s);
}

// ------------------------------------------------------------------ 托盘图标

struct Canvas {
    w: i32,
    h: i32,
    data: Vec<u8>,
}

impl Canvas {
    fn new(w: i32, h: i32) -> Self {
        Self { w, h, data: vec![0; (w * h * 4) as usize] }
    }

    fn px(&mut self, x: i32, y: i32, c: [u8; 4]) {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return;
        }
        let i = ((y * self.w + x) * 4) as usize;
        if c[3] == 0 {
            self.data[i..i + 4].copy_from_slice(&[0, 0, 0, 0]);
            return;
        }
        // 简单 alpha 合成，便于画半透明分隔刻度
        let a = c[3] as f32 / 255.0;
        for k in 0..3 {
            let old = self.data[i + k] as f32;
            self.data[i + k] = (c[k] as f32 * a + old * (1.0 - a)) as u8;
        }
        self.data[i + 3] = self.data[i + 3].max(c[3]);
    }

    /// 真正的圆角矩形填充，替代以前「画方框再手动抠掉几个角像素」的做法。
    fn round_rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, r: i32, c: [u8; 4]) {
        let r = r.max(0).min((x1 - x0) / 2).min((y1 - y0) / 2);
        for y in y0..y1 {
            for x in x0..x1 {
                let dx = if x < x0 + r {
                    x0 + r - x
                } else if x >= x1 - r {
                    x - (x1 - r - 1)
                } else {
                    0
                };
                let dy = if y < y0 + r {
                    y0 + r - y
                } else if y >= y1 - r {
                    y - (y1 - r - 1)
                } else {
                    0
                };
                if dx * dx + dy * dy <= r * r {
                    self.px(x, y, c);
                }
            }
        }
    }
}

fn battery_color(p: u8) -> [u8; 4] {
    if p < 20 {
        [233, 84, 74, 255]
    } else if p < 40 {
        [240, 160, 56, 255]
    } else {
        [26, 166, 120, 255]
    }
}

/// 生成电池托盘图标（RGBA）。坐标以 32px 为基准等比缩放，
/// 高 DPI 下用 64px 生成比让系统放大 32px 清晰得多。
///
/// 扁平圆角造型：中蓝灰外壳在深色 / 浅色任务栏上都有对比，近白内底避免
/// 透明挖空在深色背景上变成黑洞；电量用整块颜色表示，不放刻度碎线。
pub fn tray_rgba(percent: Option<u8>, size: i32) -> Vec<u8> {
    let size = size.max(16);
    let k = size as f32 / 32.0;
    let s = |v: f32| (v * k).round() as i32;

    let mut c = Canvas::new(size, size);
    let rim = [126, 140, 158, 255];
    let inner = [252, 253, 255, 255];

    // 正极帽
    c.round_rect(s(27.0), s(12.0), s(31.0), s(20.0), s(1.6).max(1), rim);
    // 外壳
    c.round_rect(s(1.0), s(6.0), s(28.0), s(26.0), s(5.5), rim);
    // 内底：留出一圈均匀边框
    c.round_rect(s(3.5), s(8.5), s(25.5), s(23.5), s(3.5), inner);

    match percent {
        None => {
            // 未读到设备：居中的浅灰状态点，不用横杠以免被误读成 0%
            let (cx, cy, r) = (s(14.5), s(16.0), s(3.0).max(1));
            for y in (cy - r)..=(cy + r) {
                for x in (cx - r)..=(cx + r) {
                    if (x - cx) * (x - cx) + (y - cy) * (y - cy) <= r * r {
                        c.px(x, y, [190, 199, 210, 255]);
                    }
                }
            }
        }
        Some(p) => {
            let p = p.min(100);
            let x0 = s(4.5);
            let w = s(24.5) - x0;
            let fw = (w as f32 * p as f32 / 100.0).round() as i32;
            if fw > 0 {
                let r = s(3.0).min(fw / 2).max(0);
                c.round_rect(x0, s(9.5), x0 + fw, s(22.5), r, battery_color(p));
            }
        }
    }
    c.data
}
