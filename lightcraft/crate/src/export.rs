//! Export: decode the original at full resolution, develop it with the photo's settings and save
//! the result to the gallery (Pictures/LightCraft).

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};

use lightcraft_develop::DevelopSettings;
use lightcraft_pipeline::{Quality, RenderRequest};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Jpeg,
    Png,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    /// Long edge in pixels; `None` = the original's size.
    pub long_edge: Option<usize>,
    pub quality: u8,
    pub format: Format,
}

impl Default for Options {
    fn default() -> Self {
        Options { long_edge: None, quality: 92, format: Format::Jpeg }
    }
}

pub const SIZES: [(&str, Option<usize>); 4] = [("Original", None), ("4096 px", Some(4096)), ("2048 px", Some(2048)), ("1080 px", Some(1080))];

pub enum Progress {
    Step(&'static str, f32),
    /// The file is written to `path`; save it to the gallery as `name`.
    Written { path: PathBuf, name: String, mime: &'static str, width: usize, height: usize },
    Failed(String),
}

/// Start an export on a worker thread; progress arrives on the returned channel.
pub fn start(source: PathBuf, stem: String, settings: DevelopSettings, opts: Options, ctx: egui::Context) -> Receiver<Progress> {
    let (tx, rx) = channel();
    std::thread::Builder::new()
        .name("lc-export".into())
        .spawn(move || {
            let send = |p: Progress| {
                let _ = tx.send(p);
                ctx.request_repaint();
            };
            match run(&source, &stem, &settings, opts, &send) {
                Ok(p) => send(p),
                Err(e) => send(Progress::Failed(e)),
            }
        })
        .expect("export thread");
    rx
}

fn run(source: &std::path::Path, stem: &str, s: &DevelopSettings, o: Options, send: &dyn Fn(Progress)) -> Result<Progress, String> {
    send(Progress::Step("Reading the original…", 0.05));
    let bytes = std::fs::read(source).map_err(|e| format!("Couldn't read the original: {e}"))?;
    send(Progress::Step("Decoding…", 0.15));
    let (src, info) = lightcraft_engine::files::load_vec(bytes, usize::MAX, Default::default())?;
    send(Progress::Step("Developing…", 0.45));
    let (w, h) = lightcraft_pipeline::native_output_size(src.width, src.height, s);
    let (mut w, mut h) = (w.round().max(1.0) as usize, h.round().max(1.0) as usize);
    if let Some(edge) = o.long_edge
        && w.max(h) > edge
    {
        let k = edge as f64 / w.max(h) as f64;
        (w, h) = (((w as f64) * k).round().max(1.0) as usize, ((h as f64) * k).round().max(1.0) as usize);
    }
    lightcraft_pipeline::check_output_size(w, h)?;
    let req = RenderRequest { quality: Quality::Full, ..RenderRequest::exact(w, h) };
    let r = lightcraft_pipeline::render(&src, &info, s, &req);
    drop(src);
    send(Progress::Step("Encoding…", 0.85));
    let img = lightcraft_codecs::EncodeImage::rgba8(&r.image);
    let meta = lightcraft_codecs::EncodeMeta::default();
    let (bytes, ext, mime) = match o.format {
        Format::Jpeg => {
            let rgb = strip_alpha(&r.image);
            let e = lightcraft_codecs::EncodeImage::new(r.image.width as u32, r.image.height as u32, 3, lightcraft_codecs::Samples::U8(&rgb));
            (lightcraft_codecs::encode_jpeg(&e, o.quality, lightcraft_codecs::ChromaSubsampling::default(), &meta).map_err(|e| e.to_string())?, "jpg", "image/jpeg")
        }
        Format::Png => (lightcraft_codecs::encode_png(&img, &meta).map_err(|e| e.to_string())?, "png", "image/png"),
    };
    let dir = craft_mobile_kit::platform::cache_dir().join("export");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name = format!("{stem}-edit.{ext}");
    let path = dir.join(&name);
    std::fs::write(&path, &bytes).map_err(|e| format!("Couldn't write the export: {e}"))?;
    Ok(Progress::Written { path, name, mime, width: r.image.width, height: r.image.height })
}

fn strip_alpha(img: &lightcraft_raster::Rgba8) -> Vec<u8> {
    let mut out = Vec::with_capacity(img.data.len() * 3);
    for p in &img.data {
        out.extend_from_slice(&p[..3]);
    }
    out
}
