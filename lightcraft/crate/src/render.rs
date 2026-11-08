//! The preview renderer: one worker thread renders the latest requested view of the open photo
//! with upstream's pipeline (`render_cached`, so a slider drag re-runs only the stages it touches).

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use lightcraft_develop::DevelopSettings;
use lightcraft_pipeline::{Quality, RenderRequest, SourceInfo, StageCache};
use lightcraft_raster::{Histogram, Rgb32f, Rgba8};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Purpose {
    /// The editor's view.
    View,
    /// The unedited photo (hold to compare).
    Before,
}

pub struct Job {
    pub generation: u64,
    pub purpose: Purpose,
    pub settings: Arc<DevelopSettings>,
    pub max: (usize, usize),
    pub draft: bool,
    /// The crop tool shows the whole uncropped photo.
    pub uncropped: bool,
}

pub struct Done {
    pub generation: u64,
    pub purpose: Purpose,
    pub draft: bool,
    pub uncropped: bool,
    pub image: Rgba8,
    pub histogram: Histogram,
    pub millis: f64,
}

pub struct Renderer {
    tx: Sender<Job>,
    pub rx: Receiver<Done>,
    /// The last job sent (to skip identical requests).
    last: Option<(u64, Purpose, (usize, usize), bool, bool)>,
}

impl Renderer {
    pub fn new(src: Arc<Rgb32f>, info: SourceInfo, ctx: egui::Context) -> Renderer {
        let (tx, jobs) = channel::<Job>();
        let (done_tx, rx) = channel::<Done>();
        std::thread::Builder::new()
            .name("lc-render".into())
            .spawn(move || {
                let cache = StageCache::default();
                while let Ok(mut job) = jobs.recv() {
                    // only the newest request matters
                    while let Ok(next) = jobs.try_recv() {
                        job = next;
                    }
                    let t = std::time::Instant::now();
                    let req = RenderRequest {
                        quality: if job.draft { Quality::Draft } else { Quality::Full },
                        apply_crop: !job.uncropped,
                        ..RenderRequest::fit(job.max.0.max(16), job.max.1.max(16))
                    };
                    let r = lightcraft_pipeline::render_cached(&src, &info, &job.settings, &req, &cache);
                    let done = Done {
                        generation: job.generation,
                        purpose: job.purpose,
                        draft: job.draft,
                        uncropped: job.uncropped,
                        image: r.image,
                        histogram: r.histogram,
                        millis: t.elapsed().as_secs_f64() * 1000.0,
                    };
                    if done_tx.send(done).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            })
            .expect("render thread");
        Renderer { tx, rx, last: None }
    }

    /// Ask for a render unless the same one is already on its way.
    pub fn request(&mut self, job: Job) {
        let key = (job.generation, job.purpose, job.max, job.draft, job.uncropped);
        if self.last == Some(key) {
            return;
        }
        self.last = Some(key);
        let _ = self.tx.send(job);
    }
}

/// An egui image from a render.
pub fn to_color_image(img: &Rgba8) -> egui::ColorImage {
    let bytes: &[u8] = unsafe { std::slice::from_raw_parts(img.data.as_ptr().cast::<u8>(), img.data.len() * 4) };
    egui::ColorImage::from_rgba_unmultiplied([img.width, img.height], bytes)
}

/// A small copy (box filter) of a render, for the library's thumbnails.
pub fn shrink(img: &Rgba8, max_edge: usize) -> Rgba8 {
    let s = (img.width.max(img.height) as f64 / max_edge as f64).max(1.0);
    let (w, h) = (((img.width as f64 / s).round() as usize).max(1), ((img.height as f64 / s).round() as usize).max(1));
    let mut out = Rgba8::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let (x0, x1) = ((x as f64 * s) as usize, (((x + 1) as f64 * s) as usize).min(img.width).max((x as f64 * s) as usize + 1));
            let (y0, y1) = ((y as f64 * s) as usize, (((y + 1) as f64 * s) as usize).min(img.height).max((y as f64 * s) as usize + 1));
            let mut acc = [0u32; 4];
            let mut n = 0u32;
            for yy in y0..y1.min(img.height) {
                for xx in x0..x1.min(img.width) {
                    let p = img.data[yy * img.width + xx];
                    for c in 0..4 {
                        acc[c] += u32::from(p[c]);
                    }
                    n += 1;
                }
            }
            let n = n.max(1);
            out.data[y * w + x] = [(acc[0] / n) as u8, (acc[1] / n) as u8, (acc[2] / n) as u8, (acc[3] / n) as u8];
        }
    }
    out
}
