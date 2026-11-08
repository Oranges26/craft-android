# Craft for Android

Android versions of [LightCraft](https://github.com/storytold/lightcraft) and
[FilmCraft](https://github.com/storytold/filmcraft), plus a launcher that installs and updates them.

| App | What it is | Upstream |
|---|---|---|
| **LightCraft** | Non-destructive photo editor (Lightroom-style develop) | storytold/lightcraft |
| **FilmCraft** | Video editor (timeline, titles, effects, H.264 export) | storytold/filmcraft |
| **Craft Launcher** | Lists the apps, installs them, updates them | this repo |

## How it works

The ports don't fork upstream. Each app here is a small touch interface (`<app>/crate`) that
builds *inside* the upstream workspace and uses its engine crates unchanged: LightCraft's develop
settings, pipeline and decoders, FilmCraft's sessions, commands, renderer, mixer and exporter.
LightCraft's sliders are generated from upstream's own control table, and FilmCraft's effect
controls from its effect definitions, so new upstream controls appear without changes here.

```
lightcraft/crate      Rust: LightCraft's touch UI (copied to upstream/apps/lightcraft-mobile)
lightcraft/android    Gradle project that packages it
filmcraft/crate       Rust: FilmCraft's touch UI
filmcraft/android     Gradle project
mobile-kit            Rust shared by both: touch widgets, zoom/pan view, icons, Android bridge
common/android        Java activity shared by both (file picker, gallery, insets, back button)
launcher              Java: the launcher app
tools/ci_plan.py      decides which apps need a new build
.github/workflows     the build
signing               the key every APK is signed with (see below)
```

## Builds and updates

`.github/workflows/build.yml` runs every 6 hours, on pushes, and by hand (Actions ▸ Build Android
apps ▸ Run workflow). For each app it compares upstream's latest commit and this repo's port with
what the newest release was built from, and rebuilds only what changed:

1. checks out upstream at its latest commit and drops the port into its workspace,
2. builds the native library with `cargo ndk` (arm64),
3. packages and signs the APK with Gradle,
4. publishes a release tagged `<app>-b<build>` (the three newest are kept).

The launcher reads those releases and offers **Install** / **Update**. After the first install,
updates on Android 12+ go through without a confirmation prompt.

If upstream changes something the port depends on, that build fails, the previous release stays
installed and available, and GitHub emails you; the port then needs a fix.

## Signing

Android only installs an update signed with the same key as the installed app. The key is in
`signing/` so every build uses it. To keep it out of the repository instead, add repository
secrets `CRAFT_KEYSTORE_B64` (`base64 -w0 signing/craft.keystore`) and `CRAFT_KEYSTORE_PASSWORD`
(the contents of `signing/password.txt`), then delete `signing/`. **Don't change the key**:
installed apps would have to be uninstalled before they can be updated again.

## Developing

Each port runs on the desktop in a phone-sized window, with the same code:

```sh
git clone https://github.com/storytold/lightcraft upstream
cp -r lightcraft/crate upstream/apps/lightcraft-mobile && cp -r mobile-kit upstream/mobile-kit
cd upstream && cargo run -p lightcraft-mobile -- photo.jpg   # --landscape, --tablet
```

(FilmCraft the same way, with `filmcraft-mobile` and video files; on Linux it needs
`libasound2-dev` for sound.)

## Licence

The upstream projects are MIT OR Apache-2.0; so is this port.
