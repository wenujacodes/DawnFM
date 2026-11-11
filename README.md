<div align="center">
  <h1>BitChord</h1>
  <p><strong>A fast, native YouTube Music desktop app — rebuilt in Rust.</strong></p>
  <p>Built with egui and rodio. No Electron, no embedded browser player, and light on memory while playing.</p>

  <p>
    <a href="https://github.com/jackychan4891/BitChord-PC-Continued/releases/latest"><img src="https://img.shields.io/github/v/release/jackychan4891/BitChord-PC-Continued?style=flat-square&color=a855f7&labelColor=0d1117" alt="Latest release"></a>
    <a href="https://github.com/jackychan4891/BitChord-PC-Continued/releases"><img src="https://img.shields.io/github/downloads/jackychan4891/BitChord-PC-Continued/total?style=flat-square&color=a855f7&labelColor=0d1117" alt="Downloads"></a>
    <img src="https://img.shields.io/badge/platform-macOS-333?style=flat-square&labelColor=0d1117" alt="Platforms">
    <a href="LICENSE"><img src="https://img.shields.io/github/license/jackychan4891/BitChord-PC-Continued?style=flat-square&color=333&labelColor=0d1117" alt="License: GPL-3.0"></a>
  </p>

  <p>
    <a href="https://discord.gg/cWQSGjpXM">Discord</a> ·
    <a href="https://github.com/jackychan4891/BitChord-PC-Continued/releases/latest">Download</a> ·
    <a href="#features">Features</a> ·
    <a href="#installation">Installation</a> ·
    <a href="#building-from-source">Build from source</a>
  </p>
</div>

> **BitChord macOS** is a macOS-focused copy of [BitChord PC Continued](https://github.com/jackychan4891/BitChord-PC-Continued) by **[jackychan4891](https://github.com/jackychan4891)**. All credit for the original app goes to them. This repository is maintained separately by [wenujacodes](https://github.com/wenujacodes) and may be modified from the original.

> BitChord is an independent, unofficial client. It is not affiliated with, endorsed by, or connected to YouTube, Google, or Apple.

## Screenshots

<table>
  <tr>
    <td width="50%"><img src="docs/readme/screenshot-1.png" alt="Full player with artwork, lyrics and controls"></td>
    <td width="50%"><img src="docs/readme/screenshot-2.png" alt="Browse view with shelves and playback controls"></td>
  </tr>
  <tr>
    <td align="center"><b>Player</b> — artwork, lyrics, queue and playback in one view</td>
    <td align="center"><b>Browse</b> — your music, shelves and navigation</td>
  </tr>
  <tr>
    <td colspan="2"><img src="docs/readme/screenshot-3.png" alt="BitChord interface and playback" width="100%"></td>
  </tr>
  <tr>
    <td colspan="2" align="center"><b>Desktop experience</b> — native UI, fast playback, and quality controls</td>
  </tr>
</table>

## Features

**Playback**
- **YouTube Music, natively** — search, browse, albums, playlists, artist pages, radio, mixes, charts and your library.
- **Lossless / Hi-Res Lossless / Dolby Atmos** — play higher-quality copies from configured hi-res addon sources when a matching copy is available, with YouTube audio as the fallback.
- **Quality controls in the player** — see the active audio quality, prefer Dolby Atmos when available, and revert or upgrade quality from the player menu.
- **Automix and crossfade** — DJ-style transitions between songs, with a mixing state shown in the player during transitions.
- **Queue and playback controls** — shuffle, repeat, autoplay, song radio, next-track prefetch, volume normalization, and output device selection.
- **Downloads** — save songs for offline playback.

**Lyrics and interface**
- **Synced lyrics** — timed lyrics with provider fallbacks, timing controls, romanization and translation options.
- **Native Rust desktop UI** — a fast egui/rodio interface with system media controls, tray integration, and keyboard-friendly navigation.
- **Discord Rich Presence** — show what you’re listening to on Discord.
- **Last.fm scrobbling** — connect your Last.fm account in Settings.

**Accounts and library**
- **Google sign-in** — unlock your YouTube Music feed, likes, playlists, history and recommendations.
- **Library and uploads** — access saved music and uploaded tracks where available.
- **Listen Together / jams** — start or join a shared listening session.
- **Spotify transfer** — bring playlists and liked music over from Spotify.

## Installation

Download the latest build from [Releases](https://github.com/wenujacodes/BitChord-macOS/releases/latest) when available, or build it yourself (below).

The app is signed ad hoc (there's no Developer ID), so macOS asks before its first launch: right-click `bitchord.app` and choose **Open**, or run `xattr -dr com.apple.quarantine bitchord.app`.

## Building from source

You need the [Rust toolchain](https://rustup.rs/).

```sh
git clone https://github.com/wenujacodes/BitChord-macOS.git
cd BitChord-macOS
cargo build --release
macos/bundle.sh target/release/bitchord 2.0.0 out
```

This makes `out/bitchord.app` for your Mac's architecture (Apple Silicon or Intel).

## Notes

- Audio and the main interface are native (Rust + egui + rodio). Sign-in and some web flows may open a system webview window.
- Hi-res/Lossless/Atmos playback depends on configured addon sources and whether a matching, playable copy exists for the selected track.
- BitChord does not bypass subscriptions, DRM, or regional restrictions. Availability depends on the services and sources you use.
- Third-party services (YouTube Music, lyrics providers, Discord, Last.fm, Spotify, addon sources) can change behavior over time.

## Contributing

Bug reports and pull requests are welcome. Please open an issue with clear reproduction steps, platform details, and relevant logs (with secrets/tokens removed).

## License

BitChord is licensed under [GPL-3.0](LICENSE).

## Credits

BitChord was created by **[jackychan4891](https://github.com/jackychan4891)**, and this project is based on their work in [BitChord-PC-Continued](https://github.com/jackychan4891/BitChord-PC-Continued). Thanks to them and to all contributors of the original project.

This copy is distributed under the same GPL-3.0 license, as the license requires. The original copyright notices are kept.

## Acknowledgements

- [egui](https://github.com/emilk/egui) for the interface.
- [rodio](https://github.com/RustAudio/rodio) and Symphonia for audio playback and decoding.
- Lyrics, metadata, playback, and integrations depend on the upstream libraries and third-party services used by this project.
