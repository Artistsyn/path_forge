# PathForge for iPhone

The iPhone version of PathForge, with the same editor as the Android app: pick a scene, change any
setting, save scenes of your own, and save it to Photos to set as your wallpaper. It draws on the
iPhone's GPU (Metal), with the same renderer as the desktop and Android (`mobile/`).

## Installing it without a paid Apple developer account

GitHub Actions (`.github/workflows/ios.yml`) builds **PathForge.ipa**, unsigned, into the
`ios-latest` release. iPhones only run signed apps, so it is signed on the way in with your own free
Apple ID, by **Sideloadly** (Windows or Mac) or **AltStore**. It needs iOS 16 or later.

With Sideloadly:

1. On a Windows PC or a Mac, install Sideloadly (sideloadly.io). On Windows it also needs iTunes and
   iCloud from Apple's website (not the Microsoft Store versions).
2. Download PathForge.ipa from the repository's Releases page (`ios-latest`).
3. Plug the iPhone in with a cable, unlock it, and tap **Trust** if it asks.
4. Drag PathForge.ipa into Sideloadly, type your Apple ID, and press **Start**. (A spare Apple ID is
   fine. If asked, sign in with an app-specific password from account.apple.com.)
5. On the iPhone: **Settings → General → VPN & Device Management**, tap your Apple ID under
   Developer App, and **Trust** it.
6. iOS 16 and later: **Settings → Privacy & Security → Developer Mode**, switch it on, and restart
   when asked. Then open PathForge.

The warnings along the way are expected: the app is signed by you, not by a published developer.

**Every 7 days** an app installed with a free Apple ID stops opening until it is installed again
(repeat step 4; your scenes are kept). AltStore (altstore.io) can refresh it by itself over Wi-Fi
while its AltServer runs on the computer. A free Apple ID can have up to 3 such apps at a time.

## Using it

- **Edit every setting**: every setting of the scene, part by part, built from the same description
  as the desktop's inspector and the Android editor. Long-press a setting's name for what it does;
  **Undo** steps back. **All settings** shows the rest (Post, Canvas, seeds…).
- **Save scene** keeps it as one of yours (★, first in the strip); long-press one of yours to
  rename, duplicate, export it to Files (for the desktop) or delete it. **Import** opens a scene file
  from the desktop.
- Quick changes: **Sunset** (Golden, Fiery, Pastel, Dusk), **Clouds** (a mix; **Shuffle** deals
  another) and **Walking speed**.

## The wallpaper

iPhone apps cannot set the wallpaper, and iOS has no live wallpapers like Android's. So PathForge
saves to Photos, and you set it from there (**Photos → the picture → Share → Use as Wallpaper**):

- **Still**: this moment, at the screen's full size.
- **Live Photo**: three seconds of the walk. On the lock screen it moves when the phone wakes
  (iOS 17 and later).
- **Video**: the whole loop, joining end to start, at the screen's size.

## Building it on a Mac

```bash
rustup target add aarch64-apple-ios
ios/build-native.sh        # the renderer, into ios/build/native
brew install xcodegen
cd ios && xcodegen generate && open PathForge.xcodeproj
```

In Xcode, choose your Apple ID under Signing & Capabilities to run it on your own iPhone.
