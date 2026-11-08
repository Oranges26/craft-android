//! LightCraft for Android.
//!
//! The develop engine is upstream LightCraft's own (`lightcraft-develop` settings, the
//! `lightcraft-pipeline` renderer, `lightcraft-engine` decoding and presets); this crate is only the
//! touch interface around it. Every slider comes from upstream's control table
//! (`lightcraft_develop::CONTROLS`), so controls added upstream show up here on the next build.

mod app;
mod crop;
mod doc;
mod export;
mod library;
mod render;

pub use app::MobileApp;

pub const APP_NAME: &str = "LightCraft";

/// Desktop and Android share this: build the window options and run the app.
pub fn run(mut options: eframe::NativeOptions, open: Option<std::path::PathBuf>) -> eframe::Result {
    options.renderer = eframe::Renderer::Glow;
    eframe::run_native(APP_NAME, options, Box::new(move |cc| Ok(Box::new(MobileApp::new(cc, open)))))
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    craft_mobile_kit::platform::init_logging("LightCraft");
    std::panic::set_hook(Box::new(|p| log::error!("panic: {p}")));
    let options = eframe::NativeOptions { android_app: Some(app), ..Default::default() };
    if let Err(e) = run(options, None) {
        log::error!("LightCraft stopped: {e}");
    }
}
