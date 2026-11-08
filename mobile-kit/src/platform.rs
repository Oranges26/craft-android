//! The platform bridge: what the app asks of the phone (pick a file, save to the gallery, show a
//! toast, go to the background) and what the phone tells the app (a file was picked, a save
//! finished, the system bars' insets, the back button).
//!
//! On Android every call goes to `com.craftandroid.kit.CraftActivity` (common/java), a
//! `NativeActivity` subclass shared by every Craft port, and its answers come back through the
//! `Java_com_craftandroid_kit_CraftActivity_native*` functions below. On the desktop (used to
//! develop and test the ports) the same calls use the file system and `zenity`.

use std::path::PathBuf;
use std::sync::Mutex;

/// Something the platform told the app. Drained once per frame with [`poll_events`].
#[derive(Debug, Clone)]
pub enum Event {
    /// The user picked a file; it was copied into the app's cache at `path`.
    Picked { path: PathBuf, name: String },
    /// The picker was closed without a choice.
    PickCancelled,
    /// A save to the gallery finished.
    Saved { ok: bool, message: String },
    /// The system back button / gesture.
    Back,
    /// The text dialog opened with [`prompt_text`] was confirmed (`id` names the request).
    Text { id: String, value: String },
}

struct State {
    events: Vec<Event>,
    /// System bar / cutout insets in physical pixels: left, top, right, bottom.
    insets_px: [f32; 4],
    files_dir: Option<PathBuf>,
    cache_dir: Option<PathBuf>,
    ctx: Option<egui::Context>,
}

static STATE: Mutex<State> = Mutex::new(State { events: Vec::new(), insets_px: [0.0; 4], files_dir: None, cache_dir: None, ctx: None });

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut g)
}

fn push(e: Event) {
    let ctx = with_state(|s| {
        s.events.push(e);
        s.ctx.clone()
    });
    if let Some(c) = ctx {
        c.request_repaint();
    }
}

/// Give the bridge the egui context so platform callbacks can wake the UI.
pub fn set_context(ctx: &egui::Context) {
    with_state(|s| s.ctx = Some(ctx.clone()));
}

/// Events since the last call, oldest first.
pub fn poll_events() -> Vec<Event> {
    with_state(|s| std::mem::take(&mut s.events))
}

/// The safe-area margins in points (system bars, display cutouts).
pub fn safe_area(ctx: &egui::Context) -> egui::Margin {
    let [l, t, r, b] = with_state(|s| s.insets_px);
    let ppp = ctx.pixels_per_point().max(0.1);
    let m = |v: f32| (v / ppp).round().clamp(0.0, 127.0) as i8;
    egui::Margin { left: m(l), top: m(t), right: m(r), bottom: m(b) }
}

/// Persistent app storage (edits, settings).
pub fn files_dir() -> PathBuf {
    if let Some(d) = with_state(|s| s.files_dir.clone()) {
        return d;
    }
    let d = desktop_dir("data");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// Scratch storage (picked files, exports before they are saved).
pub fn cache_dir() -> PathBuf {
    if let Some(d) = with_state(|s| s.cache_dir.clone()) {
        return d;
    }
    let d = desktop_dir("cache");
    let _ = std::fs::create_dir_all(&d);
    d
}

fn desktop_dir(kind: &str) -> PathBuf {
    let base = std::env::var_os("CRAFT_MOBILE_HOME").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("craft-mobile"));
    base.join(kind)
}

/// Which files the picker offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickKind {
    Image,
    Video,
    /// Images, videos and audio (an editor's media bin).
    Media,
    Any,
}

impl PickKind {
    fn mime(self) -> &'static str {
        match self {
            PickKind::Image => "image/*",
            PickKind::Video => "video/*",
            PickKind::Media => "image/*,video/*,audio/*",
            PickKind::Any => "*/*",
        }
    }
}

/// Open the system file picker. The answer arrives later as [`Event::Picked`] or
/// [`Event::PickCancelled`].
pub fn pick_file(kind: PickKind) {
    imp::pick_file(kind.mime());
}

/// Save the file at `path` to the shared gallery (Pictures/<album> for images, Movies/<album>
/// for videos) under `display_name`. The answer arrives as [`Event::Saved`].
pub fn save_to_gallery(path: &std::path::Path, display_name: &str, mime: &str, album: &str) {
    imp::save_to_gallery(path, display_name, mime, album);
}

/// Ask for a line of text in a native dialog (the system keyboard comes up). The answer arrives
/// as [`Event::Text`] with the same `id`; nothing arrives when the dialog is cancelled.
pub fn prompt_text(id: &str, title: &str, initial: &str) {
    imp::prompt_text(id, title, initial);
}

/// A short message at the bottom of the screen.
pub fn toast(message: &str) {
    imp::toast(message);
}

/// Send the app to the background (back pressed with nothing left to close).
pub fn move_to_background() {
    imp::move_to_background();
}

/// Whether this build runs on a phone or tablet (so the UI can drop desktop affordances).
pub const fn is_android() -> bool {
    cfg!(target_os = "android")
}

#[cfg(not(target_os = "android"))]
mod imp {
    use super::*;

    pub fn pick_file(mime: &str) {
        let mime = mime.to_string();
        std::thread::spawn(move || {
            let filter = mime
                .split(',')
                .map(|m| match m {
                    "image/*" => "*.jpg *.jpeg *.png *.tif *.tiff *.webp *.heic *.dng *.cr2 *.cr3 *.nef *.arw *.raf *.orf *.rw2",
                    "video/*" => "*.mp4 *.mov *.mkv *.webm *.avi",
                    "audio/*" => "*.wav *.mp3 *.aac *.m4a *.flac *.ogg",
                    _ => "*",
                })
                .collect::<Vec<_>>()
                .join(" ");
            let out = std::process::Command::new("zenity").args(["--file-selection", &format!("--file-filter={filter}")]).output();
            match out {
                Ok(o) if o.status.success() => {
                    let p = PathBuf::from(String::from_utf8_lossy(&o.stdout).trim());
                    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    push(Event::Picked { path: p, name });
                }
                _ => push(Event::PickCancelled),
            }
        });
    }

    pub fn save_to_gallery(path: &std::path::Path, display_name: &str, _mime: &str, album: &str) {
        let dir = std::env::var_os("CRAFT_MOBILE_GALLERY").map(PathBuf::from).unwrap_or_else(|| desktop_dir("gallery")).join(album);
        let r = std::fs::create_dir_all(&dir).and_then(|_| std::fs::copy(path, dir.join(display_name)));
        match r {
            Ok(_) => push(Event::Saved { ok: true, message: format!("Saved to {}", dir.join(display_name).display()) }),
            Err(e) => push(Event::Saved { ok: false, message: format!("Couldn't save: {e}") }),
        }
    }

    pub fn toast(message: &str) {
        log::info!("toast: {message}");
    }

    pub fn prompt_text(id: &str, title: &str, initial: &str) {
        let (id, title, initial) = (id.to_string(), title.to_string(), initial.to_string());
        std::thread::spawn(move || {
            let out = std::process::Command::new("zenity").args(["--entry", &format!("--text={title}"), &format!("--entry-text={initial}")]).output();
            if let Ok(o) = out
                && o.status.success()
            {
                push(Event::Text { id, value: String::from_utf8_lossy(&o.stdout).trim_end_matches('\n').to_string() });
            }
        });
    }

    pub fn move_to_background() {}
}

#[cfg(target_os = "android")]
mod imp {
    use super::*;
    use jni::JNIEnv;
    use jni::objects::{JClass, JObject, JString, JValue};
    use jni::sys::{jboolean, jint};

    /// Run `f` with a JNI env attached to this thread and the activity object.
    fn with_activity(f: impl FnOnce(&mut JNIEnv, &JObject) -> jni::errors::Result<()>) {
        let ctx = ndk_context::android_context();
        let vm = match unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) } {
            Ok(vm) => vm,
            Err(e) => return log::error!("no JavaVM: {e}"),
        };
        let mut env = match vm.attach_current_thread() {
            Ok(env) => env,
            Err(e) => return log::error!("attach failed: {e}"),
        };
        let activity = unsafe { JObject::from_raw(ctx.context().cast()) };
        let env: &mut JNIEnv = &mut env;
        if let Err(e) = f(env, &activity) {
            log::error!("JNI call failed: {e}");
            let _ = env.exception_describe();
            let _ = env.exception_clear();
        }
    }

    pub fn pick_file(mime: &str) {
        with_activity(|env, act| {
            let m = env.new_string(mime)?;
            env.call_method(act, "pickFile", "(Ljava/lang/String;)V", &[JValue::Object(&m)])?;
            Ok(())
        });
    }

    pub fn save_to_gallery(path: &std::path::Path, display_name: &str, mime: &str, album: &str) {
        let path = path.to_string_lossy().to_string();
        with_activity(|env, act| {
            let p = env.new_string(path)?;
            let n = env.new_string(display_name)?;
            let m = env.new_string(mime)?;
            let a = env.new_string(album)?;
            env.call_method(
                act,
                "saveToGallery",
                "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)V",
                &[JValue::Object(&p), JValue::Object(&n), JValue::Object(&m), JValue::Object(&a)],
            )?;
            Ok(())
        });
    }

    pub fn toast(message: &str) {
        with_activity(|env, act| {
            let m = env.new_string(message)?;
            env.call_method(act, "toast", "(Ljava/lang/String;)V", &[JValue::Object(&m)])?;
            Ok(())
        });
    }

    pub fn prompt_text(id: &str, title: &str, initial: &str) {
        with_activity(|env, act| {
            let i = env.new_string(id)?;
            let t = env.new_string(title)?;
            let v = env.new_string(initial)?;
            env.call_method(
                act,
                "promptText",
                "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)V",
                &[JValue::Object(&i), JValue::Object(&t), JValue::Object(&v)],
            )?;
            Ok(())
        });
    }

    pub fn move_to_background() {
        with_activity(|env, act| {
            env.call_method(act, "moveTaskToBack", "(Z)Z", &[JValue::Bool(1)])?;
            Ok(())
        });
    }

    fn string(env: &mut JNIEnv, s: &JString) -> String {
        env.get_string(s).map(|s| s.into()).unwrap_or_default()
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_craftandroid_kit_CraftActivity_nativeOnDirs(mut env: JNIEnv, _c: JClass, files: JString, cache: JString) {
        let (f, c) = (string(&mut env, &files), string(&mut env, &cache));
        with_state(|s| {
            s.files_dir = Some(PathBuf::from(f));
            s.cache_dir = Some(PathBuf::from(c));
        });
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_craftandroid_kit_CraftActivity_nativeOnPicked(mut env: JNIEnv, _c: JClass, path: JString, name: JString) {
        let (p, n) = (string(&mut env, &path), string(&mut env, &name));
        if p.is_empty() { push(Event::PickCancelled) } else { push(Event::Picked { path: PathBuf::from(p), name: n }) }
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_craftandroid_kit_CraftActivity_nativeOnSaved(mut env: JNIEnv, _c: JClass, ok: jboolean, message: JString) {
        let m = string(&mut env, &message);
        push(Event::Saved { ok: ok != 0, message: m });
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_craftandroid_kit_CraftActivity_nativeOnInsets(_env: JNIEnv, _c: JClass, l: jint, t: jint, r: jint, b: jint) {
        let ctx = with_state(|s| {
            s.insets_px = [l as f32, t as f32, r as f32, b as f32];
            s.ctx.clone()
        });
        if let Some(c) = ctx {
            c.request_repaint();
        }
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_craftandroid_kit_CraftActivity_nativeOnBack(_env: JNIEnv, _c: JClass) {
        push(Event::Back);
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_craftandroid_kit_CraftActivity_nativeOnText(mut env: JNIEnv, _c: JClass, id: JString, value: JString) {
        let (i, v) = (string(&mut env, &id), string(&mut env, &value));
        push(Event::Text { id: i, value: v });
    }
}

/// Android logging (logcat) for the port's crates; a no-op on the desktop, where `env_logger`
/// or the upstream logger is used.
pub fn init_logging(tag: &'static str) {
    #[cfg(target_os = "android")]
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_tag(tag));
    #[cfg(not(target_os = "android"))]
    let _ = tag;
}
