//! Desktop build of the Android app, for development: the same touch UI in a phone-sized window.
//! `lightcraft-mobile [PHOTO] [--landscape] [--tablet]`

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let size = if args.iter().any(|a| a == "--tablet") {
        [1280.0, 800.0]
    } else if args.iter().any(|a| a == "--landscape") {
        [860.0, 400.0]
    } else {
        [400.0, 860.0]
    };
    let open = args.iter().find(|a| !a.starts_with("--")).map(std::path::PathBuf::from);
    let options = eframe::NativeOptions { viewport: egui::ViewportBuilder::default().with_inner_size(size).with_title("LightCraft (mobile)"), ..Default::default() };
    lightcraft_mobile::run(options, open)
}
