# PathForge for Android

The phone version of PathForge: pick a scene, give it a sunset or a mixed sky, set the pace, and
make it your phone's wallpaper. The desktop app is unchanged; this is a separate app built from the
same renderer (`android/native` wraps `path_forge::runtime` for Java).

## Installing

Every push that touches the app or the renderer builds it on GitHub Actions
(`.github/workflows/android.yml`) and publishes **PathForge.apk** as the `android-latest` release.
On the phone, open the repository's Releases page, download PathForge.apk, open it, and allow
installing from that source when Android asks. Newer builds install over older ones. It needs
Android 10 or later on a 64-bit ARM phone (practically every phone since 2019).

## Using it

- **Scenes**: the strip shows every built-in preset. **Import** opens a scene file made on the
  desktop (`.json`; copy it to the phone first). Scenes that use image files (kits, sprites) show
  without those images.
- **Sky**: leave it as designed, or turn it into a Golden, Fiery, Pastel or Dusk sunset (the sun is
  brought down low; at dusk it has set). **Clouds**: as designed, or a mix of several kinds;
  **Shuffle** deals another mix.
- **Walking speed**: from a stroll to a run.
- **Set as wallpaper** draws the loop once and makes it a live wallpaper. **Still** sets this moment
  as the home screen, lock screen or both. **Save** puts it in Pictures/PathForge.
- Tap the scene to hide the controls; **Edit** brings them back.

## Made for phones

- **The wallpaper costs almost nothing while it runs.** A PathForge loop joins without a seam, so
  the app renders it once ("Drawing your wallpaper's loop…") and the wallpaper only plays the
  frames back, one JPEG decoded per frame, and stops when the screen is off or an app covers it.
  **Live wallpaper** quality sets the frames' size and rate: Battery saver (360 px wide, 12 fps),
  Balanced (540 px, 20 fps), Smooth (720 px, 30 fps); the GPU scales them to the screen.
- **The preview adapts to the phone.** It renders a fraction of the screen's size, smaller on a
  slower phone, aiming for 20-30 frames a second, and stops when the app is in the background.
- **It draws on the phone's GPU** (Vulkan, through the same wgpu renderer the desktop uses on its
  GPU), sharing one GPU renderer between the preview, the wallpaper and stills so its shaders are
  built once. The line under the scene's name says what is drawing, at what size and how fast.
  Where the GPU cannot draw a frame (a phone with tighter limits), that frame and every one after
  is drawn on the CPU instead, across all cores. If a GPU driver crashes the app outright, the next
  start notices and switches to the CPU; **Draw on** switches by hand.

## Building it yourself

You need the Android SDK and NDK (Android Studio installs both), Rust with the Android target, and
cargo-ndk:

```bash
rustup target add aarch64-linux-android
cargo install cargo-ndk
export ANDROID_NDK_HOME=/path/to/ndk
android/build-native.sh             # the renderer, into app/src/main/jniLibs
cd android && ./gradlew assembleRelease
```

The APK is in `android/app/build/outputs/apk/release/`. Or open `android/` in Android Studio after
running `build-native.sh`. The app is signed with `app/sideload.keystore`, whose passwords are in
`app/build.gradle`: fine for installing it yourself, but a store release needs a private key.
