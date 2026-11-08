//! The touch timeline: the playhead stays in the middle and the tracks slide under it. Drag empty
//! space to scrub, pinch to zoom, tap a clip to select it, drag a selected clip to move it (onto
//! another track of its kind too), drag its edge handles to trim.

use egui::{Align2, Color32, CornerRadius, FontId, Rect, Sense, Stroke, Ui, pos2, vec2};
use filmcraft_project::{ClipId, Sequence, Track, TrackId};
use filmcraft_time::Tick;

use craft_mobile_kit::theme::{self, TEXT, TEXT_DIM};

const RULER: f32 = 24.0;
const VIDEO_H: f32 = 50.0;
const AUDIO_H: f32 = 38.0;
const HANDLE: f32 = 16.0;

#[derive(Clone, Copy, Debug)]
enum Drag {
    Scrub,
    Trim { clip: ClipId, edge_in: bool, dx: f32 },
    Move { clip: ClipId, dx: f32, y: f32 },
}

#[derive(Clone, Debug)]
pub enum Action {
    /// Move the playhead (scrubbing).
    Seek(Tick),
    Select(Option<ClipId>),
    /// Trim a clip's edge by `delta` (ripple: the clips after it follow).
    Trim { clip: ClipId, edge_in: bool, delta: Tick },
    Move { clip: ClipId, track: TrackId, time: Tick },
}

pub struct Timeline {
    /// Points per second.
    pub pps: f32,
    drag: Option<Drag>,
    /// The playhead when a scrub began (so it follows the finger exactly).
    scrub_from: f64,
}

impl Default for Timeline {
    fn default() -> Self {
        Timeline { pps: 60.0, drag: None, scrub_from: 0.0 }
    }
}

struct Row<'a> {
    track: &'a Track,
    video: bool,
    rect: Rect,
}

impl Timeline {
    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    pub fn show(&mut self, ui: &mut Ui, seq: &Sequence, playhead: Tick, selection: &[ClipId]) -> Vec<Action> {
        let accent = theme::accent(ui.ctx());
        let mut out = Vec::new();
        let (area, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        let p = ui.painter_at(area);
        p.rect_filled(area, CornerRadius::ZERO, theme::BG);
        let cx = area.center().x;
        let ph = playhead.seconds();
        let x_of = |t: f64, pps: f32| cx + ((t - ph) as f32) * pps;

        // rows: video tracks top-down from the highest, then audio
        let mut rows: Vec<Row> = Vec::new();
        let mut y = area.top() + RULER + 4.0;
        for t in seq.video_tracks.iter().rev() {
            rows.push(Row { track: t, video: true, rect: Rect::from_min_size(pos2(area.left(), y), vec2(area.width(), VIDEO_H)) });
            y += VIDEO_H + 4.0;
        }
        y += 4.0;
        for t in &seq.audio_tracks {
            rows.push(Row { track: t, video: false, rect: Rect::from_min_size(pos2(area.left(), y), vec2(area.width(), AUDIO_H)) });
            y += AUDIO_H + 4.0;
        }

        // pinch zoom
        if let Some(mt) = ui.input(|i| i.multi_touch()).filter(|_| resp.contains_pointer() || resp.dragged()) {
            self.pps = (self.pps * mt.zoom_delta_2d.x.max(0.2)).clamp(8.0, 900.0);
        } else if resp.contains_pointer() {
            let z = ui.input(|i| i.zoom_delta());
            if (z - 1.0).abs() > 1e-4 {
                self.pps = (self.pps * z).clamp(8.0, 900.0);
            }
        }
        let pps = self.pps;
        let multi = ui.input(|i| i.multi_touch()).is_some();

        let clip_rect = |row: &Row, start: Tick, dur: Tick| {
            let x0 = x_of(start.seconds(), pps);
            let x1 = x_of((start + dur).seconds(), pps);
            Rect::from_min_max(pos2(x0, row.rect.top()), pos2(x1.max(x0 + 2.0), row.rect.bottom()))
        };

        // gestures
        if resp.drag_started()
            && !multi
            && let Some(pos) = resp.interact_pointer_pos()
        {
            self.drag = Some(Drag::Scrub);
            self.scrub_from = ph;
            'find: for row in &rows {
                for it in &row.track.items {
                    if !selection.contains(&it.id) {
                        continue;
                    }
                    let r = clip_rect(row, it.start, it.duration);
                    if !r.expand2(vec2(HANDLE, 6.0)).contains(pos) {
                        continue;
                    }
                    if (pos.x - r.left()).abs() < HANDLE * 1.2 {
                        self.drag = Some(Drag::Trim { clip: it.id, edge_in: true, dx: 0.0 });
                    } else if (pos.x - r.right()).abs() < HANDLE * 1.2 {
                        self.drag = Some(Drag::Trim { clip: it.id, edge_in: false, dx: 0.0 });
                    } else if r.contains(pos) {
                        self.drag = Some(Drag::Move { clip: it.id, dx: 0.0, y: pos.y });
                    }
                    break 'find;
                }
            }
        }
        if resp.dragged() && !multi {
            let d = resp.drag_delta();
            match &mut self.drag {
                Some(Drag::Scrub) => {
                    self.scrub_from -= f64::from(d.x / pps);
                    out.push(Action::Seek(Tick::from_seconds_f64(self.scrub_from.max(0.0))));
                }
                Some(Drag::Trim { dx, .. }) => *dx += d.x,
                Some(Drag::Move { dx, y, .. }) => {
                    *dx += d.x;
                    *y += d.y;
                }
                None => {}
            }
        }
        if resp.drag_stopped() {
            match self.drag.take() {
                Some(Drag::Trim { clip, edge_in, dx }) if dx.abs() > 1.0 => {
                    out.push(Action::Trim { clip, edge_in, delta: Tick::from_seconds_f64(f64::from(dx / pps)) });
                }
                Some(Drag::Move { clip, dx, y }) => {
                    if let Some((row, it)) = find(&rows, clip) {
                        let target = rows.iter().filter(|r| r.video == row.video).min_by(|a, b| {
                            (a.rect.center().y - y).abs().total_cmp(&(b.rect.center().y - y).abs())
                        });
                        let track = target.map(|r| r.track.id).unwrap_or(row.track.id);
                        let time = Tick::from_seconds_f64((it.start.seconds() + f64::from(dx / pps)).max(0.0));
                        if dx.abs() > 2.0 || track != row.track.id {
                            out.push(Action::Move { clip, track, time });
                        }
                    }
                }
                _ => {}
            }
        }
        if resp.clicked()
            && let Some(pos) = resp.interact_pointer_pos()
        {
            let mut hit = None;
            for row in &rows {
                for it in &row.track.items {
                    if clip_rect(row, it.start, it.duration).contains(pos) {
                        hit = Some(it.id);
                    }
                }
            }
            out.push(Action::Select(hit));
        }

        // ruler
        let ruler = Rect::from_min_size(area.min, vec2(area.width(), RULER));
        p.rect_filled(ruler, CornerRadius::ZERO, theme::PANEL);
        let step = [0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0].into_iter().find(|s| *s as f32 * pps >= 70.0).unwrap_or(600.0);
        let t0 = ((ph - f64::from(area.width() / 2.0 / pps)) / step).floor() * step;
        let mut t = t0.max(0.0);
        while x_of(t, pps) < area.right() {
            let x = x_of(t, pps);
            p.line_segment([pos2(x, ruler.bottom() - 7.0), pos2(x, ruler.bottom())], Stroke::new(1.0, TEXT_DIM));
            p.text(pos2(x + 3.0, ruler.top() + 3.0), Align2::LEFT_TOP, clock(t), FontId::proportional(10.5), TEXT_DIM);
            for k in 1..4 {
                let xm = x_of(t + step * f64::from(k) / 4.0, pps);
                p.line_segment([pos2(xm, ruler.bottom() - 3.0), pos2(xm, ruler.bottom())], Stroke::new(1.0, TEXT_DIM.gamma_multiply(0.5)));
            }
            t += step;
        }

        // tracks and clips
        for row in &rows {
            p.rect_filled(row.rect, CornerRadius::ZERO, Color32::from_rgb(24, 24, 28));
            let base = if row.video { Color32::from_rgb(70, 110, 190) } else { Color32::from_rgb(60, 150, 100) };
            for it in &row.track.items {
                let sel = selection.contains(&it.id);
                let (mut start, mut dur) = (it.start, it.duration);
                let mut offset = vec2(0.0, 0.0);
                match self.drag {
                    Some(Drag::Trim { clip, edge_in, dx }) if clip == it.id => {
                        let d = Tick::from_seconds_f64(f64::from(dx / pps));
                        if edge_in {
                            start = start + d;
                            dur = Tick((dur.0 - d.0).max(1));
                        } else {
                            dur = Tick((dur.0 + d.0).max(1));
                        }
                    }
                    Some(Drag::Move { clip, dx, y }) if clip == it.id => {
                        offset = vec2(dx, (y - row.rect.center().y) * 0.6);
                    }
                    _ => {}
                }
                let r = clip_rect(row, start, dur).translate(offset).shrink2(vec2(0.5, 1.0));
                if r.right() < area.left() || r.left() > area.right() {
                    continue;
                }
                let fill = if it.enabled { base } else { base.gamma_multiply(0.4) };
                p.rect_filled(r, CornerRadius::same(6), if sel { fill.gamma_multiply(1.25) } else { fill.gamma_multiply(0.85) });
                let text_rect = r.intersect(area).shrink2(vec2(6.0, 2.0));
                if text_rect.width() > 20.0 {
                    let label = if (it.speed - 1.0).abs() > 1e-3 { format!("{} · {:.2}×", it.name, it.speed) } else { it.name.clone() };
                    p.with_clip_rect(text_rect).text(text_rect.left_top() + vec2(0.0, 2.0), Align2::LEFT_TOP, label, FontId::proportional(12.0), TEXT);
                }
                if sel {
                    p.rect_stroke(r, CornerRadius::same(6), Stroke::new(2.0, Color32::WHITE), egui::StrokeKind::Inside);
                    for (x, left) in [(r.left(), true), (r.right(), false)] {
                        let h = Rect::from_min_max(pos2(if left { x } else { x - HANDLE }, r.top()), pos2(if left { x + HANDLE } else { x }, r.bottom()));
                        p.rect_filled(h, CornerRadius::same(5), Color32::WHITE);
                        let c = h.center();
                        p.line_segment([c - vec2(0.0, 6.0), c + vec2(0.0, 6.0)], Stroke::new(2.0, Color32::from_gray(90)));
                    }
                }
            }
            // transitions
            for tr in &row.track.transitions {
                {
                    let x0 = x_of(tr.start.seconds(), pps);
                    let x1 = x_of(tr.end().seconds(), pps);
                    let r = Rect::from_min_max(pos2(x0, row.rect.top() + row.rect.height() * 0.55), pos2(x1.max(x0 + 6.0), row.rect.bottom() - 2.0));
                    p.rect_filled(r, CornerRadius::same(3), Color32::from_white_alpha(170));
                }
            }
            // track name
            p.text(row.rect.left_top() + vec2(4.0, 2.0), Align2::LEFT_TOP, &row.track.name, FontId::proportional(9.5), TEXT_DIM.gamma_multiply(0.7));
        }

        // playhead
        p.line_segment([pos2(cx, area.top()), pos2(cx, area.bottom())], Stroke::new(2.0, Color32::WHITE));
        p.add(egui::Shape::convex_polygon(vec![pos2(cx - 7.0, area.top()), pos2(cx + 7.0, area.top()), pos2(cx, area.top() + 9.0)], accent, Stroke::NONE));
        out
    }
}

fn find<'a>(rows: &'a [Row<'a>], clip: ClipId) -> Option<(&'a Row<'a>, &'a filmcraft_project::TrackItem)> {
    for r in rows {
        if let Some(it) = r.track.items.iter().find(|i| i.id == clip) {
            return Some((r, it));
        }
    }
    None
}

/// `m:ss` (or `m:ss.f` when zoomed far in).
pub fn clock(t: f64) -> String {
    let t = t.max(0.0);
    let m = (t / 60.0).floor();
    let s = t - m * 60.0;
    if (s - s.round()).abs() > 1e-6 { format!("{m:.0}:{s:04.1}") } else { format!("{m:.0}:{:02.0}", s) }
}
