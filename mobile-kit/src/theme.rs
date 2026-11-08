//! A dark style sized for fingers: 44 pt touch targets, larger text, rounder corners.

use egui::{Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Visuals};

pub const BG: Color32 = Color32::from_rgb(18, 18, 20);
pub const PANEL: Color32 = Color32::from_rgb(28, 28, 32);
pub const RAISED: Color32 = Color32::from_rgb(40, 40, 46);
pub const TEXT: Color32 = Color32::from_rgb(232, 232, 236);
pub const TEXT_DIM: Color32 = Color32::from_rgb(150, 150, 160);

/// Each app's accent colour (the selected chip, slider thumbs, primary buttons).
#[derive(Clone, Copy)]
pub struct Accent(pub Color32);

pub fn accent(ctx: &egui::Context) -> Color32 {
    ctx.data(|d| d.get_temp::<Color32>(egui::Id::unique("craft-accent"))).unwrap_or(Color32::from_rgb(64, 156, 255))
}

/// Minimum size of anything the user taps.
pub const TOUCH: f32 = 44.0;

pub fn apply(ctx: &egui::Context, accent_color: Color32) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("craft-accent"), accent_color));
    let mut v = Visuals::dark();
    v.panel_fill = PANEL;
    v.window_fill = PANEL;
    v.extreme_bg_color = BG;
    v.faint_bg_color = RAISED;
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = accent_color;
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    let r = CornerRadius::same(10);
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open, &mut v.widgets.noninteractive] {
        w.corner_radius = r;
    }
    v.widgets.inactive.weak_bg_fill = RAISED;
    v.widgets.inactive.bg_fill = RAISED;
    v.window_corner_radius = CornerRadius::same(16);
    ctx.set_visuals(v);

    ctx.global_style_mut(|s| {
        s.text_styles = [
            (TextStyle::Small, FontId::new(12.0, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(15.0, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(15.0, FontFamily::Proportional)),
            (TextStyle::Heading, FontId::new(20.0, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(14.0, FontFamily::Monospace)),
        ]
        .into();
        s.spacing.interact_size = egui::vec2(TOUCH, TOUCH);
        s.spacing.button_padding = egui::vec2(14.0, 10.0);
        s.spacing.item_spacing = egui::vec2(8.0, 8.0);
        s.spacing.scroll.floating = true;
        s.interaction.tooltip_delay = 10.0; // no hover on a touch screen
    });
}
