//! Touch widgets.

use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2, pos2, vec2};

use crate::theme::{self, RAISED, TEXT, TEXT_DIM, TOUCH};

/// How a slider's track is painted.
#[derive(Clone, Debug)]
pub enum TrackPaint {
    /// Filled from the left end.
    Plain,
    /// Filled from the given position (0…1), e.g. 0.5 for a −100…+100 control.
    Centered(f32),
    /// A gradient through these colours, left to right (white balance, hue, saturation…).
    Gradient(Vec<Color32>),
}

/// What happened to a slider this frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct SliderOut {
    /// The new track position (0…1), when it moved.
    pub moved_to: Option<f64>,
    /// A drag began (start of an undo step).
    pub started: bool,
    /// A drag ended (end of an undo step).
    pub finished: bool,
    /// Double tap: back to the default.
    pub reset: bool,
}

#[derive(Clone, Copy, Default)]
struct DragAxis {
    /// None until the finger has moved far enough to tell a horizontal drag from a scroll.
    horizontal: Option<bool>,
    travel: Vec2,
}

/// A full-width slider row: label and value above a track. Dragging is *relative* — the value
/// moves with the finger wherever the drag starts, so a fingertip never hides what it adjusts and
/// a tap never makes the value jump. A vertical drag scrolls the panel instead. Moving the finger
/// above or below the row while dragging gives fine control (a quarter of the speed).
pub fn slider(ui: &mut Ui, id_salt: impl std::hash::Hash + std::fmt::Debug, label: &str, value_text: &str, pos: f64, track: &TrackPaint, enabled: bool) -> SliderOut {
    let accent = theme::accent(ui.ctx());
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 52.0), if enabled { Sense::click_and_drag() } else { Sense::hover() });
    let id = ui.make_persistent_id(id_salt);
    let mut out = SliderOut::default();
    let track_rect = Rect::from_min_max(pos2(rect.left() + 12.0, rect.bottom() - 16.0), pos2(rect.right() - 12.0, rect.bottom() - 12.0));
    let mut p = pos.clamp(0.0, 1.0);

    if enabled {
        if resp.double_clicked() {
            out.reset = true;
        }
        let mut axis: DragAxis = ui.data(|d| d.get_temp(id)).unwrap_or_default();
        if resp.drag_started() {
            axis = DragAxis::default();
        }
        if resp.dragged() {
            let d = resp.drag_delta();
            match axis.horizontal {
                None => {
                    axis.travel += d;
                    if axis.travel.length() > 6.0 {
                        let h = axis.travel.x.abs() > axis.travel.y.abs();
                        axis.horizontal = Some(h);
                        if h {
                            out.started = true;
                            p += f64::from(axis.travel.x / track_rect.width());
                            out.moved_to = Some(p.clamp(0.0, 1.0));
                        } else {
                            ui.scroll_with_delta(vec2(0.0, axis.travel.y));
                        }
                    }
                }
                Some(true) => {
                    let fine = resp.interact_pointer_pos().is_some_and(|q| (q.y - rect.center().y).abs() > rect.height() * 1.5);
                    let k = if fine { 0.25 } else { 1.0 };
                    p += f64::from(d.x / track_rect.width() * k);
                    out.moved_to = Some(p.clamp(0.0, 1.0));
                }
                Some(false) => ui.scroll_with_delta(vec2(0.0, d.y)),
            }
            ui.data_mut(|dm| dm.insert_temp(id, axis));
        }
        if resp.drag_stopped() {
            if axis.horizontal == Some(true) {
                out.finished = true;
            }
            ui.data_mut(|dm| dm.remove::<DragAxis>(id));
        }
    }
    let p = out.moved_to.unwrap_or(p) as f32;

    let painter = ui.painter_at(rect.expand(4.0));
    let dim = if enabled { 1.0 } else { 0.4 };
    let txt = |c: Color32| c.gamma_multiply(dim);
    painter.text(pos2(rect.left() + 12.0, rect.top() + 14.0), Align2::LEFT_CENTER, label, FontId::proportional(15.0), txt(TEXT));
    painter.text(pos2(rect.right() - 12.0, rect.top() + 14.0), Align2::RIGHT_CENTER, value_text, FontId::monospace(14.0), txt(TEXT_DIM));

    let r = CornerRadius::same(2);
    match track {
        TrackPaint::Gradient(stops) if stops.len() >= 2 => {
            let n = stops.len() - 1;
            for (i, w) in stops.windows(2).enumerate() {
                let x0 = track_rect.left() + track_rect.width() * i as f32 / n as f32;
                let x1 = track_rect.left() + track_rect.width() * (i + 1) as f32 / n as f32;
                let seg = Rect::from_min_max(pos2(x0, track_rect.top()), pos2(x1, track_rect.bottom()));
                gradient_rect(&painter, seg, txt(w[0]), txt(w[1]));
            }
        }
        _ => {
            painter.rect_filled(track_rect, r, RAISED.gamma_multiply(1.6));
            let from = match track {
                TrackPaint::Centered(c) => *c,
                _ => 0.0,
            };
            let (a, b) = if p >= from { (from, p) } else { (p, from) };
            let fill = Rect::from_min_max(
                pos2(track_rect.left() + track_rect.width() * a, track_rect.top()),
                pos2(track_rect.left() + track_rect.width() * b, track_rect.bottom()),
            );
            painter.rect_filled(fill, r, txt(accent));
            if let TrackPaint::Centered(c) = track {
                let x = track_rect.left() + track_rect.width() * c;
                painter.line_segment([pos2(x, track_rect.top() - 4.0), pos2(x, track_rect.bottom() + 4.0)], Stroke::new(1.5, TEXT_DIM));
            }
        }
    }
    let thumb = pos2(track_rect.left() + track_rect.width() * p, track_rect.center().y);
    let active = resp.dragged() || resp.is_pointer_button_down_on();
    painter.circle(thumb, if active { 11.0 } else { 9.0 }, Color32::WHITE.gamma_multiply(dim), Stroke::new(2.0, Color32::from_black_alpha(80)));
    out
}

fn gradient_rect(painter: &egui::Painter, r: Rect, a: Color32, b: Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(r.left_top(), a);
    mesh.colored_vertex(r.right_top(), b);
    mesh.colored_vertex(r.right_bottom(), b);
    mesh.colored_vertex(r.left_bottom(), a);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(mesh);
}

/// A horizontally scrolling row of chips; returns the index tapped this frame.
pub fn chip_row(ui: &mut Ui, id_salt: impl std::hash::Hash + std::fmt::Debug, labels: &[&str], selected: Option<usize>) -> Option<usize> {
    let accent = theme::accent(ui.ctx());
    let mut tapped = None;
    let row_id = ui.make_persistent_id((&id_salt, "chip-sel"));
    egui::ScrollArea::horizontal().id_salt(id_salt).scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            for (i, l) in labels.iter().enumerate() {
                let on = selected == Some(i);
                let text = egui::RichText::new(*l).size(14.5).color(if on { Color32::WHITE } else { TEXT });
                let b = egui::Button::new(text).fill(if on { accent } else { RAISED }).corner_radius(CornerRadius::same(18)).min_size(vec2(0.0, 36.0));
                let r = ui.add(b);
                if r.clicked() {
                    tapped = Some(i);
                }
                // bring a newly selected chip into view (e.g. chosen from elsewhere)
                if on && ui.data(|d| d.get_temp::<usize>(row_id)) != Some(i) {
                    r.scroll_to_me(Some(egui::Align::Center));
                    ui.data_mut(|d| d.insert_temp(row_id, i));
                }
            }
            ui.add_space(8.0);
        });
    });
    tapped
}

/// A round icon-style button (text glyph), TOUCH points square.
pub fn icon_button(ui: &mut Ui, glyph: &str, enabled: bool) -> Response {
    let b = egui::Button::new(egui::RichText::new(glyph).size(20.0)).frame(false).min_size(vec2(TOUCH, TOUCH));
    ui.add_enabled(enabled, b)
}

/// A labelled toolbar button: glyph above a small caption.
pub fn tool_button(ui: &mut Ui, glyph: &str, caption: &str, on: bool) -> Response {
    let accent = theme::accent(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(64.0, 54.0), Sense::click());
    let c = if on { accent } else { TEXT };
    let p = ui.painter_at(rect);
    if resp.is_pointer_button_down_on() {
        p.rect_filled(rect.shrink(2.0), CornerRadius::same(10), RAISED);
    }
    p.text(rect.center() - vec2(0.0, 9.0), Align2::CENTER_CENTER, glyph, FontId::proportional(20.0), c);
    p.text(rect.center() + vec2(0.0, 15.0), Align2::CENTER_CENTER, caption, FontId::proportional(11.5), if on { accent } else { TEXT_DIM });
    resp
}

/// A big filled button for the main action of a sheet.
pub fn primary_button(ui: &mut Ui, text: &str) -> Response {
    let accent = theme::accent(ui.ctx());
    ui.add(egui::Button::new(egui::RichText::new(text).size(16.0).color(Color32::WHITE)).fill(accent).min_size(vec2(ui.available_width(), 48.0)))
}

/// A grab handle at the top of a sheet; returns the vertical drag in points (positive = down).
pub fn sheet_handle(ui: &mut Ui) -> (Response, f32) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::click_and_drag());
    let bar = Rect::from_center_size(rect.center(), vec2(40.0, 5.0));
    ui.painter().rect_filled(bar, CornerRadius::same(3), TEXT_DIM.gamma_multiply(0.6));
    let dy = if resp.dragged() { resp.drag_delta().y } else { 0.0 };
    (resp, dy)
}

/// A modal card centred on the screen with a dimmed backdrop. Returns `false` when the backdrop
/// was tapped (dismiss).
pub fn modal_card(ctx: &egui::Context, id: &str, add: impl FnOnce(&mut Ui)) -> bool {
    let screen = ctx.content_rect();
    let mut keep = true;
    egui::Area::new(egui::Id::unique((id, "backdrop"))).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
        let (r, resp) = ui.allocate_exact_size(screen.size(), Sense::click());
        ui.painter().rect_filled(r, CornerRadius::ZERO, Color32::from_black_alpha(150));
        if resp.clicked() {
            keep = false;
        }
    });
    let w = (screen.width() - 32.0).min(420.0);
    egui::Area::new(egui::Id::unique((id, "card"))).order(egui::Order::Tooltip).anchor(Align2::CENTER_CENTER, Vec2::ZERO).show(ctx, |ui| {
        egui::Frame::new().fill(theme::PANEL).corner_radius(CornerRadius::same(18)).inner_margin(20.0).show(ui, |ui| {
            ui.set_width(w - 40.0);
            add(ui);
        });
    });
    keep
}

/// A spinner with a caption, centred in `rect`.
pub fn busy(ui: &Ui, rect: Rect, caption: &str) {
    let t = ui.input(|i| i.time) as f32;
    let c = rect.center();
    let p = ui.painter();
    for k in 0..8 {
        let a = t * 6.0 + k as f32 * std::f32::consts::TAU / 8.0;
        let alpha = ((k as f32 + 1.0) / 8.0 * 255.0) as u8;
        p.circle_filled(c + vec2(a.cos(), a.sin()) * 16.0, 3.5, Color32::from_white_alpha(alpha));
    }
    p.text(c + vec2(0.0, 36.0), Align2::CENTER_CENTER, caption, FontId::proportional(14.0), TEXT_DIM);
    ui.ctx().request_repaint();
}

/// A thin determinate progress bar.
pub fn progress(ui: &mut Ui, fraction: f32, caption: &str) {
    ui.label(egui::RichText::new(caption).color(TEXT_DIM));
    ui.add(egui::ProgressBar::new(fraction.clamp(0.0, 1.0)).desired_height(8.0));
}

/// Convert a `#rrggbb` hex colour.
pub fn hex(s: &str) -> Color32 {
    let s = s.trim_start_matches('#');
    let v = u32::from_str_radix(s, 16).unwrap_or(0x808080);
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// The colour of a hue in degrees (full saturation, for hue tracks).
pub fn hue(deg: f32) -> Color32 {
    let h = deg.rem_euclid(360.0) / 60.0;
    let x = 1.0 - (h % 2.0 - 1.0).abs();
    let (r, g, b) = match h as u32 {
        0 => (1.0, x, 0.0),
        1 => (x, 1.0, 0.0),
        2 => (0.0, 1.0, x),
        3 => (0.0, x, 1.0),
        4 => (x, 0.0, 1.0),
        _ => (1.0, 0.0, x),
    };
    Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

/// Distance between two points (for handle hit tests).
pub fn dist(a: Pos2, b: Pos2) -> f32 {
    (a - b).length()
}
