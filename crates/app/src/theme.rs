//! Colours and fonts. The palette is matched to FL Studio's default dark skin.

use goaty_engine::Rgb;
use egui::{Color32, FontData, FontDefinitions, FontFamily, FontId, Stroke};
use std::sync::Arc;

const fn c(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

// Global chrome.
pub const TOP_BG: Color32 = c(0x262c2e);
pub const TOP_BG2: Color32 = c(0x2c3334);
pub const TOP_SEP: Color32 = c(0x1a1e1f);
pub const WINDOW_BG: Color32 = c(0x232527);
pub const TITLE_BG: Color32 = c(0x25292b);
pub const WORKSPACE: Color32 = c(0x2f3436);
pub const BUTTON: Color32 = c(0x333a3c);
pub const BUTTON_HOVER: Color32 = c(0x3f4749);

// Text.
pub const TEXT: Color32 = c(0xc3cacd);
pub const TEXT_BRIGHT: Color32 = c(0xe8ecee);
pub const TEXT_DIM: Color32 = c(0x7d878c);
pub const TEXT_FAINT: Color32 = c(0x565f63);
pub const HINT: Color32 = c(0xc0935e);

// Indicators.
pub const LED_ON: Color32 = c(0xace25c);
pub const LED_OFF: Color32 = c(0x3b4136);
pub const SONG_ON: Color32 = c(0xe6e960);
pub const RECORD: Color32 = c(0xe8384f);
pub const ORANGE: Color32 = c(0xf2a33a);
pub const SELECT_GREEN: Color32 = c(0x9ad14f);
pub const PLAYHEAD: Color32 = c(0x7fe36a);

// Browser.
pub const BROWSER_BG: Color32 = c(0x1d2123);
pub const BROWSER_OPEN: Color32 = c(0x242528);
pub const BROWSER_SEL: Color32 = c(0x30363a);
pub const BROWSER_TEXT: Color32 = c(0xb5bfc5);
pub const BROWSER_ICON: Color32 = c(0x9aa7ae);

// Playlist / piano roll grids.
pub const GRID_A: Color32 = c(0x2b3135);
pub const GRID_B: Color32 = c(0x262b2f);
pub const GRID_LINE: Color32 = c(0x22272a);
pub const GRID_BEAT: Color32 = c(0x1b1f22);
pub const GRID_BAR: Color32 = c(0x101315);
pub const RULER_BG: Color32 = c(0x1c2023);
pub const OVERVIEW_BG: Color32 = c(0x33373a);
pub const TRACK_HDR: Color32 = c(0x2a2c2e);
pub const PATPICK_BG: Color32 = c(0x202527);
pub const PATPICK_ITEM: Color32 = c(0x27292a);
pub const CLIP_DEFAULT: Color32 = c(0x5a6d78);
pub const AUDIO_CLIP_DEFAULT: Color32 = c(0x4a5357);

// Piano roll.
pub const PR_ROW_WHITE: Color32 = c(0x353b3f);
pub const PR_ROW_BLACK: Color32 = c(0x2c3135);
pub const PR_NOTE: Color32 = c(0x7cc96b);
pub const KEY_WHITE: Color32 = c(0xdfe3e5);
pub const KEY_BLACK: Color32 = c(0x1b1e20);

// Channel rack.
pub const STEP_A: Color32 = c(0x3e4449);
pub const STEP_B: Color32 = c(0x553f47);
pub const STEP_ON: Color32 = c(0xc7cfd3);
pub const RACK_ROW: Color32 = c(0x2a2e31);
pub const RACK_NAME: Color32 = c(0x464d51);

// Mixer.
pub const MIX_BG: Color32 = c(0x232527);
pub const MIX_STRIP: Color32 = c(0x1e2022);
pub const MIX_CARD: Color32 = c(0x191b1c);
pub const MIX_HDR: Color32 = c(0x323537);
pub const MIX_MASTER: Color32 = c(0x232628);
pub const MIX_PANEL: Color32 = c(0x313436);
pub const SLOT_ROW: Color32 = c(0x292c2e);
pub const KNOB: Color32 = c(0x242629);
pub const KNOB_RIM: Color32 = c(0x131516);
pub const FADER_HANDLE: Color32 = c(0x595a5d);
pub const FADER_TRACK: Color32 = c(0x131517);
pub const METER_BG: Color32 = c(0x181a1b);
pub const METER_LOW: Color32 = c(0x8fd14f);
pub const METER_MID: Color32 = c(0xe3d14a);
pub const METER_HIGH: Color32 = c(0xe8484a);

pub fn rgb(c: Rgb) -> Color32 {
    Color32::from_rgb(c.0, c.1, c.2)
}

pub fn lighten(c: Color32, amount: f32) -> Color32 {
    let f = |v: u8| (v as f32 + (255.0 - v as f32) * amount) as u8;
    Color32::from_rgb(f(c.r()), f(c.g()), f(c.b()))
}

pub fn darken(c: Color32, amount: f32) -> Color32 {
    let f = |v: u8| (v as f32 * (1.0 - amount)) as u8;
    Color32::from_rgb(f(c.r()), f(c.g()), f(c.b()))
}

pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

pub fn font(size: f32) -> FontId {
    FontId::proportional(size)
}

/// The slightly heavier UI font.
pub fn font_medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("medium".into()))
}

fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "barlow".into(),
        Arc::new(FontData::from_static(include_bytes!("../assets/fonts/BarlowSemiCondensed-Regular.ttf"))),
    );
    fonts.font_data.insert(
        "barlow_medium".into(),
        Arc::new(FontData::from_static(include_bytes!("../assets/fonts/BarlowSemiCondensed-Medium.ttf"))),
    );
    // A system font with wide script coverage (Arabic, symbols) as fallback.
    let fallback = ["C:\\Windows\\Fonts\\segoeui.ttf", "/System/Library/Fonts/SFNS.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"]
        .iter()
        .find_map(|p| std::fs::read(p).ok());
    if let Some(bytes) = fallback {
        fonts.font_data.insert("system".into(), Arc::new(FontData::from_owned(bytes)));
    }
    let has_system = fonts.font_data.contains_key("system");

    let prop = fonts.families.entry(FontFamily::Proportional).or_default();
    prop.insert(0, "barlow".into());
    if has_system {
        prop.push("system".into());
    }
    let mut medium = vec!["barlow_medium".to_string(), "barlow".to_string()];
    if has_system {
        medium.push("system".into());
    }
    fonts.families.insert(FontFamily::Name("medium".into()), medium);
    if has_system {
        fonts.families.entry(FontFamily::Monospace).or_default().push("system".into());
    }
    ctx.set_fonts(fonts);
}

pub fn apply(ctx: &egui::Context) {
    setup_fonts(ctx);
    let mut v = egui::Visuals::dark();
    v.panel_fill = WINDOW_BG;
    v.window_fill = WINDOW_BG;
    v.extreme_bg_color = c(0x181b1c);
    v.faint_bg_color = c(0x2a2d2f);
    v.window_stroke = Stroke::new(1.0_f32, c(0x15181a));
    v.window_shadow = egui::Shadow { offset: [0, 4], blur: 12, spread: 0, color: Color32::from_black_alpha(120) };
    v.popup_shadow = egui::Shadow { offset: [0, 3], blur: 8, spread: 0, color: Color32::from_black_alpha(120) };
    v.window_corner_radius = egui::CornerRadius::same(2);
    v.menu_corner_radius = egui::CornerRadius::same(2);
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = c(0x4a5a3a);
    v.selection.stroke = Stroke::new(1.0_f32, LED_ON);
    v.hyperlink_color = HINT;
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = WINDOW_BG;
    w.noninteractive.weak_bg_fill = WINDOW_BG;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, c(0x34393b));
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    for (state, fill) in [(&mut w.inactive, BUTTON), (&mut w.hovered, BUTTON_HOVER), (&mut w.active, c(0x4b5457)), (&mut w.open, BUTTON_HOVER)] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.corner_radius = egui::CornerRadius::same(2);
        state.bg_stroke = Stroke::new(1.0_f32, c(0x1b1f20));
    }
    w.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT_BRIGHT);
    w.active.fg_stroke = Stroke::new(1.0_f32, TEXT_BRIGHT);
    ctx.set_visuals(v);
    ctx.style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(5.0, 4.0);
        s.spacing.button_padding = egui::vec2(6.0, 2.0);
        s.spacing.interact_size.y = 20.0;
        s.spacing.window_margin = egui::Margin::same(0);
        use egui::TextStyle::*;
        s.text_styles.insert(Body, font(14.5));
        s.text_styles.insert(Button, font(14.5));
        s.text_styles.insert(Small, font(12.0));
        s.text_styles.insert(Heading, font_medium(18.0));
        s.text_styles.insert(Monospace, FontId::monospace(13.0));
    });
}
