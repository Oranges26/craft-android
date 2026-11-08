//! FilmCraft for Android.
//!
//! Projects are edited by upstream FilmCraft's own engine (`filmcraft-engine` sessions and their
//! commands: import, place, razor, trim, move, speed, gain, transitions, titles, effects, export),
//! frames come from its renderer and sound from its mixer. This crate is the touch interface.

mod app;
mod player;
mod store;
mod timeline;

pub use app::MobileApp;

pub const APP_NAME: &str = "FilmCraft";

pub fn run(mut options: eframe::NativeOptions, open: Vec<std::path::PathBuf>) -> eframe::Result {
    options.renderer = eframe::Renderer::Glow;
    eframe::run_native(APP_NAME, options, Box::new(move |cc| Ok(Box::new(MobileApp::new(cc, open)))))
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    craft_mobile_kit::platform::init_logging("FilmCraft");
    std::panic::set_hook(Box::new(|p| log::error!("panic: {p}")));
    let options = eframe::NativeOptions { android_app: Some(app), ..Default::default() };
    if let Err(e) = run(options, Vec::new()) {
        log::error!("FilmCraft stopped: {e}");
    }
}
