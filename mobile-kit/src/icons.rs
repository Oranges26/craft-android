//! Vector icons drawn with the painter, so they look the same on every phone whatever fonts it
//! has (egui's bundled fonts lack most arrow and media glyphs).

use egui::{Color32, Pos2, Rect, Shape, Stroke, pos2, vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Back,
    Undo,
    Redo,
    Compare,
    RotateLeft,
    RotateRight,
    FlipH,
    FlipV,
    Reset,
    Plus,
    Play,
    Pause,
    Split,
    Trash,
    Speed,
    Volume,
    Duplicate,
    Text,
    Transition,
    Sparkle,
    ToStart,
    Check,
    Close,
    Music,
    Photo,
    Film,
}

/// Draw `icon` centred in `rect` (its size is the icon's box).
pub fn paint(p: &egui::Painter, icon: Icon, rect: Rect, color: Color32) {
    let s = rect.width().min(rect.height());
    let c = rect.center();
    let k = s / 24.0; // icons are designed on a 24-unit grid
    let at = |x: f32, y: f32| pos2(c.x + (x - 12.0) * k, c.y + (y - 12.0) * k);
    let st = Stroke::new(2.0 * k.max(0.75), color);
    let line = |pts: &[(f32, f32)]| Shape::line(pts.iter().map(|(x, y)| at(*x, *y)).collect(), st);
    let arc = |cx: f32, cy: f32, r: f32, a0: f32, a1: f32| {
        let n = 24;
        let pts: Vec<Pos2> = (0..=n).map(|i| {
            let a = (a0 + (a1 - a0) * i as f32 / n as f32).to_radians();
            at(cx + r * a.cos(), cy + r * a.sin())
        }).collect();
        Shape::line(pts, st)
    };
    let fill = |pts: &[(f32, f32)]| Shape::convex_polygon(pts.iter().map(|(x, y)| at(*x, *y)).collect(), color, Stroke::NONE);
    let shapes: Vec<Shape> = match icon {
        Icon::Back => vec![line(&[(15.0, 5.0), (8.0, 12.0), (15.0, 19.0)])],
        Icon::Undo => vec![arc(12.0, 13.0, 6.5, 200.0, 450.0), fill(&[(3.5, 10.0), (9.5, 8.0), (6.5, 14.5)])],
        Icon::Redo => vec![arc(12.0, 13.0, 6.5, 90.0, 340.0), fill(&[(20.5, 10.0), (14.5, 8.0), (17.5, 14.5)])],
        Icon::Compare => vec![
            arc(12.0, 12.0, 8.0, 0.0, 360.0),
            Shape::convex_polygon((0..=16).map(|i| {
                let a = (90.0 + 180.0 * i as f32 / 16.0f32).to_radians();
                at(12.0 + 8.0 * a.cos(), 12.0 + 8.0 * a.sin())
            }).collect(), color, Stroke::NONE),
        ],
        Icon::RotateLeft => vec![
            Shape::rect_stroke(Rect::from_min_max(at(9.0, 10.0), at(19.0, 19.0)), 1.0, st, egui::StrokeKind::Middle),
            arc(9.0, 9.0, 5.0, 180.0, 300.0),
            fill(&[(2.0, 8.0), (8.0, 8.0), (4.0, 12.5)]),
        ],
        Icon::RotateRight => vec![
            Shape::rect_stroke(Rect::from_min_max(at(5.0, 10.0), at(15.0, 19.0)), 1.0, st, egui::StrokeKind::Middle),
            arc(15.0, 9.0, 5.0, 240.0, 360.0),
            fill(&[(22.0, 8.0), (16.0, 8.0), (20.0, 12.5)]),
        ],
        Icon::FlipH => vec![line(&[(12.0, 3.0), (12.0, 21.0)]), fill(&[(10.0, 6.0), (10.0, 18.0), (3.0, 18.0)]), line(&[(14.0, 6.0), (14.0, 18.0), (21.0, 18.0), (14.0, 6.0)])],
        Icon::FlipV => vec![line(&[(3.0, 12.0), (21.0, 12.0)]), fill(&[(6.0, 10.0), (18.0, 10.0), (18.0, 3.0)]), line(&[(6.0, 14.0), (18.0, 14.0), (18.0, 21.0), (6.0, 14.0)])],
        Icon::Reset => vec![arc(12.0, 12.0, 7.0, 220.0, 500.0), fill(&[(4.0, 4.5), (9.5, 7.5), (4.0, 10.0)])],
        Icon::Plus => vec![line(&[(12.0, 4.0), (12.0, 20.0)]), line(&[(4.0, 12.0), (20.0, 12.0)])],
        Icon::Play => vec![fill(&[(7.0, 4.0), (20.0, 12.0), (7.0, 20.0)])],
        Icon::Pause => vec![
            Shape::rect_filled(Rect::from_min_max(at(6.0, 4.0), at(10.0, 20.0)), 1.0, color),
            Shape::rect_filled(Rect::from_min_max(at(14.0, 4.0), at(18.0, 20.0)), 1.0, color),
        ],
        Icon::Split => vec![line(&[(12.0, 2.0), (12.0, 22.0)]), line(&[(4.0, 7.0), (9.0, 12.0), (4.0, 17.0)]), line(&[(20.0, 7.0), (15.0, 12.0), (20.0, 17.0)])],
        Icon::Trash => vec![line(&[(4.0, 6.0), (20.0, 6.0)]), line(&[(9.0, 6.0), (9.0, 3.5), (15.0, 3.5), (15.0, 6.0)]), line(&[(6.0, 6.0), (7.0, 21.0), (17.0, 21.0), (18.0, 6.0)]), line(&[(10.0, 10.0), (10.0, 17.0)]), line(&[(14.0, 10.0), (14.0, 17.0)])],
        Icon::Speed => vec![arc(12.0, 14.0, 8.5, 180.0, 360.0), line(&[(12.0, 14.0), (17.0, 8.0)]), line(&[(3.5, 14.0), (3.5, 17.0)]), line(&[(20.5, 14.0), (20.5, 17.0)])],
        Icon::Volume => vec![fill(&[(3.0, 9.0), (8.0, 9.0), (13.0, 4.0), (13.0, 20.0), (8.0, 15.0), (3.0, 15.0)]), arc(13.0, 12.0, 4.0, -50.0, 50.0), arc(13.0, 12.0, 8.0, -50.0, 50.0)],
        Icon::Duplicate => vec![
            Shape::rect_stroke(Rect::from_min_max(at(8.0, 8.0), at(20.0, 20.0)), 2.0, st, egui::StrokeKind::Middle),
            line(&[(4.0, 16.0), (4.0, 4.0), (16.0, 4.0)]),
        ],
        Icon::Text => vec![line(&[(5.0, 7.0), (5.0, 4.0), (19.0, 4.0), (19.0, 7.0)]), line(&[(12.0, 4.0), (12.0, 20.0)]), line(&[(9.0, 20.0), (15.0, 20.0)])],
        Icon::Transition => vec![
            Shape::rect_stroke(Rect::from_min_max(at(2.0, 6.0), at(12.0, 18.0)), 1.0, st, egui::StrokeKind::Middle),
            Shape::rect_filled(Rect::from_min_max(at(12.0, 6.0), at(22.0, 18.0)), 1.0, color),
            line(&[(9.0, 3.0), (15.0, 21.0)]),
        ],
        Icon::Sparkle => vec![fill(&[(12.0, 2.0), (14.5, 9.5), (22.0, 12.0), (14.5, 14.5), (12.0, 22.0), (9.5, 14.5), (2.0, 12.0), (9.5, 9.5)])],
        Icon::ToStart => vec![line(&[(5.0, 4.0), (5.0, 20.0)]), fill(&[(19.0, 4.0), (19.0, 20.0), (7.0, 12.0)])],
        Icon::Check => vec![line(&[(4.0, 12.5), (9.5, 18.0), (20.0, 6.0)])],
        Icon::Close => vec![line(&[(5.0, 5.0), (19.0, 19.0)]), line(&[(19.0, 5.0), (5.0, 19.0)])],
        Icon::Music => vec![line(&[(9.0, 18.0), (9.0, 4.0), (20.0, 2.5), (20.0, 16.0)]), Shape::circle_filled(at(6.5, 18.0), 3.0 * k, color), Shape::circle_filled(at(17.5, 16.0), 3.0 * k, color)],
        Icon::Photo => vec![
            Shape::rect_stroke(Rect::from_min_max(at(3.0, 5.0), at(21.0, 19.0)), 2.0, st, egui::StrokeKind::Middle),
            fill(&[(5.0, 17.0), (10.0, 10.0), (14.0, 15.0)]),
            fill(&[(11.0, 17.0), (15.5, 11.5), (19.0, 17.0)]),
            Shape::circle_filled(at(16.0, 8.5), 1.6 * k, color),
        ],
        Icon::Film => vec![
            Shape::rect_stroke(Rect::from_min_max(at(4.0, 3.0), at(20.0, 21.0)), 1.5, st, egui::StrokeKind::Middle),
            line(&[(8.0, 3.0), (8.0, 21.0)]),
            line(&[(16.0, 3.0), (16.0, 21.0)]),
            line(&[(4.0, 12.0), (8.0, 12.0)]),
            line(&[(16.0, 12.0), (20.0, 12.0)]),
        ],
    };
    p.extend(shapes);
    let _ = vec2;
}

/// A borderless TOUCH-sized icon button.
pub fn button(ui: &mut egui::Ui, icon: Icon, enabled: bool) -> egui::Response {
    let size = vec2(crate::theme::TOUCH, crate::theme::TOUCH);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { egui::Sense::click() } else { egui::Sense::hover() });
    let col = if enabled { crate::theme::TEXT } else { crate::theme::TEXT_DIM.gamma_multiply(0.5) };
    if enabled && resp.is_pointer_button_down_on() {
        ui.painter().circle_filled(rect.center(), 20.0, crate::theme::RAISED);
    }
    paint(ui.painter(), icon, Rect::from_center_size(rect.center(), vec2(22.0, 22.0)), col);
    resp
}

/// An icon above a small caption (tool bars).
pub fn tool(ui: &mut egui::Ui, icon: Icon, caption: &str, on: bool, enabled: bool) -> egui::Response {
    let accent = crate::theme::accent(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(64.0, 56.0), if enabled { egui::Sense::click() } else { egui::Sense::hover() });
    let c = if !enabled { crate::theme::TEXT_DIM.gamma_multiply(0.5) } else if on { accent } else { crate::theme::TEXT };
    let p = ui.painter_at(rect);
    if enabled && resp.is_pointer_button_down_on() {
        p.rect_filled(rect.shrink(2.0), 10, crate::theme::RAISED);
    }
    paint(&p, icon, Rect::from_center_size(rect.center() - vec2(0.0, 9.0), vec2(22.0, 22.0)), c);
    p.text(rect.center() + vec2(0.0, 16.0), egui::Align2::CENTER_CENTER, caption, egui::FontId::proportional(11.5), if on && enabled { accent } else { c.gamma_multiply(0.85) });
    resp
}
