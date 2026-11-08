//! The touch interface: a home screen of projects, and the editor — monitor on top, transport,
//! the timeline, and a tool bar that changes with the selection.

use std::collections::HashMap;
use std::path::PathBuf;

use craft_mobile_kit::icons::{self, Icon};
use craft_mobile_kit::theme::{self, TEXT, TEXT_DIM};
use craft_mobile_kit::widgets::{self, TrackPaint};
use craft_mobile_kit::{Event, Layout, PickKind, platform};
use egui::{Color32, RichText, Ui, vec2};
use filmcraft_engine::Session;
use filmcraft_project::{ClipId, ItemId, ItemKind, ParamKind, ParamValue, Sequence, TrackItem, TrackKind};
use filmcraft_time::Tick;
use serde_json::{Value, json};

use crate::player::{FrameWorker, Playing, Sound};
use crate::store::{self, ProjectEntry, Store};
use crate::timeline::{self, Action, Timeline};

const ACCENT: Color32 = Color32::from_rgb(255, 112, 80);

struct Editor {
    s: Session,
    id: String,
    name: String,
    saved_revision: u64,
    changed_at: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sheet {
    Speed,
    Volume,
    Adjust,
    AddEffect,
    Export,
}

struct ExportJob {
    job: u64,
    path: PathBuf,
    name: String,
    saving: bool,
    result: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
struct ExportOpts {
    height: u32,
    crf: u8,
}

pub struct MobileApp {
    store: Store,
    posters: HashMap<String, Option<egui::TextureHandle>>,
    ed: Option<Editor>,
    frames: FrameWorker,
    monitor: Option<(egui::TextureHandle, [usize; 2])>,
    playing: Option<Playing>,
    timeline: Timeline,
    sheet: Option<Sheet>,
    export_opts: ExportOpts,
    export: Option<ExportJob>,
    effect_category: Option<String>,
    confirm_delete: Option<String>,
    message: Option<(String, f64)>,
    pending: Vec<PathBuf>,
    /// Imports waiting for the media to be adopted (desktop: paths given on the command line).
    import_on_open: bool,
}

impl MobileApp {
    pub fn new(cc: &eframe::CreationContext<'_>, open: Vec<PathBuf>) -> MobileApp {
        theme::apply(&cc.egui_ctx, ACCENT);
        platform::set_context(&cc.egui_ctx);
        MobileApp {
            store: Store::load(),
            posters: HashMap::new(),
            ed: None,
            frames: FrameWorker::new(cc.egui_ctx.clone()),
            monitor: None,
            playing: None,
            timeline: Timeline::default(),
            sheet: None,
            export_opts: ExportOpts { height: 1080, crf: 21 },
            export: None,
            effect_category: None,
            confirm_delete: None,
            message: None,
            import_on_open: !open.is_empty(),
            pending: open,
        }
    }

    fn say(&mut self, ctx: &egui::Context, msg: impl Into<String>) {
        self.message = Some((msg.into(), ctx.input(|i| i.time) + 3.5));
    }

    /// Run an engine command, reporting a failure on screen.
    fn run(&mut self, ctx: &egui::Context, id: &str, params: Value) -> Option<Value> {
        let ed = self.ed.as_mut()?;
        match ed.s.execute(id, params) {
            Ok(v) => {
                ed.changed_at = ctx.input(|i| i.time);
                Some(v)
            }
            Err(e) => {
                log::warn!("{id}: {e}");
                self.say(ctx, e.to_string());
                None
            }
        }
    }

    // ---------------------------------------------------------------------------------------
    // projects

    fn new_project(&mut self, ctx: &egui::Context) {
        let id = self.store.new_id();
        let n = self.store.projects.len() + 1;
        let name = format!("Project {n}");
        let mut s = Session::default();
        let _ = s.execute("file.newProject", json!({"name": name}));
        let _ = s.execute("file.newSequence", json!({"name": "Main", "video": 2, "audio": 2}));
        let _ = s.execute("file.saveAs", json!({"path": store::project_path(&id).to_string_lossy()}));
        self.store.upsert(ProjectEntry { id: id.clone(), name: name.clone(), modified: store::now(), seconds: 0.0 });
        let rev = s.revision;
        self.ed = Some(Editor { s, id, name, saved_revision: rev, changed_at: ctx.input(|i| i.time) });
        self.reset_view();
    }

    fn open_project(&mut self, ctx: &egui::Context, e: &ProjectEntry) {
        let mut s = Session::default();
        if let Err(err) = s.execute("file.open", json!({"path": store::project_path(&e.id).to_string_lossy()})) {
            self.say(ctx, format!("Couldn't open {}: {err}", e.name));
            return;
        }
        if s.state.active_sequence.is_none() {
            let first = s.project.sequences().next().map(|i| i.id);
            if let Some(id) = first {
                let _ = s.execute("sequence.open", json!({"sequence": id.0}));
                if s.state.active_sequence.is_none() {
                    s.state.active_sequence = Some(id);
                }
            }
        }
        let rev = s.revision;
        self.ed = Some(Editor { s, id: e.id.clone(), name: e.name.clone(), saved_revision: rev, changed_at: 0.0 });
        self.reset_view();
    }

    fn reset_view(&mut self) {
        self.monitor = None;
        self.playing = None;
        self.sheet = None;
        self.frames.forget();
        self.timeline = Timeline::default();
    }

    fn save(&mut self) {
        let Some(ed) = &mut self.ed else { return };
        if ed.s.revision == ed.saved_revision {
            return;
        }
        match ed.s.execute("file.save", json!({})) {
            Ok(_) => ed.saved_revision = ed.s.revision,
            Err(e) => log::warn!("save: {e}"),
        }
        let seconds = ed.s.active_sequence().map(|q| q.duration().seconds()).unwrap_or(0.0);
        self.store.upsert(ProjectEntry { id: ed.id.clone(), name: ed.name.clone(), modified: store::now(), seconds });
    }

    fn close_project(&mut self) {
        self.playing = None;
        self.save();
        if let (Some(ed), Some((tex, size))) = (&self.ed, &self.monitor) {
            let _ = (tex, size);
            if let Some(seq) = ed.s.state.active_sequence {
                let provider = ed.s.media.provider(ed.s.project.clone(), ed.s.services.clone());
                let opts = filmcraft_render::RenderOptions { scale: 0.2, ..Default::default() };
                let t = if ed.s.playhead() > Tick::ZERO { ed.s.playhead() } else { Tick::from_seconds_f64(0.5) };
                let (w, h, px) = filmcraft_render::render_sequence_rgba8(&ed.s.project, seq, t, opts, &provider);
                if w > 0 && h > 0 {
                    store::save_poster(&ed.id, w, h, &px);
                    self.posters.remove(&ed.id);
                }
            }
        }
        self.ed = None;
        self.reset_view();
    }

    // ---------------------------------------------------------------------------------------
    // media

    fn import(&mut self, ctx: &egui::Context, path: PathBuf, name: String) {
        if self.ed.is_none() {
            self.new_project(ctx);
        }
        let file = match store::adopt_media(&path, &name) {
            Ok(f) => f,
            Err(e) => return self.say(ctx, format!("Couldn't keep {name}: {e}")),
        };
        let Some(r) = self.run(ctx, "file.import", json!({"paths": [file.to_string_lossy()]})) else { return };
        let items: Vec<u64> = r["items"].as_array().map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
        if items.is_empty() {
            let errs = r["errors"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("; ")).unwrap_or_default();
            return self.say(ctx, if errs.is_empty() { format!("{name}: nothing to import") } else { errs });
        }
        for item in items {
            self.place(ctx, ItemId(item));
        }
    }

    /// Put a newly imported item on the timeline: pictures go to the end of V1 (the first
    /// clip sets the sequence's size and frame rate), sound-only files go under the playhead on A2.
    fn place(&mut self, ctx: &egui::Context, item: ItemId) {
        let Some(ed) = &self.ed else { return };
        let (video, _audio) = ed.s.project.source_streams(item);
        let seq = ed.s.active_sequence();
        let empty = seq.is_none_or(|q| q.video_tracks.iter().chain(q.audio_tracks.iter()).all(|t| t.items.is_empty()));
        let v1_end = seq.and_then(|q| q.video_tracks.first()).map(|t| t.items.iter().map(|i| i.start + i.duration).max().unwrap_or(Tick::ZERO)).unwrap_or(Tick::ZERO);
        let playhead = ed.s.playhead();
        let params = if video {
            json!({"item": item.0, "track": "V1", "audioTrack": "A1", "time": v1_end.0, "matchSequence": empty})
        } else {
            json!({"item": item.0, "track": "A2", "time": playhead.0})
        };
        if self.run(ctx, "timeline.place", params).is_some()
            && let Some(ed) = &mut self.ed
        {
            if video {
                ed.s.set_playhead(v1_end);
            }
            self.frames.forget();
        }
    }

    // ---------------------------------------------------------------------------------------
    // playback

    fn toggle_play(&mut self) {
        if self.playing.take().is_some() {
            return;
        }
        let Some(ed) = &mut self.ed else { return };
        let Some(seq) = ed.s.state.active_sequence else { return };
        let dur = ed.s.active_sequence().map(|q| q.duration()).unwrap_or(Tick::ZERO);
        if dur <= Tick::ZERO {
            return;
        }
        let mut from = ed.s.playhead();
        if from >= dur - Tick::from_seconds_f64(0.05) {
            from = Tick::ZERO;
            ed.s.set_playhead(from);
        }
        let sound = match Sound::start(&ed.s, seq, from) {
            Ok(s) => Some(s),
            Err(e) => {
                log::info!("playing without sound: {e}");
                None
            }
        };
        self.playing = Some(Playing { from, wall: std::time::Instant::now(), sound });
    }

    fn advance(&mut self, ctx: &egui::Context) {
        let Some(p) = &self.playing else { return };
        let Some(ed) = &mut self.ed else { return };
        let dur = ed.s.active_sequence().map(|q| q.duration()).unwrap_or(Tick::ZERO);
        let t = p.now();
        if t >= dur {
            ed.s.set_playhead(dur);
            self.playing = None;
        } else {
            ed.s.set_playhead(t);
            ctx.request_repaint();
        }
    }

    fn pump_frames(&mut self, ctx: &egui::Context, monitor_px: f32) {
        while let Ok(f) = self.frames.rx.try_recv() {
            if f.w == 0 || f.h == 0 {
                continue;
            }
            let img = egui::ColorImage::from_rgba_unmultiplied([f.w, f.h], &f.rgba);
            match &mut self.monitor {
                Some((tex, size)) => {
                    tex.set(img, egui::TextureOptions::LINEAR);
                    *size = [f.w, f.h];
                }
                None => self.monitor = Some((ctx.load_texture("program", img, egui::TextureOptions::LINEAR), [f.w, f.h])),
            }
        }
        let Some(ed) = &self.ed else { return };
        let Some(seq) = ed.s.state.active_sequence else { return };
        let w = ed.s.active_sequence().map(|q| q.settings.width.max(1)).unwrap_or(1920) as f32;
        // a smaller frame while playing keeps up on a phone; full monitor size when still
        let cap = if self.playing.is_some() { 0.5 } else { 1.0 };
        let scale = (monitor_px / w).clamp(0.1, cap);
        let t = ed.s.playhead();
        self.frames.request(&ed.s, seq, t, (scale * 20.0).ceil() / 20.0);
    }

    // ---------------------------------------------------------------------------------------
    // events

    fn handle_events(&mut self, ctx: &egui::Context) {
        for e in platform::poll_events() {
            match e {
                Event::Picked { path, name } => self.import(ctx, path, name),
                Event::PickCancelled => {}
                Event::Saved { ok: _, message } => {
                    if let Some(x) = &mut self.export {
                        x.saving = false;
                        x.result = Some(message.clone());
                    }
                    self.say(ctx, message);
                }
                Event::Text { id, value } => self.text_entered(ctx, &id, value),
                Event::Back => {
                    if self.confirm_delete.is_some() {
                        self.confirm_delete = None;
                    } else if self.sheet.is_some() {
                        if self.export.as_ref().is_none_or(|x| x.result.is_some()) {
                            self.sheet = None;
                            self.export = None;
                        }
                    } else if self.ed.is_some() {
                        self.close_project();
                    } else {
                        platform::move_to_background();
                    }
                }
            }
        }
        if self.import_on_open && !self.pending.is_empty() {
            self.import_on_open = false;
            for p in std::mem::take(&mut self.pending) {
                let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                self.import(ctx, p, name);
            }
            if let Some(ed) = &mut self.ed {
                ed.s.set_playhead(Tick::from_seconds_f64(1.0));
            }
            self.test_hooks(ctx);
        }
    }

    /// Desktop test hooks (screenshots without touch): FC_TEST_SELECT=1 selects the first clip,
    /// FC_TEST_SHEET=speed|volume|adjust|effects|export opens a sheet, FC_TEST_TEXT adds a title,
    /// FC_TEST_SPLIT=1 splits at the playhead, FC_TEST_EXPORT=go exports.
    fn test_hooks(&mut self, ctx: &egui::Context) {
        if platform::is_android() {
            return;
        }
        if let Ok(t) = std::env::var("FC_TEST_TEXT") {
            self.text_entered(ctx, "title", t);
        }
        if std::env::var_os("FC_TEST_SPLIT").is_some() {
            self.split(ctx);
        }
        if std::env::var_os("FC_TEST_SELECT").is_some() {
            let first = self.ed.as_ref().and_then(|e| e.s.active_sequence()).and_then(|q| q.video_tracks.first()).and_then(|t| t.items.first()).map(|i| i.id);
            if let Some(c) = first {
                self.run(ctx, "timeline.select", json!({"clips": [c.0]}));
            }
        }
        if std::env::var_os("FC_TEST_TRANSITION").is_some() {
            self.transition(ctx);
        }
        self.sheet = match std::env::var("FC_TEST_SHEET").as_deref() {
            Ok("speed") => Some(Sheet::Speed),
            Ok("volume") => Some(Sheet::Volume),
            Ok("adjust") => Some(Sheet::Adjust),
            Ok("effects") => Some(Sheet::AddEffect),
            Ok("export") => Some(Sheet::Export),
            _ => None,
        };
        if std::env::var("FC_TEST_EXPORT").is_ok_and(|v| v == "go") {
            self.sheet = Some(Sheet::Export);
            self.start_export(ctx);
        }
    }

    fn text_entered(&mut self, ctx: &egui::Context, id: &str, value: String) {
        match id {
            "title" if !value.trim().is_empty() => {
                // a lower-third title, sized to the sequence's frame
                let (w, h) = self.ed.as_ref().and_then(|e| e.s.active_sequence()).map(|q| (f64::from(q.settings.width), f64::from(q.settings.height))).unwrap_or((1920.0, 1080.0));
                let params = json!({"text": value, "newClip": true, "size": (h * 0.075).round(), "position": [(w * 0.07).round(), (h * 0.68).round()], "box": [(w * 0.86).round(), (h * 0.26).round()]});
                if self.run(ctx, "graphics.newText", params).is_some() {
                    self.frames.forget();
                }
            }
            "rename" if !value.trim().is_empty() => {
                if let Some(ed) = &mut self.ed {
                    ed.name = value.trim().to_string();
                    ed.saved_revision = 0; // save the new name with the project
                }
            }
            _ => {}
        }
    }

    // ---------------------------------------------------------------------------------------
    // edits

    fn selected_clip(&self) -> Option<(TrackKind, TrackItem)> {
        let ed = self.ed.as_ref()?;
        let id = *ed.s.state.selection.first()?;
        let q = ed.s.active_sequence()?;
        for t in &q.video_tracks {
            if let Some(i) = t.items.iter().find(|i| i.id == id) {
                return Some((TrackKind::Video, i.clone()));
            }
        }
        for t in &q.audio_tracks {
            if let Some(i) = t.items.iter().find(|i| i.id == id) {
                return Some((TrackKind::Audio, i.clone()));
            }
        }
        None
    }

    fn split(&mut self, ctx: &egui::Context) {
        let Some(ed) = &self.ed else { return };
        let t = ed.s.playhead();
        let r = match self.selected_clip() {
            Some((_, c)) if c.start < t && t < c.start + c.duration => self.run(ctx, "timeline.razor", json!({"time": t.0, "clip": c.id.0})),
            _ => self.run(ctx, "sequence.addEditAllTracks", json!({})),
        };
        if r.is_some() {
            self.frames.forget();
        }
    }

    fn transition(&mut self, ctx: &egui::Context) {
        let Some((kind, c)) = self.selected_clip() else { return };
        let Some(ed) = &self.ed else { return };
        // the cut nearer the playhead
        let t = ed.s.playhead();
        let edge = if (t - c.start).0.abs() < (c.start + c.duration - t).0.abs() { "in" } else { "out" };
        let cmd = if kind == TrackKind::Video { "sequence.applyVideoTransition" } else { "sequence.applyAudioTransition" };
        if self.run(ctx, cmd, json!({"clip": c.id.0, "edge": edge})).is_some() {
            self.say(ctx, "Transition added");
        }
    }

    fn apply_timeline(&mut self, ctx: &egui::Context, actions: Vec<Action>) {
        for a in actions {
            match a {
                Action::Seek(t) => {
                    self.playing = None;
                    if let Some(ed) = &mut self.ed {
                        let dur = ed.s.active_sequence().map(|q| q.duration()).unwrap_or(Tick::ZERO);
                        ed.s.set_playhead(t.min(dur));
                    }
                }
                Action::Select(c) => {
                    let clips: Vec<u64> = c.map(|c| vec![c.0]).unwrap_or_default();
                    self.run(ctx, "timeline.select", json!({"clips": clips}));
                    if c.is_none() && matches!(self.sheet, Some(Sheet::Speed | Sheet::Volume | Sheet::Adjust | Sheet::AddEffect)) {
                        self.sheet = None;
                    }
                }
                Action::Trim { clip, edge_in, delta } => {
                    let rate = self.ed.as_ref().map(|e| e.s.sequence_rate());
                    let delta = rate.map(|r| r.snap_nearest(delta)).unwrap_or(delta);
                    self.run(ctx, "timeline.trim", json!({"clip": clip.0, "edge": if edge_in { "in" } else { "out" }, "mode": "ripple", "delta": delta.0}));
                    self.frames.forget();
                }
                Action::Move { clip, track, time } => {
                    let rate = self.ed.as_ref().map(|e| e.s.sequence_rate());
                    let time = rate.map(|r| r.snap_nearest(time)).unwrap_or(time);
                    self.run(ctx, "timeline.move", json!({"moves": [{"clip": clip.0, "track": track.0, "time": time.0}]}));
                    self.frames.forget();
                }
            }
        }
    }

    fn autosave(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let due = self.ed.as_ref().is_some_and(|e| e.s.revision != e.saved_revision && now - e.changed_at > 1.5);
        if due && !self.timeline.is_dragging() && self.playing.is_none() {
            self.save();
        }
        if self.ed.as_ref().is_some_and(|e| e.s.revision != e.saved_revision) {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }

    // ---------------------------------------------------------------------------------------
    // export

    fn start_export(&mut self, ctx: &egui::Context) {
        let Some(ed) = &self.ed else { return };
        let Some(q) = ed.s.active_sequence() else { return };
        if q.duration() <= Tick::ZERO {
            return self.say(ctx, "Add a clip first");
        }
        let (sw, sh) = (q.settings.width.max(2), q.settings.height.max(2));
        let h = self.export_opts.height.min(sh);
        let w = ((f64::from(sw) * f64::from(h) / f64::from(sh)) / 2.0).round() as u32 * 2;
        let dir = platform::cache_dir().join("export");
        let _ = std::fs::create_dir_all(&dir);
        let stem: String = ed.name.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect();
        let name = format!("{} {}.mp4", stem.trim(), store::now() % 100000);
        let path = dir.join(&name);
        let _ = std::fs::remove_file(&path);
        let params = json!({"path": path.to_string_lossy(), "format": "h264", "width": w, "height": h - h % 2, "bitrateMode": "crf", "crf": self.export_opts.crf, "audio": true});
        if let Some(r) = self.run(ctx, "file.exportMedia", params)
            && let Some(job) = r["job"].as_u64()
        {
            self.export = Some(ExportJob { job, path, name, saving: false, result: None });
        }
    }

    fn poll_export(&mut self) -> Option<f32> {
        let x = self.export.as_mut()?;
        let ed = self.ed.as_mut()?;
        ed.s.pump_jobs(std::time::Duration::from_millis(8));
        if x.saving || x.result.is_some() {
            return Some(1.0);
        }
        let j = ed.s.jobs.iter().find(|j| j.id == x.job)?;
        let res = j.result.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let frac = j.progress.fraction();
        match res {
            None => Some(frac),
            Some(Ok(_)) => {
                x.saving = true;
                platform::save_to_gallery(&x.path, &x.name, "video/mp4", "FilmCraft");
                Some(1.0)
            }
            Some(Err(e)) => {
                x.result = Some(format!("Export failed: {e}"));
                Some(1.0)
            }
        }
    }

    // ---------------------------------------------------------------------------------------
    // screens

    fn home(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.heading(RichText::new("FilmCraft").size(26.0).strong());
        });
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(RichText::new("Video editing").color(TEXT_DIM));
        });
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.set_width(ui.available_width() - 8.0);
                if widgets::primary_button(ui, "+  New project").clicked() {
                    self.new_project(&ctx);
                    platform::pick_file(PickKind::Media);
                }
            });
        });
        ui.add_space(18.0);
        if self.store.projects.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.label(RichText::new("Start a project and add videos, photos or music.\nProjects save as you work.").color(TEXT_DIM));
            });
            return;
        }
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(RichText::new("Projects").strong());
            ui.label(RichText::new("· long-press to delete").small().color(TEXT_DIM));
        });
        let entries = self.store.projects.clone();
        let mut open = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for e in &entries {
                let tex = self.posters.entry(e.id.clone()).or_insert_with(|| store::load_poster(&e.id).map(|img| ctx.load_texture(&e.id, img, Default::default()))).clone();
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 76.0), egui::Sense::click());
                let row = rect.shrink2(vec2(8.0, 4.0));
                ui.painter().rect_filled(row, 10, theme::RAISED);
                let thumb = egui::Rect::from_min_size(row.min + vec2(6.0, 6.0), vec2(96.0, row.height() - 12.0));
                ui.painter().rect_filled(thumb, 6, theme::BG);
                if let Some(t) = tex {
                    let s = t.size_vec2();
                    let k = (thumb.width() / s.x).min(thumb.height() / s.y);
                    ui.painter().image(t.id(), egui::Rect::from_center_size(thumb.center(), s * k), egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                } else {
                    icons::paint(ui.painter(), Icon::Film, egui::Rect::from_center_size(thumb.center(), vec2(22.0, 22.0)), TEXT_DIM);
                }
                ui.painter().text(thumb.right_top() + vec2(12.0, 8.0), egui::Align2::LEFT_TOP, &e.name, egui::FontId::proportional(16.0), TEXT);
                ui.painter().text(thumb.right_top() + vec2(12.0, 32.0), egui::Align2::LEFT_TOP, timeline::clock(e.seconds.round()), egui::FontId::proportional(12.5), TEXT_DIM);
                if resp.clicked() {
                    open = Some(e.clone());
                }
                if resp.long_touched() || resp.secondary_clicked() {
                    self.confirm_delete = Some(e.id.clone());
                }
            }
            ui.add_space(24.0);
        });
        if let Some(e) = open {
            self.open_project(&ctx, &e);
        }
    }

    fn editor(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let full = ui.available_rect_before_wrap();
        let layout = Layout::of(full);
        self.top_bar(ui);
        // the tool bar: along the bottom in portrait, down the right side in landscape
        if layout == Layout::Portrait {
            egui::Panel::bottom("tools").exact_size(64.0).frame(egui::Frame::new().fill(theme::PANEL)).show(ui, |ui| {
                egui::ScrollArea::horizontal().scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
                    ui.horizontal_centered(|ui| self.tool_bar(ui));
                });
            });
        } else {
            egui::Panel::right("tools").exact_size(72.0).frame(egui::Frame::new().fill(theme::PANEL)).show(ui, |ui| {
                egui::ScrollArea::vertical().scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
                    ui.vertical_centered(|ui| self.tool_bar(ui));
                });
            });
        }
        let seq_aspect = self.ed.as_ref().and_then(|e| e.s.active_sequence()).map(|q| q.settings.width as f32 / q.settings.height.max(1) as f32).unwrap_or(16.0 / 9.0);
        match layout {
            Layout::Portrait => {
                let mon_h = (full.width() / seq_aspect).min(full.height() * 0.38);
                egui::Panel::top("monitor").exact_size(mon_h).frame(egui::Frame::new().fill(theme::BG)).show(ui, |ui| self.monitor(ui));
                egui::Panel::top("transport").exact_size(48.0).frame(egui::Frame::new().fill(theme::PANEL)).show(ui, |ui| self.transport(ui));
                egui::CentralPanel::default().frame(egui::Frame::new()).show(ui, |ui| self.timeline_area(ui));
            }
            Layout::Landscape => {
                let tl_h = (full.height() * 0.40).max(130.0);
                egui::Panel::bottom("timeline").exact_size(tl_h).frame(egui::Frame::new()).show(ui, |ui| self.timeline_area(ui));
                egui::Panel::bottom("transport").exact_size(44.0).frame(egui::Frame::new().fill(theme::PANEL)).show(ui, |ui| self.transport(ui));
                egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::BG)).show(ui, |ui| self.monitor(ui));
            }
        }
        self.sheets(&ctx, full);
    }

    fn top_bar(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        egui::Panel::top("top").exact_size(52.0).frame(egui::Frame::new().fill(theme::PANEL).inner_margin(egui::Margin::symmetric(4, 4))).show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                if icons::button(ui, Icon::Back, true).clicked() {
                    self.close_project();
                    return;
                }
                let (u, r) = self.ed.as_ref().map(|e| (e.s.history.can_undo(), e.s.history.can_redo())).unwrap_or_default();
                if icons::button(ui, Icon::Undo, u).clicked()
                    && let Some(e) = &mut self.ed
                {
                    e.s.undo();
                    e.changed_at = ctx.input(|i| i.time);
                    self.frames.forget();
                }
                if icons::button(ui, Icon::Redo, r).clicked()
                    && let Some(e) = &mut self.ed
                {
                    e.s.redo();
                    e.changed_at = ctx.input(|i| i.time);
                    self.frames.forget();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::Button::new(RichText::new("Export").strong()).min_size(vec2(84.0, 40.0))).clicked() {
                        self.playing = None;
                        self.sheet = Some(Sheet::Export);
                        self.export = None;
                    }
                    if let Some(e) = &self.ed {
                        let name = e.name.clone();
                        if ui.add(egui::Label::new(RichText::new(&name).color(TEXT_DIM)).truncate().sense(egui::Sense::click())).clicked() {
                            platform::prompt_text("rename", "Project name", &name);
                        }
                    }
                });
            });
        });
    }

    fn monitor(&mut self, ui: &mut Ui) {
        let rect = ui.available_rect_before_wrap();
        let resp = ui.allocate_rect(rect, egui::Sense::click());
        let ppp = ui.ctx().pixels_per_point();
        let ctx = ui.ctx().clone();
        self.pump_frames(&ctx, rect.width().min(rect.height() * 16.0 / 9.0) * ppp);
        let p = ui.painter_at(rect);
        let empty = self.ed.as_ref().and_then(|e| e.s.active_sequence()).is_none_or(|q| q.duration() <= Tick::ZERO);
        if empty {
            p.text(rect.center(), egui::Align2::CENTER_CENTER, "Tap  +  Media  to add videos and photos", egui::FontId::proportional(14.0), TEXT_DIM);
        } else if let Some((tex, size)) = &self.monitor {
            let s = vec2(size[0] as f32, size[1] as f32);
            let k = (rect.width() / s.x).min(rect.height() / s.y);
            p.image(tex.id(), egui::Rect::from_center_size(rect.center(), s * k), egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
        } else {
            widgets::busy(ui, rect, "Rendering…");
        }
        if resp.clicked() && !empty {
            self.toggle_play();
        }
    }

    fn transport(&mut self, ui: &mut Ui) {
        let (t, d) = self.ed.as_ref().map(|e| (e.s.playhead().seconds(), e.s.active_sequence().map(|q| q.duration().seconds()).unwrap_or(0.0))).unwrap_or_default();
        ui.horizontal_centered(|ui| {
            ui.add_space(8.0);
            ui.label(RichText::new(format!("{} / {}", fine_clock(t), fine_clock(d))).monospace().color(TEXT_DIM));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(4.0);
                // zoom to fit
                if ui.add(egui::Button::new(RichText::new("Fit").small()).min_size(vec2(44.0, 34.0))).clicked() && d > 0.0 {
                    let w = ui.ctx().content_rect().width();
                    self.timeline.pps = ((w * 0.45) / d as f32).clamp(8.0, 900.0);
                }
                ui.add_space(20.0);
                let playing = self.playing.is_some();
                if icons::button(ui, if playing { Icon::Pause } else { Icon::Play }, d > 0.0).clicked() {
                    self.toggle_play();
                }
                if icons::button(ui, Icon::ToStart, d > 0.0).clicked() {
                    self.playing = None;
                    if let Some(e) = &mut self.ed {
                        e.s.set_playhead(Tick::ZERO);
                    }
                }
            });
        });
    }

    fn timeline_area(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let Some(ed) = &self.ed else { return };
        let Some(seq): Option<Sequence> = ed.s.active_sequence().cloned() else {
            return;
        };
        let sel = ed.s.state.selection.clone();
        let ph = ed.s.playhead();
        let actions = self.timeline.show(ui, &seq, ph, &sel);
        self.apply_timeline(&ctx, actions);
    }

    fn tool_bar(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let has_seq = self.ed.as_ref().is_some_and(|e| e.s.state.active_sequence.is_some());
        let sel = self.selected_clip();
        {
            {
                ui.add_space(4.0);
                match &sel {
                    None => {
                        if icons::tool(ui, Icon::Plus, "Media", false, true).clicked() {
                            platform::pick_file(PickKind::Media);
                        }
                        if icons::tool(ui, Icon::Text, "Text", false, has_seq).clicked() {
                            platform::prompt_text("title", "Title text", "");
                        }
                        if icons::tool(ui, Icon::Music, "Audio", false, has_seq).clicked() {
                            platform::pick_file(PickKind::Media);
                        }
                        if icons::tool(ui, Icon::Split, "Split", false, has_seq).clicked() {
                            self.split(&ctx);
                        }
                    }
                    Some((kind, c)) => {
                        let video = *kind == TrackKind::Video;
                        if icons::tool(ui, Icon::Check, "Done", false, true).clicked() {
                            self.run(&ctx, "timeline.select", json!({"clips": []}));
                            self.sheet = None;
                        }
                        if icons::tool(ui, Icon::Split, "Split", false, true).clicked() {
                            self.split(&ctx);
                        }
                        if icons::tool(ui, Icon::Trash, "Delete", false, true).clicked() {
                            self.run(&ctx, "timeline.delete", json!({"clips": [c.id.0], "ripple": true}));
                            self.sheet = None;
                            self.frames.forget();
                        }
                        if icons::tool(ui, Icon::Speed, "Speed", self.sheet == Some(Sheet::Speed), true).clicked() {
                            self.sheet = toggle(self.sheet, Sheet::Speed);
                        }
                        if icons::tool(ui, Icon::Volume, "Volume", self.sheet == Some(Sheet::Volume), true).clicked() {
                            self.sheet = toggle(self.sheet, Sheet::Volume);
                        }
                        if icons::tool(ui, Icon::Sparkle, "Adjust", self.sheet == Some(Sheet::Adjust), true).clicked() {
                            self.sheet = toggle(self.sheet, Sheet::Adjust);
                        }
                        if video && icons::tool(ui, Icon::Photo, "Effects", self.sheet == Some(Sheet::AddEffect), true).clicked() {
                            self.sheet = toggle(self.sheet, Sheet::AddEffect);
                        }
                        if icons::tool(ui, Icon::Transition, "Transition", false, true).clicked() {
                            self.transition(&ctx);
                        }
                        if icons::tool(ui, Icon::Duplicate, "Duplicate", false, true).clicked() {
                            let track = self.ed.as_ref().and_then(|e| e.s.active_sequence()).and_then(|q| {
                                q.video_tracks.iter().chain(q.audio_tracks.iter()).find(|t| t.items.iter().any(|i| i.id == c.id)).map(|t| t.id)
                            });
                            if let Some(track) = track {
                                self.run(&ctx, "timeline.move", json!({"moves": [{"clip": c.id.0, "track": track.0, "time": (c.start + c.duration).0}], "copy": true, "insert": true}));
                                self.frames.forget();
                            }
                        }
                    }
                }
            }
        }
    }

    // ---------------------------------------------------------------------------------------
    // sheets

    fn sheets(&mut self, ctx: &egui::Context, full: egui::Rect) {
        let Some(sheet) = self.sheet else { return };
        let h = match sheet {
            Sheet::Speed | Sheet::Volume => 170.0,
            Sheet::Export => 330.0,
            _ => (full.height() * 0.45).clamp(220.0, 420.0),
        };
        let w = full.width().min(560.0);
        let mut close = false;
        egui::Area::new(egui::Id::unique("sheet"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_BOTTOM, if Layout::of(full) == Layout::Portrait { vec2(0.0, -64.0 - platform::safe_area(ctx).bottom as f32) } else { vec2(-36.0, -(platform::safe_area(ctx).bottom as f32)) })
            .show(ctx, |ui| {
                egui::Frame::new().fill(theme::PANEL).corner_radius(egui::CornerRadius { nw: 16, ne: 16, sw: 0, se: 0 }).inner_margin(egui::Margin::symmetric(4, 6)).show(ui, |ui| {
                    ui.set_width(w - 8.0);
                    ui.set_height(h);
                    ui.horizontal(|ui| {
                        ui.add_space(10.0);
                        ui.label(RichText::new(match sheet {
                            Sheet::Speed => "Speed",
                            Sheet::Volume => "Volume",
                            Sheet::Adjust => "Adjust clip",
                            Sheet::AddEffect => "Add an effect",
                            Sheet::Export => "Export video",
                        }).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if icons::button(ui, Icon::Close, true).clicked() {
                                close = true;
                            }
                        });
                    });
                    match sheet {
                        Sheet::Speed => self.speed_sheet(ui),
                        Sheet::Volume => self.volume_sheet(ui),
                        Sheet::Adjust => self.adjust_sheet(ui),
                        Sheet::AddEffect => self.effects_sheet(ui),
                        Sheet::Export => self.export_sheet(ui),
                    }
                });
            });
        if close {
            self.sheet = None;
            if self.export.as_ref().is_some_and(|x| x.result.is_some()) {
                self.export = None;
            }
        }
    }

    fn speed_sheet(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let Some((_, c)) = self.selected_clip() else { return };
        let speeds = [0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0, 4.0];
        let labels: Vec<String> = speeds.iter().map(|s| format!("{s}×")).collect();
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let cur = speeds.iter().position(|s| (s - c.speed.abs()).abs() < 1e-3);
        ui.add_space(8.0);
        if let Some(i) = widgets::chip_row(ui, "speeds", &refs, cur) {
            self.run(&ctx, "clip.speedDuration", json!({"clips": [c.id.0], "speed": speeds[i] * 100.0, "ripple": true, "reverse": c.reverse}));
            self.frames.forget();
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            let mut rev = c.reverse;
            if ui.checkbox(&mut rev, "Play backwards").changed() {
                self.run(&ctx, "clip.speedDuration", json!({"clips": [c.id.0], "speed": c.speed.abs() * 100.0, "ripple": true, "reverse": rev}));
                self.frames.forget();
            }
        });
    }

    fn volume_sheet(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let Some((_, c)) = self.selected_clip() else { return };
        // the clip's gain lives in its intrinsic Volume effect (level, dB)
        let (idx, db) = volume_of(&c);
        ui.add_space(8.0);
        let pos = ((db + 30.0) / 45.0).clamp(0.0, 1.0);
        let out = widgets::slider(ui, "clip-volume", "Level", &if db <= -59.0 { "muted".to_string() } else { format!("{db:+.1} dB") }, pos, &TrackPaint::Plain, true);
        if let (Some(p), Some(idx)) = (out.moved_to, idx) {
            let v = ((p * 45.0 - 30.0) * 2.0).round() / 2.0;
            self.run(&ctx, "effects.setParam", json!({"clip": c.id.0, "effect": idx, "param": "level", "value": v, "merge": format!("vol-{}", c.id.0)}));
        }
        if out.reset
            && let Some(idx) = idx
        {
            self.run(&ctx, "effects.setParam", json!({"clip": c.id.0, "effect": idx, "param": "level", "value": 0.0}));
        }
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            if ui.add(egui::Button::new("Mute").min_size(vec2(0.0, 38.0))).clicked()
                && let Some(idx) = idx
            {
                self.run(&ctx, "effects.setParam", json!({"clip": c.id.0, "effect": idx, "param": "level", "value": -96.0}));
            }
            if ui.add(egui::Button::new("Normalize").min_size(vec2(0.0, 38.0))).clicked() {
                self.run(&ctx, "clip.audioGain", json!({"clips": [c.id.0], "mode": "normalizeMax", "db": -1.0}));
            }
        });
    }

    fn adjust_sheet(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let Some((_, c)) = self.selected_clip() else { return };
        egui::ScrollArea::vertical().id_salt("adjust").show(ui, |ui| {
            ui.set_width(ui.available_width());
            for (idx, inst) in c.effects.iter().enumerate() {
                let Some(def) = filmcraft_project::find_effect(&inst.effect) else { continue };
                let editable: Vec<_> = def.params.iter().filter(|p| matches!(p.kind, ParamKind::Float { .. } | ParamKind::Angle | ParamKind::Bool)).collect();
                if editable.is_empty() {
                    continue;
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.label(RichText::new(def.name).strong().color(TEXT));
                    if !def.intrinsic {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if icons::button(ui, Icon::Trash, true).clicked() {
                                self.run(&ctx, "effects.remove", json!({"clip": c.id.0, "effect": idx}));
                                self.frames.forget();
                            }
                        });
                    }
                });
                for pd in editable {
                    let cur = inst.params.get(pd.id).map(|p| p.value.clone()).unwrap_or_else(|| pd.default.clone());
                    match &pd.kind {
                        ParamKind::Bool => {
                            let mut b = matches!(cur, ParamValue::Bool(true));
                            ui.horizontal(|ui| {
                                ui.add_space(12.0);
                                if ui.checkbox(&mut b, pd.label).changed() {
                                    self.run(&ctx, "effects.setParam", json!({"clip": c.id.0, "effect": idx, "param": pd.id, "value": b}));
                                    self.frames.forget();
                                }
                            });
                        }
                        kind => {
                            let (lo, hi, dec, unit) = match kind {
                                // percentages (scale…) whose slider stops at 100 % can still go further
                                ParamKind::Float { soft_min, soft_max, max, decimals, unit, .. } if *unit == "%" && *soft_max <= 100.0 && *max > *soft_max => {
                                    (*soft_min, max.min(400.0), *decimals, *unit)
                                }
                                ParamKind::Float { soft_min, soft_max, decimals, unit, .. } => (*soft_min, *soft_max, *decimals, *unit),
                                _ => (-180.0, 180.0, 1, "°"),
                            };
                            let v = cur.as_f64().unwrap_or(0.0);
                            let pos = if hi > lo { (v - lo) / (hi - lo) } else { 0.0 };
                            let track = if lo < 0.0 && hi > 0.0 { TrackPaint::Centered((-lo / (hi - lo)) as f32) } else { TrackPaint::Plain };
                            let text = format!("{v:.*}{unit}", dec as usize);
                            let out = widgets::slider(ui, (c.id.0, idx, pd.id), pd.label, &text, pos, &track, true);
                            let new = out.moved_to.map(|p| lo + p * (hi - lo)).or(out.reset.then(|| pd.default.as_f64().unwrap_or(0.0)));
                            if let Some(nv) = new {
                                let step = 10f64.powi(-i32::from(dec));
                                let nv = (nv / step).round() * step;
                                self.run(&ctx, "effects.setParam", json!({"clip": c.id.0, "effect": idx, "param": pd.id, "value": nv, "merge": format!("{}-{idx}-{}", c.id.0, pd.id)}));
                                self.frames.forget();
                            }
                        }
                    }
                }
                ui.separator();
            }
            ui.add_space(16.0);
        });
    }

    fn effects_sheet(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let Some((_, c)) = self.selected_clip() else { return };
        let defs: Vec<&filmcraft_project::EffectDef> =
            filmcraft_project::effect_defs().iter().filter(|d| {
                d.kind == filmcraft_project::EffectKind::Video
                    && !d.intrinsic
                    && !filmcraft_project::graphic::is_layer_id(d.id)
                    && !d.category.iter().any(|c| c.eq_ignore_ascii_case("obsolete"))
            })
            .collect();
        let mut cats: Vec<String> = Vec::new();
        for d in &defs {
            let c = d.category.last().copied().unwrap_or("Other").to_string();
            if !cats.contains(&c) {
                cats.push(c);
            }
        }
        let cur = self.effect_category.clone().filter(|c| cats.contains(c)).unwrap_or_else(|| cats.first().cloned().unwrap_or_default());
        let refs: Vec<&str> = cats.iter().map(String::as_str).collect();
        if let Some(i) = widgets::chip_row(ui, "fx-cats", &refs, cats.iter().position(|c| *c == cur)) {
            self.effect_category = Some(cats[i].clone());
        }
        let cur = self.effect_category.clone().unwrap_or(cur);
        egui::ScrollArea::vertical().id_salt("fx-list").show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.add_space(8.0);
                for d in defs.iter().filter(|d| d.category.last().copied().unwrap_or("Other") == cur) {
                    if ui.add(egui::Button::new(d.name).min_size(vec2(0.0, 40.0))).clicked() {
                        if self.run(&ctx, "effects.apply", json!({"clips": [c.id.0], "effect": d.id})).is_some() {
                            self.say(&ctx, format!("{} added — adjust it under Adjust", d.name));
                            self.frames.forget();
                        }
                    }
                }
            });
        });
    }

    fn export_sheet(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        if let Some(frac) = self.poll_export() {
            let x = self.export.as_ref().expect("export");
            ui.add_space(12.0);
            if let Some(r) = &x.result {
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    ui.add(egui::Label::new(r).wrap());
                });
            } else if x.saving {
                widgets::progress(ui, 1.0, "Saving to the gallery…");
            } else {
                widgets::progress(ui, frac, &format!("Exporting… {:.0}%", frac * 100.0));
                ui.add_space(8.0);
                if ui.button("Cancel").clicked()
                    && let Some(ed) = &self.ed
                    && let Some(j) = ed.s.jobs.iter().find(|j| j.id == x.job)
                {
                    j.progress.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
            return;
        }
        let sh = self.ed.as_ref().and_then(|e| e.s.active_sequence()).map(|q| q.settings.height).unwrap_or(1080);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            ui.label(RichText::new("Resolution").color(TEXT_DIM));
        });
        let heights: Vec<u32> = [480u32, 720, 1080, 1440, 2160].into_iter().filter(|h| *h <= sh.max(480)).collect();
        let labels: Vec<String> = heights.iter().map(|h| if *h == 2160 { "4K".into() } else { format!("{h}p") }).collect();
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        if let Some(i) = widgets::chip_row(ui, "exp-res", &refs, heights.iter().position(|h| *h == self.export_opts.height.min(sh))) {
            self.export_opts.height = heights[i];
        }
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            ui.label(RichText::new("Quality").color(TEXT_DIM));
        });
        let q = [("Best", 17u8), ("High", 21), ("Smaller file", 26)];
        let refs: Vec<&str> = q.iter().map(|x| x.0).collect();
        if let Some(i) = widgets::chip_row(ui, "exp-q", &refs, q.iter().position(|x| x.1 == self.export_opts.crf)) {
            self.export_opts.crf = q[i].1;
        }
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            ui.vertical(|ui| {
                ui.set_width(ui.available_width() - 10.0);
                if widgets::primary_button(ui, "Export H.264 to Gallery").clicked() {
                    self.playing = None;
                    self.start_export(&ctx);
                }
            });
        });
    }

    fn delete_dialog(&mut self, ctx: &egui::Context) {
        let Some(id) = self.confirm_delete.clone() else { return };
        let mut answer = None;
        let keep = widgets::modal_card(ctx, "del", |ui| {
            ui.heading("Delete project?");
            ui.label(RichText::new("The project and its edits are deleted. Imported media stay in the app until no project uses them.").color(TEXT_DIM));
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new("Cancel").min_size(vec2(120.0, 44.0))).clicked() {
                    answer = Some(false);
                }
                if ui.add(egui::Button::new(RichText::new("Delete").color(Color32::WHITE)).fill(Color32::from_rgb(200, 60, 60)).min_size(vec2(120.0, 44.0))).clicked() {
                    answer = Some(true);
                }
            });
        });
        if answer == Some(true) {
            self.store.remove(&id);
            self.posters.remove(&id);
        }
        if answer.is_some() || !keep {
            self.confirm_delete = None;
        }
    }

    fn banner(&mut self, ctx: &egui::Context) {
        let Some((m, until)) = &self.message else { return };
        if ctx.input(|i| i.time) > *until {
            self.message = None;
            return;
        }
        let m = m.clone();
        egui::Area::new(egui::Id::unique("banner")).order(egui::Order::Tooltip).anchor(egui::Align2::CENTER_BOTTOM, vec2(0.0, -90.0)).show(ctx, |ui| {
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
        self.advance(ctx);
        self.autosave(ctx);
        if let Some(ed) = &mut self.ed {
            for e in ed.s.drain_events() {
                if let filmcraft_engine::Event::Toast { message, error: true } = e {
                    self.message = Some((message, ctx.input(|i| i.time) + 3.5));
                }
            }
            if self.export.is_some() {
                ctx.request_repaint_after(std::time::Duration::from_millis(150));
            }
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let safe = platform::safe_area(&ctx);
        egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::PANEL).inner_margin(safe)).show(ui, |ui| {
            if self.ed.is_some() {
                self.editor(ui);
            } else {
                self.home(ui);
            }
        });
        self.delete_dialog(&ctx);
        self.banner(&ctx);
    }
}

fn toggle(cur: Option<Sheet>, s: Sheet) -> Option<Sheet> {
    if cur == Some(s) { None } else { Some(s) }
}

/// The clip's intrinsic Volume effect (index) and its level in dB.
fn volume_of(c: &TrackItem) -> (Option<usize>, f64) {
    for (i, e) in c.effects.iter().enumerate() {
        if e.effect == "volume"
            && let Some(p) = e.params.get("level")
        {
            return (Some(i), p.value.as_f64().unwrap_or(0.0));
        }
    }
    (None, 0.0)
}

fn fine_clock(t: f64) -> String {
    let t = t.max(0.0);
    let m = (t / 60.0).floor();
    let s = t - m * 60.0;
    format!("{m:.0}:{s:04.1}")
}

#[allow(dead_code)]
fn _kinds(_: &ItemKind, _: ClipId) {}
