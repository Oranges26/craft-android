//! A pinch-zoom / pan image view. Zoom 1 fits the picture in the view; pinch (or Ctrl + wheel on
//! the desktop) zooms about the fingers, one finger pans a zoomed picture, a double tap toggles
//! between fit and 1:1-ish (3×).

use egui::{Color32, CornerRadius, Rect, Response, Sense, TextureId, Ui, Vec2, pos2, vec2};

#[derive(Clone, Copy, Debug)]
pub struct ZoomPan {
    pub zoom: f32,
    /// Offset of the picture's centre from the view's centre, in points.
    pub offset: Vec2,
}

impl Default for ZoomPan {
    fn default() -> Self {
        ZoomPan { zoom: 1.0, offset: Vec2::ZERO }
    }
}

pub struct ViewOut {
    pub response: Response,
    /// Where the picture is drawn (may extend past the view when zoomed).
    pub image_rect: Rect,
    /// The view itself.
    pub view_rect: Rect,
}

impl ZoomPan {
    pub fn reset(&mut self) {
        *self = ZoomPan::default();
    }

    /// The picture's rectangle for an image of `size` (any unit, only the aspect matters) in `view`.
    pub fn image_rect(&self, view: Rect, size: Vec2, margin: f32) -> Rect {
        let inner = view.shrink(margin);
        let s = (inner.width() / size.x.max(1.0)).min(inner.height() / size.y.max(1.0));
        let fit = size * s * self.zoom;
        Rect::from_center_size(view.center() + self.offset, fit)
    }

    /// Show `tex` (`size` = its aspect) filling the available space. `pan_one_finger`: whether a
    /// one-finger drag pans (turn off while a tool uses drags, e.g. the crop handles).
    pub fn show(&mut self, ui: &mut Ui, tex: Option<TextureId>, size: Vec2, margin: f32, pan_one_finger: bool) -> ViewOut {
        let (view, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        ui.painter().rect_filled(view, CornerRadius::ZERO, crate::theme::BG);

        let (zoom_delta, multi, wheel) = ui.input(|i| (i.zoom_delta(), i.multi_touch(), i.smooth_scroll_delta()));
        let hover = response.hover_pos().or(response.interact_pointer_pos());
        if let Some(mt) = multi.filter(|_| response.contains_pointer() || response.dragged()) {
            self.zoom_about(view, size, margin, mt.center_pos, mt.zoom_delta);
            self.offset += mt.translation_delta;
        } else if (zoom_delta - 1.0).abs() > 1e-4 && response.contains_pointer() {
            self.zoom_about(view, size, margin, hover.unwrap_or(view.center()), zoom_delta);
        } else if response.contains_pointer() && self.zoom > 1.0 && wheel != Vec2::ZERO && !crate::platform::is_android() {
            self.offset += wheel;
        }
        if pan_one_finger && response.dragged() && multi.is_none() && self.zoom > 1.0 {
            self.offset += response.drag_delta();
        }
        if response.double_clicked() {
            if self.zoom > 1.01 {
                self.reset();
            } else if let Some(p) = response.interact_pointer_pos() {
                self.zoom_about(view, size, margin, p, 3.0);
            }
        }
        self.clamp(view, size, margin);

        let image_rect = self.image_rect(view, size, margin);
        if let Some(t) = tex {
            ui.painter_at(view).image(t, image_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        ViewOut { response, image_rect, view_rect: view }
    }

    fn zoom_about(&mut self, view: Rect, size: Vec2, margin: f32, at: egui::Pos2, factor: f32) {
        let old = self.zoom;
        let new = (old * factor).clamp(1.0, 12.0);
        let k = new / old;
        // keep the point under the fingers still: c' = at + (c - at)·k
        let c = view.center() + self.offset;
        let c2 = at + (c - at) * k;
        self.zoom = new;
        self.offset = c2 - view.center();
        let _ = (size, margin);
    }

    fn clamp(&mut self, view: Rect, size: Vec2, margin: f32) {
        if self.zoom <= 1.0 {
            self.zoom = 1.0;
            self.offset = Vec2::ZERO;
            return;
        }
        let r = self.image_rect(view, size, margin);
        let max = vec2(((r.width() - view.width()) / 2.0).max(0.0), ((r.height() - view.height()) / 2.0).max(0.0));
        self.offset = self.offset.clamp(-max, max);
    }

    /// Screen point → normalized picture coordinates (0…1).
    pub fn to_image(image_rect: Rect, p: egui::Pos2) -> egui::Pos2 {
        pos2((p.x - image_rect.left()) / image_rect.width(), (p.y - image_rect.top()) / image_rect.height())
    }

    /// Normalized picture coordinates → screen point.
    pub fn to_screen(image_rect: Rect, p: egui::Pos2) -> egui::Pos2 {
        pos2(image_rect.left() + p.x * image_rect.width(), image_rect.top() + p.y * image_rect.height())
    }
}
