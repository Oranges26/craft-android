//! The touch interface: a home screen of recent photos and the editor (picture on top, tools
//! below in portrait; tools on the right in landscape).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::Receiver;

use craft_mobile_kit::theme::{self, TEXT_DIM};
use craft_mobile_kit::viewer::ZoomPan;
use craft_mobile_kit::widgets::{self, TrackPaint};
use craft_mobile_kit::icons::{self, Icon};
use craft_mobile_kit::{Event, Layout, PickKind, platform};
use egui::{Color32, RichText, Ui, vec2};
use lightcraft_develop::controls::{self, ControlSpec, Section, Track};
use lightcraft_develop::{DevelopSettings, Preset, Treatment};

use crate::crop::{self, Grip};
use crate::doc::{self, Doc, Loaded};
use crate::export;
use crate::library::{self, Library};
use crate::render::{self, Job, Purpose, Renderer};

const ACCENT: Color32 = Color32::from_rgb(64, 156, 255);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tool {
    Looks,
    Crop,
    Section(Section),
}

impl Tool {
    fn label(self) -> &'static str {
        match self {
            Tool::Looks => "Looks",
            Tool::Crop => "Crop",
            Tool::Section(Section::Mixer) => "Mixer",
            Tool::Section(Section::BwMix) => "B&W Mix",
            Tool::Section(Section::Grading) => "Grading",
            Tool::Section(Section::Curve) => "Curve",
            Tool::Section(s) => s.label(),
        }
    }
}

/// Sections in the order a phone user reaches for them.
const SECTION_ORDER: [Section; 13] = [
    Section::Light,
    Section::Color,
    Section::Effects,
    Section::Detail,
    Section::Grading,
    Section::Mixer,
    Section::BwMix,
    Section::Vignette,
    Section::Grain,
    Section::Curve,
    Section::Optics,
    Section::Geometry,
    Section::Calibration,
];

struct Shown {
    tex: egui::TextureHandle,
    generation: u64,
    draft: bool,
    uncropped: bool,
    size: [usize; 2],
}

#[derive(Default)]
struct LookState {
    /// The settings the look is applied on (taken when the first look is tapped).
    base: Option<DevelopSettings>,
    selected: Option<usize>,
    amount: f64,
}

struct ExportUi {
    opts: export::Options,
    rx: Option<Receiver<export::Progress>>,
    step: String,
    fraction: f32,
    result: Option<String>,
}

pub struct MobileApp {
    library: Library,
    thumbs: HashMap<String, Option<egui::TextureHandle>>,
    loading: Option<(String, Receiver<Result<Loaded, String>>)>,
    doc: Option<Doc>,
    renderer: Option<Renderer>,
    shown: Option<Shown>,
    before: Option<(egui::TextureHandle, u64)>,
    last_full: Option<Arc<lightcraft_raster::Rgba8>>,
    histogram: Option<lightcraft_raster::Histogram>,
    view: ZoomPan,
    view_px: (usize, usize),
    tool: Tool,
    panel_open: bool,
    sub_group: HashMap<Section, String>,
    crop_grip: Option<Grip>,
    looks: Vec<Preset>,
    look: LookState,
    export: Option<ExportUi>,
    confirm_remove: Option<String>,
    message: Option<(String, f64)>,
    pending_open: Option<PathBuf>,
}

impl MobileApp {
    pub fn new(cc: &eframe::CreationContext<'_>, open: Option<PathBuf>) -> MobileApp {
        theme::apply(&cc.egui_ctx, ACCENT);
        platform::set_context(&cc.egui_ctx);
        let mut app = MobileApp::blank(open);
        // desktop test hook: LC_TEST_TOOL=crop|looks|<section label>, LC_TEST_EXPORT=1
        if !platform::is_android()
            && let Ok(t) = std::env::var("LC_TEST_TOOL")
        {
            let t = t.to_lowercase();
            app.tool = match t.as_str() {
                "crop" => Tool::Crop,
                "looks" => Tool::Looks,
                _ => SECTION_ORDER.iter().find(|s| s.label().to_lowercase().contains(&t)).map(|s| Tool::Section(*s)).unwrap_or(app.tool),
            };
        }
        app
    }

    fn blank(open: Option<PathBuf>) -> MobileApp {
        MobileApp {
            library: Library::load(),
            thumbs: HashMap::new(),
            loading: None,
            doc: None,
            renderer: None,
            shown: None,
            before: None,
            last_full: None,
            histogram: None,
            view: ZoomPan::default(),
            view_px: (1080, 1080),
            tool: Tool::Section(Section::Light),
            panel_open: true,
            sub_group: HashMap::new(),
            crop_grip: None,
            looks: lightcraft_engine::presets::builtin(),
            look: LookState { amount: 1.0, ..Default::default() },
            export: None,
            confirm_remove: None,
            message: None,
            pending_open: open,
        }
    }

    fn say(&mut self, ctx: &egui::Context, msg: impl Into<String>) {
        self.message = Some((msg.into(), ctx.input(|i| i.time) + 3.0));
    }

    // ---------------------------------------------------------------------------------------
    // opening and closing photos

    fn open_picked(&mut self, ctx: &egui::Context, path: PathBuf, name: String) {
        match self.library.add(&path, &name) {
            Ok((file, _key)) => self.open_file(ctx, file, name),
            Err(e) => self.say(ctx, e),
        }
    }

    fn open_file(&mut self, ctx: &egui::Context, file: PathBuf, name: String) {
        self.close_doc();
        let (tx, rx) = std::sync::mpsc::channel();
        let c = ctx.clone();
        let n = name.clone();
        std::thread::Builder::new()
            .name("lc-open".into())
            .spawn(move || {
                let _ = tx.send(doc::load(&file, &n));
                c.request_repaint();
            })
            .expect("open thread");
        self.loading = Some((name, rx));
    }

    fn poll_loading(&mut self, ctx: &egui::Context) {
        let Some((_, rx)) = &self.loading else { return };
        let Ok(r) = rx.try_recv() else { return };
        self.loading = None;
        match r {
            Ok(photo) => {
                self.library.touch(&photo.key);
                let saved = library::load_edits(&photo.key);
                self.renderer = Some(Renderer::new(photo.src.clone(), photo.info.clone(), ctx.clone()));
                let mut doc = Doc::new(photo, saved);
                // desktop test hook: LC_TEST_SET='{"light":{"exposure":0.5}}' (partial settings), LC_TEST_ANGLE=8
                if !platform::is_android() {
                    if let Some(v) = std::env::var("LC_TEST_SET").ok().and_then(|j| serde_json::from_str::<serde_json::Value>(&j).ok())
                        && let Ok(s) = doc.settings.merged(&v)
                    {
                        doc.set(s);
                    }
                    if let Some(a) = std::env::var("LC_TEST_ANGLE").ok().and_then(|a| a.parse::<f64>().ok()) {
                        let (w, h) = doc.oriented_size();
                        doc.edit(|s| crop::set_aspect(s, Some((4, 5)), w, h));
                        doc.edit(|s| crop::set_angle(s, a, w, h));
                    }
                }
                self.doc = Some(doc);
                self.view.reset();
                self.look = LookState { amount: 1.0, ..Default::default() };
                if self.tool == Tool::Crop && std::env::var_os("LC_TEST_TOOL").is_none() {
                    self.tool = Tool::Section(Section::Light);
                }
                if !platform::is_android() && std::env::var_os("LC_TEST_EXPORT").is_some() {
                    self.export = Some(ExportUi { opts: Default::default(), rx: None, step: String::new(), fraction: 0.0, result: None });
                }
            }
            Err(e) => self.say(ctx, e),
        }
    }

    fn close_doc(&mut self) {
        if let Some(d) = &self.doc {
            library::save_edits(&d.photo.key, &d.settings);
            if let Some(img) = &self.last_full {
                library::save_thumb(&d.photo.key, img);
                self.thumbs.remove(&d.photo.key);
            }
        }
        self.doc = None;
        self.renderer = None;
        self.shown = None;
        self.before = None;
        self.last_full = None;
        self.histogram = None;
        self.crop_grip = None;
        self.export = None;
    }

    // ---------------------------------------------------------------------------------------
    // rendering

    fn pump_renders(&mut self, ctx: &egui::Context) {
        let (Some(doc), Some(r)) = (&self.doc, &mut self.renderer) else { return };
        while let Ok(done) = r.rx.try_recv() {
            let img = render::to_color_image(&done.image);
            match done.purpose {
                Purpose::View => {
                    let size = [done.image.width, done.image.height];
                    let opts = egui::TextureOptions::LINEAR;
                    match &mut self.shown {
                        Some(s) => s.tex.set(img, opts),
                        None => {
                            self.shown =
                                Some(Shown { tex: ctx.load_texture("photo", img, opts), generation: 0, draft: true, uncropped: false, size });
                        }
                    }
                    if let Some(s) = &mut self.shown {
                        s.generation = done.generation;
                        s.draft = done.draft;
                        s.uncropped = done.uncropped;
                        s.size = size;
                    }
                    if !done.draft && !done.uncropped {
                        self.last_full = Some(Arc::new(done.image));
                    }
                    self.histogram = Some(done.histogram);
                    log::debug!("render {:.0} ms", done.millis);
                }
                Purpose::Before => {
                    self.before = Some((ctx.load_texture("before", img, egui::TextureOptions::LINEAR), done.generation));
                }
            }
        }
        let uncropped = self.tool == Tool::Crop;
        let draft = doc.in_gesture();
        let fresh = self.shown.as_ref().is_some_and(|s| s.generation == doc.generation && s.uncropped == uncropped && (!s.draft || draft));
        let edge = doc::PREVIEW_EDGE;
        let max = if self.view.zoom > 1.0 { (edge, edge) } else { (self.view_px.0.min(edge), self.view_px.1.min(edge)) };
        if !fresh {
            r.request(Job { generation: doc.generation, purpose: Purpose::View, settings: Arc::new(doc.settings.clone()), max, draft, uncropped });
        }
    }

    fn request_before(&mut self) {
        let (Some(doc), Some(r)) = (&self.doc, &mut self.renderer) else { return };
        if self.before.is_none() {
            let edge = doc::PREVIEW_EDGE;
            let max = (self.view_px.0.min(edge), self.view_px.1.min(edge));
            r.request(Job { generation: 0, purpose: Purpose::Before, settings: Arc::new(doc.original.clone()), max, draft: false, uncropped: false });
        }
    }

    // ---------------------------------------------------------------------------------------
    // platform events

    fn handle_events(&mut self, ctx: &egui::Context) {
        for e in platform::poll_events() {
            match e {
                Event::Picked { path, name } => self.open_picked(ctx, path, name),
                Event::PickCancelled | Event::Text { .. } => {}
                Event::Saved { ok: _, message } => {
                    if let Some(x) = &mut self.export {
                        x.result = Some(message.clone());
                    }
                    self.say(ctx, message);
                }
                Event::Back => {
                    if self.export.is_some() {
                        if self.export.as_ref().is_some_and(|x| x.rx.is_none()) {
                            self.export = None;
                        }
                    } else if self.confirm_remove.is_some() {
                        self.confirm_remove = None;
                    } else if self.doc.is_some() || self.loading.is_some() {
                        if self.tool == Tool::Crop {
                            self.tool = Tool::Section(Section::Light);
                        } else {
                            self.loading = None;
                            self.close_doc();
                        }
                    } else {
                        platform::move_to_background();
                    }
                }
            }
        }
        if let Some(p) = self.pending_open.take() {
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if p.exists() {
                self.open_picked(ctx, p, name);
            }
        }
    }

    fn autosave(&mut self) {
        if let Some(d) = &mut self.doc
            && d.generation != d.saved_generation
            && !d.in_gesture()
        {
            library::save_edits(&d.photo.key, &d.settings);
            d.saved_generation = d.generation;
        }
    }

    // ---------------------------------------------------------------------------------------
    // screens

    fn home(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.heading(RichText::new("LightCraft").size(26.0).strong());
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(RichText::new("Non-destructive photo editing").color(TEXT_DIM));
        });
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.set_width(ui.available_width() - 8.0);
                if widgets::primary_button(ui, "+  Open a photo").clicked() {
                    platform::pick_file(PickKind::Image);
                }
            });
        });
        ui.add_space(20.0);
        if self.library.entries.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.label(RichText::new("Photos you edit appear here.\nYour edits are kept, so you can come back to them.").color(TEXT_DIM));
            });
            return;
        }
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(RichText::new("Recent").strong());
            ui.label(RichText::new("· long-press to remove").small().color(TEXT_DIM));
        });
        let cols = ((ui.available_width() / 120.0).floor() as usize).clamp(2, 8);
        let cell = (ui.available_width() - 16.0 - (cols as f32 - 1.0) * 6.0) / cols as f32;
        let entries = self.library.entries.clone();
        let mut open = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for row in entries.chunks(cols) {
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ui.spacing_mut().item_spacing.x = 6.0;
                    for e in row {
                        let tex = self.thumbs.entry(e.key.clone()).or_insert_with(|| library::load_thumb(&e.key).map(|img| ctx.load_texture(&e.key, img, Default::default()))).clone();
                        let (rect, resp) = ui.allocate_exact_size(vec2(cell, cell), egui::Sense::click());
                        ui.painter().rect_filled(rect, 8, theme::RAISED);
                        match tex {
                            Some(t) => {
                                let s = t.size_vec2();
                                let k = (cell / s.x).max(cell / s.y);
                                let shown = s * k;
                                let uv = egui::Rect::from_center_size(egui::pos2(0.5, 0.5), vec2(cell / shown.x, cell / shown.y));
                                ui.painter().image(t.id(), rect, uv, Color32::WHITE);
                            }
                            None => {
                                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, &e.name, egui::FontId::proportional(11.0), TEXT_DIM);
                            }
                        }
                        if resp.clicked() {
                            open = Some(e.clone());
                        }
                        if resp.long_touched() || resp.secondary_clicked() {
                            self.confirm_remove = Some(e.key.clone());
                        }
                    }
                });
            }
            ui.add_space(24.0);
        });
        if let Some(e) = open {
            self.open_file(&ctx, e.file, e.name);
        }
    }

    fn editor(&mut self, ui: &mut Ui) {
        let full = ui.available_rect_before_wrap();
        let layout = Layout::of(full);
        self.top_bar(ui);
        match layout {
            Layout::Portrait => {
                let panel_h = if self.panel_open { (full.height() * 0.40).clamp(220.0, 420.0) } else { 0.0 };
                egui::Panel::bottom("tools").exact_size(panel_h + 112.0).frame(egui::Frame::new().fill(theme::PANEL)).show(ui, |ui| {
                    self.tools(ui, panel_h);
                });
                egui::CentralPanel::default().frame(egui::Frame::new()).show(ui, |ui| self.picture(ui));
            }
            Layout::Landscape => {
                let w = (full.width() * 0.38).clamp(300.0, 420.0);
                egui::Panel::right("tools").exact_size(w).frame(egui::Frame::new().fill(theme::PANEL)).show(ui, |ui| {
                    let h = ui.available_height() - 112.0;
                    self.tools(ui, h.max(120.0));
                });
                egui::CentralPanel::default().frame(egui::Frame::new()).show(ui, |ui| self.picture(ui));
            }
        }
    }

    fn top_bar(&mut self, ui: &mut Ui) {
        egui::Panel::top("top").exact_size(52.0).frame(egui::Frame::new().fill(theme::PANEL).inner_margin(egui::Margin::symmetric(4, 4))).show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                if icons::button(ui, Icon::Back, true).clicked() {
                    self.loading = None;
                    self.close_doc();
                    return;
                }
                let (can_undo, can_redo) = self.doc.as_ref().map(|d| (d.can_undo(), d.can_redo())).unwrap_or_default();
                if icons::button(ui, Icon::Undo, can_undo).clicked()
                    && let Some(d) = &mut self.doc
                {
                    d.undo();
                    self.look.base = None;
                    self.look.selected = None;
                }
                if icons::button(ui, Icon::Redo, can_redo).clicked()
                    && let Some(d) = &mut self.doc
                {
                    d.redo();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let has = self.doc.is_some();
                    if ui.add_enabled(has, egui::Button::new(RichText::new("Export").strong()).min_size(vec2(84.0, 40.0))).clicked() {
                        self.export = Some(ExportUi { opts: Default::default(), rx: None, step: String::new(), fraction: 0.0, result: None });
                    }
                    let cmp = icons::button(ui, Icon::Compare, has);
                    if cmp.is_pointer_button_down_on() {
                        self.request_before();
                        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique("compare"), true));
                    } else {
                        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique("compare"), false));
                    }
                    if let Some(d) = &self.doc {
                        ui.add(egui::Label::new(RichText::new(&d.photo.name).small().color(TEXT_DIM)).truncate());
                    }
                });
            });
        });
    }

    fn picture(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        if self.doc.is_none() {
            let r = ui.available_rect_before_wrap();
            ui.painter().rect_filled(r, 0, theme::BG);
            let name = self.loading.as_ref().map(|l| l.0.clone()).unwrap_or_default();
            widgets::busy(ui, r, &format!("Opening {name}…"));
            return;
        }
        let compare = ctx.data(|d| d.get_temp::<bool>(egui::Id::unique("compare"))).unwrap_or(false) && self.tool != Tool::Crop;
        let crop_mode = self.tool == Tool::Crop;
        let (tex, size) = match (&self.shown, &self.before) {
            (_, Some((b, _))) if compare => (Some(b.id()), b.size_vec2()),
            (Some(s), _) => (Some(s.tex.id()), vec2(s.size[0] as f32, s.size[1] as f32)),
            _ => (None, vec2(3.0, 2.0)),
        };
        let margin = if crop_mode { 28.0 } else { 0.0 };
        let out = self.view.show(ui, tex, size, margin, !crop_mode);
        let ppp = ctx.pixels_per_point();
        self.view_px = (((out.view_rect.width() - 2.0 * margin) * ppp) as usize, ((out.view_rect.height() - 2.0 * margin) * ppp) as usize);
        if compare {
            ui.painter().text(out.view_rect.left_top() + vec2(12.0, 12.0), egui::Align2::LEFT_TOP, "Before", egui::FontId::proportional(14.0), Color32::WHITE);
        }
        if self.shown.is_none() {
            widgets::busy(ui, out.view_rect, "Developing…");
        }
        if crop_mode && self.shown.as_ref().is_some_and(|s| s.uncropped) {
            self.crop_overlay(ui, &out);
        }
    }

    fn crop_overlay(&mut self, ui: &mut Ui, out: &craft_mobile_kit::viewer::ViewOut) {
        let Some(doc) = &mut self.doc else { return };
        let (w, h) = doc.oriented_size();
        let corners = crop::corners(&doc.settings, w, h, out.image_rect);
        let r = &out.response;
        if r.drag_started()
            && let Some(p) = r.interact_pointer_pos()
        {
            self.crop_grip = crop::hit(&corners, p, 30.0);
            if self.crop_grip.is_some() {
                doc.begin();
            }
        }
        if r.dragged()
            && let Some(g) = self.crop_grip
        {
            let d = r.drag_delta();
            let nd = vec2(d.x / out.image_rect.width(), d.y / out.image_rect.height());
            doc.edit(|s| crop::drag(s, g, nd, w, h));
        }
        if r.drag_stopped() {
            self.crop_grip = None;
            doc.end();
        }
        let corners = crop::corners(&doc.settings, w, h, out.image_rect);
        crop::paint(&ui.painter_at(out.view_rect), &corners, out.view_rect, self.crop_grip.is_some());
    }

    fn tool_list(&self) -> Vec<Tool> {
        let bw = self.doc.as_ref().is_some_and(|d| d.settings.treatment == Treatment::Bw);
        let mut v = vec![Tool::Looks, Tool::Crop];
        for s in SECTION_ORDER {
            if controls::in_section(s).next().is_none() {
                continue;
            }
            // the colour mixer and the B&W mixer are one panel each way round
            if (s == Section::Mixer && bw) || (s == Section::BwMix && !bw) {
                continue;
            }
            v.push(Tool::Section(s));
        }
        v
    }

    fn tools(&mut self, ui: &mut Ui, panel_h: f32) {
        let tools = self.tool_list();
        if !tools.contains(&self.tool) {
            self.tool = Tool::Section(Section::Light);
        }
        let labels: Vec<&str> = tools.iter().map(|t| t.label()).collect();
        let sel = tools.iter().position(|t| *t == self.tool);
        ui.add_space(6.0);
        if panel_h > 0.0 {
            let (_, dy) = widgets::sheet_handle(ui);
            if dy > 18.0 {
                self.panel_open = false;
            }
        } else {
            let (resp, dy) = widgets::sheet_handle(ui);
            if dy < -18.0 || resp.clicked() {
                self.panel_open = true;
            }
        }
        if let Some(i) = widgets::chip_row(ui, "tool-chips", &labels, sel) {
            if Some(i) == sel && self.tool != Tool::Crop {
                self.panel_open = !self.panel_open;
            } else {
                self.tool = tools[i];
                self.panel_open = true;
            }
        }
        ui.add_space(4.0);
        if panel_h <= 0.0 {
            return;
        }
        ui.separator();
        let tool = self.tool;
        egui::ScrollArea::vertical().id_salt(("panel", tool.label())).max_height(panel_h).scroll_source(egui::scroll_area::ScrollSource::ALL).show(ui, |ui| {
            ui.set_width(ui.available_width());
            match tool {
                Tool::Looks => self.looks_panel(ui),
                Tool::Crop => self.crop_panel(ui),
                Tool::Section(s) => self.section_panel(ui, s),
            }
            ui.add_space(16.0);
        });
    }

    fn section_panel(&mut self, ui: &mut Ui, section: Section) {
        let Some(doc) = &mut self.doc else { return };
        let raw = doc.photo.info.raw;
        // extras on top of upstream's sliders
        if section == Section::Light {
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                if ui.add(egui::Button::new("Auto").min_size(vec2(96.0, 40.0))).clicked() {
                    let a = lightcraft_pipeline::auto::auto_tone(&doc.photo.src, &doc.photo.info, &doc.settings);
                    doc.edit(|s| {
                        s.light.exposure = a.exposure;
                        s.light.contrast = a.contrast;
                        s.light.highlights = a.highlights;
                        s.light.shadows = a.shadows;
                        s.light.whites = a.whites;
                        s.light.blacks = a.blacks;
                        s.color.vibrance = a.vibrance;
                        s.color.saturation = a.saturation;
                    });
                }
                if ui.add(egui::Button::new("Reset light").min_size(vec2(0.0, 40.0))).clicked() {
                    let o = doc.original.light.clone();
                    doc.edit(|s| s.light = o);
                }
                if let Some(h) = &self.histogram {
                    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width() - 8.0, 40.0), egui::Sense::hover());
                    paint_histogram(ui, rect, h);
                }
            });
        }
        if section == Section::Color {
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                let bw = doc.settings.treatment == Treatment::Bw;
                for (label, t) in [("Color", Treatment::Color), ("Black & White", Treatment::Bw)] {
                    let on = (t == Treatment::Bw) == bw;
                    let b = egui::Button::new(RichText::new(label).color(if on { Color32::WHITE } else { theme::TEXT }))
                        .fill(if on { ACCENT } else { theme::RAISED })
                        .min_size(vec2(0.0, 38.0));
                    if ui.add(b).clicked() && !on {
                        doc.edit(|s| s.treatment = t);
                    }
                }
            });
        }
        let specs: Vec<&'static ControlSpec> = controls::in_section(section).collect();
        // sections with many sliders in groups (Color Mixer bands, Grading wheels): one group at a time
        let groups = group_names(&specs);
        let mut chosen = None;
        if groups.len() > 1 {
            let cur = self.sub_group.get(&section).cloned().filter(|g| groups.contains(g)).unwrap_or_else(|| groups[0].clone());
            let labels: Vec<String> = groups.iter().map(|g| title_case(g)).collect();
            let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
            if let Some(i) = widgets::chip_row(ui, ("groups", section.label()), &refs, groups.iter().position(|g| *g == cur)) {
                self.sub_group.insert(section, groups[i].clone());
            }
            chosen = Some(self.sub_group.get(&section).cloned().unwrap_or(cur));
            ui.add_space(4.0);
        }
        for spec in specs {
            if let Some(g) = &chosen
                && let Some(sg) = group_of(spec.id)
                && sg != g
            {
                continue;
            }
            control_row(ui, doc, spec, raw);
        }
    }

    fn crop_panel(&mut self, ui: &mut Ui) {
        let Some(doc) = &mut self.doc else { return };
        let (w, h) = doc.oriented_size();
        let cur = doc.settings.crop.aspect;
        let labels: Vec<&str> = crop::ASPECTS.iter().map(|a| a.0).collect();
        let sel = crop::ASPECTS.iter().position(|a| a.1 == cur);
        if let Some(i) = widgets::chip_row(ui, "aspects", &labels, sel) {
            let a = crop::ASPECTS[i].1;
            doc.edit(|s| crop::set_aspect(s, a, w, h));
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            if icons::tool(ui, Icon::RotateLeft, "Rotate L", false, true).clicked() {
                doc.edit(|s| rotate(s, 3));
            }
            if icons::tool(ui, Icon::RotateRight, "Rotate R", false, true).clicked() {
                doc.edit(|s| rotate(s, 1));
            }
            if icons::tool(ui, Icon::FlipH, "Flip H", doc.settings.crop.flip_h, true).clicked() {
                doc.edit(|s| s.crop.flip_h = !s.crop.flip_h);
            }
            if icons::tool(ui, Icon::FlipV, "Flip V", doc.settings.crop.flip_v, true).clicked() {
                doc.edit(|s| s.crop.flip_v = !s.crop.flip_v);
            }
            if icons::tool(ui, Icon::Reset, "Reset", false, true).clicked() {
                let o = doc.original.clone();
                doc.edit(|s| {
                    s.crop = o.crop.clone();
                    s.orientation = o.orientation;
                });
            }
        });
        let angle = doc.settings.crop.geometry.angle;
        let pos = (angle + 45.0) / 90.0;
        let out = widgets::slider(ui, "straighten", "Straighten", &format!("{angle:+.1}°"), pos, &TrackPaint::Centered(0.5), true);
        let (w, h) = doc.oriented_size();
        if out.started {
            doc.begin();
        }
        if let Some(p) = out.moved_to {
            let a = ((p * 90.0 - 45.0) * 10.0).round() / 10.0;
            doc.edit(|s| crop::set_angle(s, a, w, h));
        }
        if out.reset {
            doc.edit(|s| crop::set_angle(s, 0.0, w, h));
        }
        if out.finished {
            doc.end();
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.add(egui::Label::new(RichText::new("Drag the corners or edges; drag inside to move. Double-tap a slider to reset it.").small().color(TEXT_DIM)).wrap());
        });
    }

    fn looks_panel(&mut self, ui: &mut Ui) {
        let Some(doc) = &mut self.doc else { return };
        if let Some(i) = self.look.selected {
            let name = self.looks.get(i).map(|p| p.name.clone()).unwrap_or_default();
            let out = widgets::slider(ui, "look-amount", &format!("{name} amount"), &format!("{:.0}%", self.look.amount * 100.0), self.look.amount / 2.0, &TrackPaint::Plain, true);
            if out.started {
                doc.begin();
            }
            let new = out.moved_to.map(|p| (p * 2.0 * 100.0).round() / 100.0).or(out.reset.then_some(1.0));
            if let Some(a) = new
                && let (Some(base), Some(p)) = (&self.look.base, self.looks.get(i))
            {
                self.look.amount = a;
                let s = p.apply(base, a);
                doc.set(s);
            }
            if out.finished {
                doc.end();
            }
        }
        let mut groups: Vec<String> = Vec::new();
        for p in &self.looks {
            if !groups.contains(&p.group) {
                groups.push(p.group.clone());
            }
        }
        let mut pick: Option<Option<usize>> = None;
        ui.horizontal_wrapped(|ui| {
            ui.add_space(8.0);
            let on = self.look.selected.is_none();
            if ui.add(egui::Button::new("None").fill(if on { ACCENT } else { theme::RAISED }).min_size(vec2(0.0, 40.0))).clicked() {
                pick = Some(None);
            }
        });
        for g in groups {
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.label(RichText::new(&g).small().color(TEXT_DIM));
            });
            ui.horizontal_wrapped(|ui| {
                ui.add_space(8.0);
                for (i, p) in self.looks.iter().enumerate().filter(|(_, p)| p.group == g) {
                    let on = self.look.selected == Some(i);
                    let b = egui::Button::new(RichText::new(&p.name).color(if on { Color32::WHITE } else { theme::TEXT }))
                        .fill(if on { ACCENT } else { theme::RAISED })
                        .min_size(vec2(0.0, 40.0));
                    if ui.add(b).clicked() {
                        pick = Some(Some(i));
                    }
                }
            });
        }
        match pick {
            Some(None) => {
                if let Some(base) = self.look.base.take() {
                    doc.set(base);
                }
                self.look.selected = None;
            }
            Some(Some(i)) => {
                let base = self.look.base.get_or_insert_with(|| doc.settings.clone()).clone();
                self.look.selected = Some(i);
                self.look.amount = 1.0;
                doc.set(self.looks[i].apply(&base, 1.0));
            }
            None => {}
        }
    }

    fn export_dialog(&mut self, ctx: &egui::Context) {
        let Some(x) = &mut self.export else { return };
        // progress from the worker
        if let Some(rx) = x.rx.take() {
            let mut keep_rx = true;
            while let Ok(p) = rx.try_recv() {
                match p {
                    export::Progress::Step(s, f) => {
                        x.step = s.to_string();
                        x.fraction = f;
                    }
                    export::Progress::Written { path, name, mime, width, height } => {
                        x.step = format!("Saving {width} × {height}…");
                        x.fraction = 0.95;
                        platform::save_to_gallery(&path, &name, mime, "LightCraft");
                        keep_rx = false;
                    }
                    export::Progress::Failed(e) => {
                        x.result = Some(e);
                        keep_rx = false;
                    }
                }
                if !keep_rx {
                    break;
                }
            }
            if keep_rx {
                x.rx = Some(rx);
            }
        }
        let busy = x.rx.is_some() || (x.fraction > 0.0 && x.result.is_none());
        let mut start = false;
        let mut close = false;
        let keep = widgets::modal_card(ctx, "export", |ui| {
            ui.heading("Export");
            ui.add_space(8.0);
            if let Some(r) = &x.result {
                ui.label(r);
                ui.add_space(12.0);
                if widgets::primary_button(ui, "Done").clicked() {
                    close = true;
                }
                return;
            }
            if busy {
                widgets::progress(ui, x.fraction, &x.step);
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
                return;
            }
            ui.label(RichText::new("Size").color(TEXT_DIM));
            ui.horizontal_wrapped(|ui| {
                for (label, edge) in export::SIZES {
                    let on = x.opts.long_edge == edge;
                    if ui.add(egui::Button::new(label).fill(if on { ACCENT } else { theme::RAISED }).min_size(vec2(0.0, 40.0))).clicked() {
                        x.opts.long_edge = edge;
                    }
                }
            });
            ui.add_space(6.0);
            ui.label(RichText::new("Format").color(TEXT_DIM));
            ui.horizontal(|ui| {
                for (label, f) in [("JPEG", export::Format::Jpeg), ("PNG", export::Format::Png)] {
                    let on = x.opts.format == f;
                    if ui.add(egui::Button::new(label).fill(if on { ACCENT } else { theme::RAISED }).min_size(vec2(72.0, 40.0))).clicked() {
                        x.opts.format = f;
                    }
                }
            });
            if x.opts.format == export::Format::Jpeg {
                let q = f64::from(x.opts.quality);
                let out = widgets::slider(ui, "jpeg-q", "Quality", &format!("{q:.0}"), (q - 50.0) / 50.0, &TrackPaint::Plain, true);
                if let Some(p) = out.moved_to {
                    x.opts.quality = (50.0 + p * 50.0).round() as u8;
                }
                if out.reset {
                    x.opts.quality = 92;
                }
            }
            ui.add_space(12.0);
            if widgets::primary_button(ui, "Save to Gallery").clicked() {
                start = true;
            }
        });
        if !platform::is_android() && x.rx.is_none() && x.fraction == 0.0 && std::env::var("LC_TEST_EXPORT").is_ok_and(|v| v == "go") {
            start = true;
        }
        if start && let Some(d) = &self.doc {
            let stem = std::path::Path::new(&d.photo.name).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "photo".into());
            x.rx = Some(export::start(d.photo.path.clone(), stem, d.settings.clone(), x.opts, ctx.clone()));
            x.fraction = 0.01;
            x.step = "Starting…".into();
        }
        if close || (!keep && !busy) {
            self.export = None;
        }
    }

    fn remove_dialog(&mut self, ctx: &egui::Context) {
        let Some(key) = self.confirm_remove.clone() else { return };
        let mut answer = None;
        let keep = widgets::modal_card(ctx, "remove", |ui| {
            ui.heading("Remove from Recent?");
            ui.label(RichText::new("The app's copy and its edits are deleted. The photo in your gallery is not touched.").color(TEXT_DIM));
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new("Cancel").min_size(vec2(120.0, 44.0))).clicked() {
                    answer = Some(false);
                }
                if ui.add(egui::Button::new(RichText::new("Remove").color(Color32::WHITE)).fill(Color32::from_rgb(200, 60, 60)).min_size(vec2(120.0, 44.0))).clicked() {
                    answer = Some(true);
                }
            });
        });
        if answer == Some(true) {
            self.library.remove(&key);
            self.thumbs.remove(&key);
        }
        if answer.is_some() || !keep {
            self.confirm_remove = None;
        }
    }

    fn banner(&mut self, ctx: &egui::Context) {
        let Some((m, until)) = &self.message else { return };
        let now = ctx.input(|i| i.time);
        if now > *until {
            self.message = None;
            return;
        }
        let m = m.clone();
        egui::Area::new(egui::Id::unique("banner")).order(egui::Order::Tooltip).anchor(egui::Align2::CENTER_BOTTOM, vec2(0.0, -140.0)).show(ctx, |ui| {
            egui::Frame::new().fill(Color32::from_black_alpha(220)).corner_radius(20).inner_margin(egui::Margin::symmetric(16, 10)).show(ui, |ui| {
                ui.set_max_width(ctx.content_rect().width() - 48.0);
                ui.label(RichText::new(m).color(Color32::WHITE));
            });
        });
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
}

impl eframe::App for MobileApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_events(ctx);
        self.poll_loading(ctx);
        self.pump_renders(ctx);
        self.autosave();
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let safe = platform::safe_area(&ctx);
        egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::PANEL).inner_margin(safe)).show(ui, |ui| {
            if self.doc.is_some() || self.loading.is_some() {
                self.editor(ui);
            } else {
                self.home(ui);
            }
        });
        self.export_dialog(&ctx);
        self.remove_dialog(&ctx);
        self.banner(&ctx);
    }
}

// -------------------------------------------------------------------------------------------
// helpers

/// The group of a three-part control id (`mixer.red.hue` → `red`).
fn group_of(id: &str) -> Option<&str> {
    let mut parts = id.split('.');
    let (_, g, _) = (parts.next()?, parts.next()?, parts.next()?);
    Some(g)
}

fn group_names(specs: &[&ControlSpec]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in specs {
        if let Some(g) = group_of(s.id)
            && !out.iter().any(|o| o == g)
        {
            out.push(g.to_string());
        }
    }
    out
}

fn title_case(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// Mireds-based relative temperature for rendered (non-raw) photos, as upstream's desktop UI does.
fn k_to_rel(k: f64) -> f64 {
    ((1e6 / 6500.0 - 1e6 / k) / 0.8).clamp(-100.0, 100.0)
}
fn rel_to_k(r: f64) -> f64 {
    1e6 / (1e6 / 6500.0 - r * 0.8)
}

fn band_hue(band: u8) -> f32 {
    [0.0, 30.0, 60.0, 120.0, 180.0, 240.0, 270.0, 310.0].get(band as usize).copied().unwrap_or(0.0)
}

fn track_paint(spec: &ControlSpec) -> TrackPaint {
    match spec.track {
        Track::Plain => TrackPaint::Plain,
        Track::Centered => TrackPaint::Centered(spec.position(0.0) as f32),
        Track::Temp => TrackPaint::Gradient(vec![Color32::from_rgb(70, 120, 230), Color32::from_rgb(235, 235, 235), Color32::from_rgb(235, 200, 60)]),
        Track::Tint => TrackPaint::Gradient(vec![Color32::from_rgb(60, 200, 80), Color32::from_rgb(235, 235, 235), Color32::from_rgb(220, 70, 220)]),
        Track::Gradient { from, to } => TrackPaint::Gradient(vec![widgets::hex(from), widgets::hex(to)]),
        Track::Hue { band } => {
            let h = band_hue(band);
            TrackPaint::Gradient(vec![widgets::hue(h - 35.0), widgets::hue(h), widgets::hue(h + 35.0)])
        }
        Track::Sat { band } => TrackPaint::Gradient(vec![Color32::from_gray(128), widgets::hue(band_hue(band))]),
        Track::Lum { band } => {
            let c = widgets::hue(band_hue(band));
            TrackPaint::Gradient(vec![c.gamma_multiply(0.25), c, Color32::from_rgb(255, 255, 255).lerp_to_gamma(c, 0.4)])
        }
        Track::Rainbow => TrackPaint::Gradient((0..=6).map(|i| widgets::hue(i as f32 * 60.0)).collect()),
    }
}

/// One upstream control as a touch slider, wired to the document's undo.
fn control_row(ui: &mut Ui, doc: &mut Doc, spec: &'static ControlSpec, raw: bool) {
    let v = controls::get(&doc.settings, spec.id).unwrap_or(spec.default);
    // rendered photos: temperature on a relative scale (−100…+100), tint clamped to ±100
    let rel_temp = spec.id == "wb.temp" && !raw;
    let (pos, text) = if rel_temp {
        let r = k_to_rel(v);
        ((r + 100.0) / 200.0, format!("{r:+.0}"))
    } else if spec.id == "wb.temp" {
        (spec.position(v), format!("{v:.0} K"))
    } else {
        (spec.position(v), spec.format(v))
    };
    let track = if rel_temp { TrackPaint::Gradient(vec![Color32::from_rgb(70, 120, 230), Color32::from_rgb(235, 235, 235), Color32::from_rgb(235, 200, 60)]) } else { track_paint(spec) };
    let out = widgets::slider(ui, spec.id, spec.label, &text, pos, &track, true);
    if out.started {
        doc.begin();
    }
    let new = if let Some(p) = out.moved_to {
        Some(if rel_temp { rel_to_k((p * 200.0 - 100.0).round()) } else { snap(spec, spec.value_at(p)) })
    } else if out.reset {
        Some(if spec.id == "wb.temp" || spec.id == "wb.tint" { controls::get(&doc.original, spec.id).unwrap_or(spec.default) } else { spec.default })
    } else {
        None
    };
    if let Some(n) = new {
        doc.edit(|s| {
            controls::set(s, spec.id, n);
        });
    }
    if out.finished {
        doc.end();
    }
}

fn snap(spec: &ControlSpec, v: f64) -> f64 {
    if spec.step > 0.0 { spec.clamp((v / spec.step).round() * spec.step) } else { v }
}

fn rotate(s: &mut DevelopSettings, quarter_turns_cw: u8) {
    let (flip, q) = s.orientation.to_parts();
    s.orientation = lightcraft_geom::Orientation::from_parts(flip, (q + quarter_turns_cw) % 4);
    // the crop is reset to the whole photo (or the largest of its locked aspect)
    s.crop.geometry = Default::default();
    if s.crop.aspect.is_some() {
        let (w, h) = (1.0, 1.0);
        let _ = (w, h);
    }
}

fn paint_histogram(ui: &Ui, rect: egui::Rect, h: &lightcraft_raster::Histogram) {
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 6, theme::BG);
    let max = h.luma.iter().copied().max().unwrap_or(1).max(1) as f32;
    let n = h.luma.len().max(1);
    for (ch, color) in [(&h.r, Color32::from_rgba_unmultiplied(230, 60, 60, 110)), (&h.g, Color32::from_rgba_unmultiplied(60, 210, 80, 110)), (&h.b, Color32::from_rgba_unmultiplied(70, 120, 240, 110))] {
        let mut pts = vec![rect.left_bottom()];
        for (i, v) in ch.iter().enumerate() {
            let x = rect.left() + rect.width() * i as f32 / (n - 1).max(1) as f32;
            let y = rect.bottom() - rect.height() * (*v as f32 / max).sqrt().min(1.0);
            pts.push(egui::pos2(x, y));
        }
        pts.push(rect.right_bottom());
        p.add(egui::Shape::line(pts, egui::Stroke::new(1.2, color)));
    }
}
