use gpui_kit::{
    component::{Theme, ThemeMode},
    *,
};

pub const BG: u32 = 0x0c0e14;
pub const SURFACE: u32 = 0x141722;
pub const SURFACE_2: u32 = 0x1b2030;
pub const SURFACE_HOVER: u32 = 0x23293d;
pub const BORDER: u32 = 0x272d42;
pub const BORDER_STRONG: u32 = 0x37405e;
pub const TEXT: u32 = 0xeef1f8;
pub const MUTED: u32 = 0x9aa3bd;

pub const ACCENT: u32 = 0x9333ea;
pub const ACCENT_HOVER: u32 = 0xa855f7;
pub const ACCENT_ACTIVE: u32 = 0x7e22ce;
pub const ACCENT_TEXT: u32 = 0xd8b4fe;

pub const GREEN: u32 = 0x10b981;
pub const GREEN_TEXT: u32 = 0x6ee7b7;
pub const GREEN_BANNER_TEXT: u32 = 0xd1fae5;

pub const AMBER: u32 = 0xf59e0b;
pub const AMBER_TEXT: u32 = 0xfcd34d;

pub const RED: u32 = 0xef4444;
pub const RED_TEXT: u32 = 0xfca5a5;
pub const RED_BANNER_TEXT: u32 = 0xfecaca;

pub const CYAN: u32 = 0x06b6d4;
pub const CYAN_TEXT: u32 = 0x67e8f9;

pub const INFO_LOG: u32 = 0xcbd5e1;

pub fn color_alpha(hex: u32, alpha: f32) -> Hsla {
    let mut c: Hsla = rgb(hex).into();
    c.a = alpha;
    c
}

#[derive(Clone, Copy)]
pub enum PillTone {
    Neutral,
    Purple,
    Green,
    Amber,
    Red,
    Cyan,
}

pub fn pill(label: impl Into<SharedString>, tone: PillTone) -> Div {
    let (bg, border, text): (Hsla, Hsla, Hsla) = match tone {
        PillTone::Neutral => (rgb(SURFACE_2).into(), rgb(BORDER).into(), rgb(MUTED).into()),
        PillTone::Purple => (
            color_alpha(ACCENT, 0.16),
            color_alpha(ACCENT, 0.42),
            rgb(ACCENT_TEXT).into(),
        ),
        PillTone::Green => (
            color_alpha(GREEN, 0.16),
            color_alpha(GREEN, 0.42),
            rgb(GREEN_TEXT).into(),
        ),
        PillTone::Amber => (
            color_alpha(AMBER, 0.16),
            color_alpha(AMBER, 0.42),
            rgb(AMBER_TEXT).into(),
        ),
        PillTone::Red => (
            color_alpha(RED, 0.16),
            color_alpha(RED, 0.42),
            rgb(RED_TEXT).into(),
        ),
        PillTone::Cyan => (
            color_alpha(CYAN, 0.16),
            color_alpha(CYAN, 0.42),
            rgb(CYAN_TEXT).into(),
        ),
    };

    div()
        .px(px(10.))
        .py(px(3.))
        .rounded_full()
        .bg(bg)
        .border_1()
        .border_color(border)
        .text_color(text)
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .child(label.into())
}

pub fn apply_app_theme(cx: &mut App) {
    // Switch to dark mode first so its dark base palette loads cleanly.
    Theme::update(cx, |theme| {
        theme.mode = ThemeMode::Dark;
    });

    // Apply our studio NLE dark theme palette onto gpui-component + gpui-base.
    Theme::update(cx, |theme| {
        theme.radius = px(8.);
        theme.radius_lg = px(12.);

        let c = &mut theme.colors;
        c.background = rgb(BG).into();
        c.foreground = rgb(TEXT).into();
        c.muted = rgb(SURFACE_2).into();
        c.muted_foreground = rgb(MUTED).into();
        c.border = rgb(BORDER).into();
        c.input = rgb(BORDER_STRONG).into();
        c.ring = rgb(ACCENT).into();

        c.popover = rgb(SURFACE).into();
        c.popover_foreground = rgb(TEXT).into();

        c.list = rgb(SURFACE).into();
        c.list_even = rgb(SURFACE).into();
        c.list_hover = rgb(SURFACE_HOVER).into();
        c.list_active = color_alpha(ACCENT, 0.22);
        c.list_active_border = rgb(ACCENT).into();

        c.primary = rgb(ACCENT).into();
        c.primary_hover = rgb(ACCENT_HOVER).into();
        c.primary_active = rgb(ACCENT_ACTIVE).into();
        c.primary_foreground = rgb(0xffffff).into();

        c.button_primary = rgb(ACCENT).into();
        c.button_primary_hover = rgb(ACCENT_HOVER).into();
        c.button_primary_active = rgb(ACCENT_ACTIVE).into();
        c.button_primary_foreground = rgb(0xffffff).into();

        c.secondary = rgb(SURFACE_2).into();
        c.secondary_hover = rgb(SURFACE_HOVER).into();
        c.secondary_active = rgb(BORDER).into();
        c.secondary_foreground = rgb(TEXT).into();

        c.button = rgb(SURFACE_2).into();
        c.button_hover = rgb(SURFACE_HOVER).into();
        c.button_active = rgb(BORDER).into();
        c.button_foreground = rgb(TEXT).into();

        c.accent = color_alpha(ACCENT, 0.22);
        c.accent_foreground = rgb(TEXT).into();

        c.switch = rgb(BORDER_STRONG).into();
        c.switch_thumb = rgb(0xffffff).into();

        c.success = rgb(GREEN).into();
        c.warning = rgb(AMBER).into();
        c.danger = rgb(RED).into();
        c.info = rgb(CYAN).into();
    });
}
