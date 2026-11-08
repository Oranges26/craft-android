//! The crop tool on a touch screen: the whole photo is shown uncropped and unrotated, the crop is
//! a (possibly tilted) frame over it with corner and edge handles. Dragging inside moves the frame;
//! handles resize it (keeping a locked aspect). Every change goes through upstream's
//! `CropGeometry::constrained`, so the frame always stays on the photo.

use egui::{Color32, Pos2, Rect, Stroke, Vec2, pos2};
use lightcraft_develop::DevelopSettings;
use lightcraft_geom::{Affine, Point};
use lightcraft_geom::{CropGeometry, crop_fit_angle};

/// Which part of the frame a drag holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grip {
    Move,
    /// Corner: which of x0/x1 and y0/y1 move.
    Corner { right: bool, bottom: bool },
    EdgeX { right: bool },
    EdgeY { bottom: bool },
}

pub const ASPECTS: [(&str, Option<(u32, u32)>); 9] = [
    ("Free", None),
    ("Original", Some((0, 0))),
    ("1:1", Some((1, 1))),
    ("4:5", Some((4, 5))),
    ("5:4", Some((5, 4))),
    ("3:2", Some((3, 2))),
    ("2:3", Some((2, 3))),
    ("16:9", Some((16, 9))),
    ("9:16", Some((9, 16))),
];

/// Display (normalized 0…1 over the uncropped photo, as drawn) ↔ rotated-frame pixels.
pub struct Mapping {
    pub w: f64,
    pub h: f64,
    pub angle: f64,
    pub flip_h: bool,
    pub flip_v: bool,
}

impl Mapping {
    pub fn of(s: &DevelopSettings, w: f64, h: f64) -> Mapping {
        Mapping { w, h, angle: s.crop.geometry.angle, flip_h: s.crop.flip_h, flip_v: s.crop.flip_v }
    }

    fn centre(&self) -> Point {
        Point::new(self.w / 2.0, self.h / 2.0)
    }

    /// A point of the rotated frame (pixels) → normalized display position.
    pub fn to_display(&self, q: Point) -> Pos2 {
        let p = Affine::rotate_about(-self.angle.to_radians(), self.centre()).apply(q);
        let (mut x, mut y) = (p.x / self.w, p.y / self.h);
        if self.flip_h {
            x = 1.0 - x;
        }
        if self.flip_v {
            y = 1.0 - y;
        }
        pos2(x as f32, y as f32)
    }

    /// Normalized display position → rotated frame (pixels).
    pub fn to_frame(&self, d: Pos2) -> Point {
        let (mut x, mut y) = (f64::from(d.x), f64::from(d.y));
        if self.flip_h {
            x = 1.0 - x;
        }
        if self.flip_v {
            y = 1.0 - y;
        }
        Affine::rotate_about(self.angle.to_radians(), self.centre()).apply(Point::new(x * self.w, y * self.h))
    }

    /// A display-space drag (normalized delta) → frame-space delta in pixels.
    pub fn delta_to_frame(&self, d: Vec2) -> (f64, f64) {
        let a = self.to_frame(pos2(0.5, 0.5));
        let b = self.to_frame(pos2(0.5 + d.x, 0.5 + d.y));
        (b.x - a.x, b.y - a.y)
    }
}

/// The aspect ratio (width / height, in pixels) a crop is locked to.
pub fn locked_ratio(s: &DevelopSettings, w: f64, h: f64) -> Option<f64> {
    match s.crop.aspect {
        Some((0, 0)) => Some(w / h),
        Some((a, b)) if a > 0 && b > 0 => Some(f64::from(a) / f64::from(b)),
        _ => None,
    }
}

/// Set the aspect lock and fit the largest crop of that aspect.
pub fn set_aspect(s: &mut DevelopSettings, aspect: Option<(u32, u32)>, w: f64, h: f64) {
    s.crop.aspect = aspect;
    let ratio = locked_ratio(s, w, h);
    if ratio.is_some() {
        s.crop.geometry = crop_fit_angle(w, h, s.crop.geometry.angle, ratio);
    }
}

pub fn set_angle(s: &mut DevelopSettings, angle: f64, w: f64, h: f64) {
    s.crop.geometry = s.crop.geometry.with_angle(angle.clamp(-45.0, 45.0), w, h);
}

/// Which grip a touch at `p` (screen) takes, given the frame's corners on screen.
pub fn hit(corners: &[Pos2; 4], p: Pos2, slop: f32) -> Option<Grip> {
    // corners: [x0y0, x1y0, x1y1, x0y1]
    let near = |a: Pos2| (a - p).length() < slop;
    for (i, c) in corners.iter().enumerate() {
        if near(*c) {
            return Some(match i {
                0 => Grip::Corner { right: false, bottom: false },
                1 => Grip::Corner { right: true, bottom: false },
                2 => Grip::Corner { right: true, bottom: true },
                _ => Grip::Corner { right: false, bottom: true },
            });
        }
    }
    let edges = [(0, 1, Grip::EdgeY { bottom: false }), (1, 2, Grip::EdgeX { right: true }), (2, 3, Grip::EdgeY { bottom: true }), (3, 0, Grip::EdgeX { right: false })];
    for (a, b, g) in edges {
        if seg_dist(corners[a], corners[b], p) < slop * 0.7 {
            return Some(g);
        }
    }
    if inside(corners, p) { Some(Grip::Move) } else { None }
}

fn seg_dist(a: Pos2, b: Pos2, p: Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    (a + ab * t - p).length()
}

fn inside(c: &[Pos2; 4], p: Pos2) -> bool {
    let mut sign = 0.0f32;
    for i in 0..4 {
        let (a, b) = (c[i], c[(i + 1) % 4]);
        let cross = (b - a).x * (p - a).y - (b - a).y * (p - a).x;
        if cross.abs() < 1e-6 {
            continue;
        }
        if sign == 0.0 {
            sign = cross.signum();
        } else if cross.signum() != sign {
            return false;
        }
    }
    true
}

/// Apply a drag of `delta` (normalized display units) with `grip` to the crop.
pub fn drag(s: &mut DevelopSettings, grip: Grip, delta: Vec2, w: f64, h: f64) {
    let m = Mapping::of(s, w, h);
    let (dx, dy) = m.delta_to_frame(delta);
    let g = s.crop.geometry;
    let r = g.rect_px(w, h);
    let (mut x0, mut y0, mut x1, mut y1) = (r.x0, r.y0, r.x1, r.y1);
    let min = 0.05 * w.min(h);
    match grip {
        Grip::Move => {
            x0 += dx;
            x1 += dx;
            y0 += dy;
            y1 += dy;
        }
        Grip::Corner { right, bottom } => {
            if right { x1 += dx } else { x0 += dx }
            if bottom { y1 += dy } else { y0 += dy }
        }
        Grip::EdgeX { right } => {
            if right { x1 += dx } else { x0 += dx }
        }
        Grip::EdgeY { bottom } => {
            if bottom { y1 += dy } else { y0 += dy }
        }
    }
    if x1 - x0 < min {
        if matches!(grip, Grip::Corner { right: true, .. } | Grip::EdgeX { right: true }) { x1 = x0 + min } else { x0 = x1 - min }
    }
    if y1 - y0 < min {
        if matches!(grip, Grip::Corner { bottom: true, .. } | Grip::EdgeY { bottom: true }) { y1 = y0 + min } else { y0 = y1 - min }
    }
    if let Some(ratio) = locked_ratio(s, w, h).filter(|_| grip != Grip::Move) {
        // keep the aspect: the edge that moved more decides, the other follows (anchored at the
        // opposite corner / the centre of the opposite edge)
        let (cw, ch) = (x1 - x0, y1 - y0);
        let (nw, nh) = match grip {
            Grip::EdgeY { .. } => (ch * ratio, ch),
            Grip::EdgeX { .. } => (cw, cw / ratio),
            _ if (cw / ratio) > ch => (cw, cw / ratio),
            _ => (ch * ratio, ch),
        };
        match grip {
            Grip::Corner { right, bottom } => {
                if right { x1 = x0 + nw } else { x0 = x1 - nw }
                if bottom { y1 = y0 + nh } else { y0 = y1 - nh }
            }
            Grip::EdgeX { .. } => {
                let cy = (y0 + y1) / 2.0;
                (y0, y1) = (cy - nh / 2.0, cy + nh / 2.0);
            }
            Grip::EdgeY { .. } => {
                let cx = (x0 + x1) / 2.0;
                (x0, x1) = (cx - nw / 2.0, cx + nw / 2.0);
            }
            Grip::Move => {}
        }
    }
    let cand = CropGeometry { rect: lightcraft_geom::Rect::new(x0 / w, y0 / h, x1 / w, y1 / h), angle: g.angle };
    if cand.is_within_image(w, h) {
        s.crop.geometry = cand;
    } else if grip == Grip::Move {
        s.crop.geometry = cand.constrained(w, h);
    }
    // a resize past the photo's edge is refused (the frame stops at the edge)
}

/// The frame's corners on screen.
pub fn corners(s: &DevelopSettings, w: f64, h: f64, image_rect: Rect) -> [Pos2; 4] {
    let m = Mapping::of(s, w, h);
    let r = s.crop.geometry.rect_px(w, h);
    let q = [Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)];
    q.map(|q| craft_mobile_kit::viewer::ZoomPan::to_screen(image_rect, m.to_display(q)))
}

/// Draw the frame: darkened outside, rule-of-thirds grid, corner handles.
pub fn paint(p: &egui::Painter, c: &[Pos2; 4], view: Rect, active: bool) {
    // darken outside: a mesh between the view and the frame
    let shade = Color32::from_black_alpha(150);
    let outer = [view.left_top(), view.right_top(), view.right_bottom(), view.left_bottom()];
    let mut mesh = egui::Mesh::default();
    for v in outer.iter().chain(c.iter()) {
        mesh.colored_vertex(*v, shade);
    }
    for i in 0..4u32 {
        let j = (i + 1) % 4;
        mesh.add_triangle(i, j, 4 + i);
        mesh.add_triangle(j, 4 + j, 4 + i);
    }
    p.add(mesh);
    let white = Stroke::new(1.5, Color32::WHITE);
    for i in 0..4 {
        p.line_segment([c[i], c[(i + 1) % 4]], white);
    }
    let grid = Stroke::new(1.0, Color32::from_white_alpha(if active { 140 } else { 70 }));
    for t in [1.0 / 3.0, 2.0 / 3.0] {
        p.line_segment([c[0].lerp(c[1], t), c[3].lerp(c[2], t)], grid);
        p.line_segment([c[0].lerp(c[3], t), c[1].lerp(c[2], t)], grid);
    }
    let thick = Stroke::new(4.0, Color32::WHITE);
    for i in 0..4 {
        let (prev, next) = (c[(i + 3) % 4], c[(i + 1) % 4]);
        let len = 18.0;
        let a = c[i] + (prev - c[i]).normalized() * len;
        let b = c[i] + (next - c[i]).normalized() * len;
        p.line_segment([a, c[i]], thick);
        p.line_segment([c[i], b], thick);
    }
}
