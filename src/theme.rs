//! 主题：颜色与尺寸集中在这里，浅色 / 深色 / 跟随系统三档。

use egui::Color32;

/// 面板排版尺寸（逻辑像素）。
pub mod metrics {
    pub const WINDOW_W: f32 = 392.0;
    /// 圆角面板与窗口边缘之间的透明留白
    pub const SHELL_INSET: f32 = 8.0;
    pub const PAD: f32 = 16.0;
    pub const HEADER_H: f32 = 56.0;
    pub const CARD_H: f32 = 92.0;
    pub const CARD_GAP: f32 = 10.0;
    pub const FOOTER_H: f32 = 44.0;
    pub const SETTINGS_H: f32 = 460.0;
    /// 顶栏与第一张卡片之间
    pub const HEADER_GAP: f32 = 10.0;
    /// 最后一张卡片与底栏之间
    pub const CARDS_FOOTER_GAP: f32 = 12.0;
}

/// 圆角规格：外框 / 卡片 / 图标块 / 控件。
pub mod corners {
    use egui::CornerRadius;
    pub const SHELL: CornerRadius = CornerRadius::same(20);
    pub const CARD: CornerRadius = CornerRadius::same(14);
    pub const BADGE: CornerRadius = CornerRadius::same(10);
    pub const CONTROL: CornerRadius = CornerRadius::same(8);
}

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub dark: bool,

    /// 面板底色
    pub shell: Color32,
    /// 卡片 / 悬浮层
    pub card: Color32,
    /// 卡片描边
    pub card_stroke: Color32,
    /// 图标底座
    pub badge: Color32,
    pub divider: Color32,
    pub hover: Color32,
    /// 进度条轨道
    pub track: Color32,

    pub text: Color32,
    pub muted: Color32,
    pub faint: Color32,

    pub accent: Color32,
    pub warn: Color32,
    pub danger: Color32,
    /// 充电中
    pub charging: Color32,
}

impl Theme {
    pub const fn light() -> Self {
        Self {
            dark: false,
            shell: Color32::from_rgb(0xF2, 0xF4, 0xF7),
            card: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            card_stroke: Color32::from_rgb(0xEA, 0xED, 0xF0),
            badge: Color32::from_rgb(0xF1, 0xF3, 0xF6),
            divider: Color32::from_rgb(0xE6, 0xE9, 0xED),
            hover: Color32::from_rgb(0xE9, 0xED, 0xF1),
            track: Color32::from_rgb(0xE6, 0xEA, 0xED),
            text: Color32::from_rgb(0x14, 0x18, 0x1D),
            muted: Color32::from_rgb(0x7C, 0x84, 0x8F),
            faint: Color32::from_rgb(0xA6, 0xAD, 0xB6),
            accent: Color32::from_rgb(0x0E, 0x8C, 0x6D),
            warn: Color32::from_rgb(0xD9, 0x88, 0x29),
            danger: Color32::from_rgb(0xD9, 0x50, 0x45),
            charging: Color32::from_rgb(0x2C, 0x7B, 0xD6),
        }
    }

    pub const fn dark() -> Self {
        Self {
            dark: true,
            shell: Color32::from_rgb(0x16, 0x18, 0x1C),
            card: Color32::from_rgb(0x1F, 0x22, 0x27),
            card_stroke: Color32::from_rgb(0x2A, 0x2E, 0x34),
            badge: Color32::from_rgb(0x26, 0x2A, 0x30),
            divider: Color32::from_rgb(0x2A, 0x2E, 0x34),
            hover: Color32::from_rgb(0x2A, 0x2F, 0x36),
            track: Color32::from_rgb(0x33, 0x38, 0x3F),
            text: Color32::from_rgb(0xEC, 0xEE, 0xF1),
            muted: Color32::from_rgb(0x9A, 0xA1, 0xAB),
            faint: Color32::from_rgb(0x6F, 0x76, 0x7F),
            accent: Color32::from_rgb(0x3E, 0xCF, 0xA0),
            warn: Color32::from_rgb(0xE0, 0xA0, 0x39),
            danger: Color32::from_rgb(0xE0, 0x6B, 0x60),
            charging: Color32::from_rgb(0x5A, 0xA7, 0xF0),
        }
    }

    /// 电量越低越警示；充电中单独用蓝色，避免和「低电量」混淆。
    pub fn battery_color(&self, percent: u8, charging: bool) -> Color32 {
        if charging {
            self.charging
        } else if percent < 20 {
            self.danger
        } else if percent < 40 {
            self.warn
        } else {
            self.accent
        }
    }
}

/// `auto` / `light` / `dark`
pub fn resolve(pref: &str) -> Theme {
    match pref {
        "light" => Theme::light(),
        "dark" => Theme::dark(),
        _ => {
            if system_is_dark() {
                Theme::dark()
            } else {
                Theme::light()
            }
        }
    }
}

/// 把主题同步给 egui 自带控件（复选框、下拉框、按钮），
/// 免得设置页里出现和面板割裂的默认配色。
pub fn apply_to_egui(ctx: &egui::Context, t: &Theme) {
    let mut v = if t.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    v.override_text_color = Some(t.text);
    v.window_fill = t.card;
    v.window_stroke = egui::Stroke::new(1.0, t.card_stroke);
    v.window_corner_radius = corners::CARD;
    v.faint_bg_color = t.badge;
    v.extreme_bg_color = t.badge;
    v.hyperlink_color = t.accent;
    v.selection.bg_fill = t.accent.gamma_multiply(0.28);
    v.selection.stroke = egui::Stroke::new(1.0, t.text);

    v.widgets.noninteractive.bg_fill = t.card;
    v.widgets.noninteractive.fg_stroke.color = t.text;
    v.widgets.inactive.bg_fill = t.badge;
    v.widgets.inactive.weak_bg_fill = t.badge;
    v.widgets.inactive.fg_stroke.color = t.text;
    v.widgets.hovered.bg_fill = t.hover;
    v.widgets.hovered.weak_bg_fill = t.hover;
    v.widgets.hovered.fg_stroke.color = t.text;
    v.widgets.active.bg_fill = t.hover;
    v.widgets.active.weak_bg_fill = t.hover;
    v.widgets.active.fg_stroke.color = t.text;
    for w in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.noninteractive,
    ] {
        w.corner_radius = corners::CONTROL;
    }
    ctx.set_visuals(v);
}

/// 读取 Windows「应用使用深色模式」设置；非 Windows 或读取失败时按浅色处理。
#[cfg(windows)]
fn system_is_dark() -> bool {
    use windows_sys::Win32::System::Registry::{
        RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    let sub = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
    let name = wide("AppsUseLightTheme");
    let mut value: u32 = 1;
    let mut size = std::mem::size_of::<u32>() as u32;
    unsafe {
        let rc = RegGetValueW(
            HKEY_CURRENT_USER,
            sub.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            &mut value as *mut u32 as *mut core::ffi::c_void,
            &mut size,
        );
        if rc == 0 {
            return value == 0;
        }
    }
    false
}

#[cfg(not(windows))]
fn system_is_dark() -> bool {
    false
}
