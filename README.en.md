# yamusiq

[![macOS Apple Silicon](docs/badges/macos-apple-silicon.svg)](https://github.com/razil3/yamusiq/releases/latest)
[![macOS Intel](docs/badges/macos-intel.svg)](https://github.com/razil3/yamusiq/releases/latest)
[![Rust](docs/badges/rust.svg)](https://www.rust-lang.org/)
[![crates.io](docs/badges/cargo.svg)](https://crates.io/crates/yamusiq)

[Русский](README.md) · **Version 1.3.0**

Keyboard-first Yandex Music terminal client for macOS on Apple Silicon and Intel.

![yamusiq — the player in the terminal](docs/preview_ru.png)

## Quick install

Requires macOS 11.0 or later (Apple Silicon / Intel) and FFmpeg for playback.

```sh
brew install ffmpeg
curl -fsSL https://raw.githubusercontent.com/razil3/yamusiq/master/scripts/install.sh | sh
yamusiq
```

On first start, yamusiq walks you through signing in to your account.

The installer selects your Mac's architecture, checks `SHA256SUMS` and verifies
the app signature. It installs `yamusiq.app` in `~/Applications` and the `yamusiq`
command in `/usr/local/bin` if writable, otherwise in `~/.local/bin`. If the
command is not found, add that directory to your `PATH`:

```sh
export PATH="$HOME/.local/bin:$PATH"
```

The command opens the player in the current terminal. Double-clicking the app
opens Terminal with the player; yamusiq has no separate Dock icon. Keep the
bundle intact — it supplies the app's identity and system media icon. Rerun the
installer to update. Override install directories with `YAMUSIQ_APP_DIR` and
`YAMUSIQ_BIN_DIR`, or pin a release with `YAMUSIQ_VERSION=vX.Y.Z`.
Bundles have a local ad-hoc signature, without Apple notarization. Installation
through `curl` normally needs no manual signing; browser downloads may require
Gatekeeper approval.

## Run and controls

```sh
yamusiq             # open the player
yamusiq --login     # sign in again / switch account
yamusiq --logout    # forget the saved key
yamusiq --help      # all command-line options
```

Command-line options:

| Option | What it does |
| --- | --- |
| `--download-all` / `--dont-download-all` | Turn on/off (and remember) the mode where every track that starts playing is also saved locally; shown as `⇣ авто` in the footer |
| `--download-fav` | Sync favourites without the UI: download every liked track not yet saved, with a progress bar, then exit (Ctrl-C stops; partial files are cleaned up on the next start) |
| `--download-to-music` | Build a browsable copy of the saved tracks in `~/Music/yamusiq` (`<Artist>/<Artist> — <Title>`). It uses hard links, so no extra space; repeated runs add only what is new |
| `--remove-all-downloaded` | Delete everything saved, after a confirmation; add `--yes` for scripts |

Requirements: `ffmpeg` in `PATH` (`brew install ffmpeg`) for playback.

### Signing in

yamusiq needs your Yandex Music access key (an OAuth token). The first time you run it, or whenever the saved key stops working, it asks for one before opening the player:

1. It offers to open the Yandex sign-in page for the Yandex Music app: <https://oauth.yandex.ru/authorize?response_type=token&client_id=23cabbbdc6cd418abb4b39c32c41195d>.
2. After you sign in, the browser lands on `https://music.yandex.ru/#access_token=…&token_type=bearer…`.
3. Paste that whole address, or just the key after `access_token=`. Input is hidden. yamusiq extracts the key and checks it with Yandex.
4. It saves the key to `~/.config/yamusiq/token`, readable only by you. Keys last about a year; when one expires, you will be asked again.

`YAMUSIQ_TOKEN` overrides the saved key. The key is never printed or logged. Without a terminal to ask in (for example in scripts), yamusiq prints where to get a key and exits.

Controls:

| Key | Action |
| --- | --- |
| `f` | search (type, `Enter` run, `Esc` cancel) |
| `p` / `w` / `l` / `d` | playlists / My Wave / likes / saved (local library) |
| `W` | start radio from the selected track (outside the list: the playing track) |
| `u` | copy the track’s public Yandex Music link to the macOS clipboard |
| `O` (Shift+o) | open a modal: paste a shared track link or ID, `Enter` to open and play, `Esc` to cancel |
| `s` / `S` | artist of the selected track (playing track only if no track row is selected): popular tracks / all tracks |
| `o` | stream quality: best (FLAC lossless) → high (AAC 256) → normal (AAC 192) → data saver (HE-AAC 64); applies immediately and is remembered |
| `j` `k` / `↑` `↓`, `Tab`, `Enter` | move, switch focus, open / play |
| `Space`, `n`, `b` | play-pause, next, previous |
| `←` / `→` | when Home cards have focus: select Deja Vu / Playlist of the Day / Stash; otherwise seek −5 s / +5 s |
| `y` | show / hide lyrics |
| `[` / `]` | scroll lyrics manually |
| `L` | like / unlike the selected track (playing track only if no track row is selected) |
| `D` | save the track to the local library; press again to delete the saved file |
| `⌥⇧D` | sync all favourites into the library (again to stop); progress bar in the bottom line |
| `a` | no loop: stop at the end of the queue |
| `c` | cycle: no loop → loop track → playlist loop (the entire queue) |
| `r` | random order |
| `Shift+←` | return from a playlist to its Home card or playlists list, or from an artist opened from Likes back to Likes; preserves selection and playback |
| `+` `-`, `v` | volume, visualiser motion on/off |
| `i` | interface language RU ↔ EN |
| `?` / `h` / `F1` | help overlay |
| `q` / `Esc` / `Ctrl-C` | quit |

Playlist loop repeats the entire playback queue, including favourites and search results. Manual next/previous commands still change tracks when single-track repeat is enabled.

Hotkeys also work with the Russian keyboard layout active (e.g. `а` = `f`, `ц` = `w`, `х` = `[`); search input is taken verbatim, case-insensitive, with «ё» = «е».

`Shift+L` targets the highlighted track in every list, falling back to the playing track only if no track row is selected. In Likes it toggles the heart without removing its row, reloading the list or resetting selection. The list refreshes when you explicitly reopen Likes. A failed request restores the heart in place.

Local library: `D` saves a track as a tagged audio file under `~/Library/yamusiq/<Artist>/<Artist> — <Title>.<ext>`. It is deliberately kept out of `~/Music`; set `YAMUSIQ_LIBRARY` to change the location. A library from earlier versions in `~/Music/yamusiq` is moved there on first start. The file is saved in the selected quality and remuxed without re-encoding, so FLAC stays bit-exact. Saved tracks show `⇣` next to the title, appear under "Saved" (`d`) and play from disk, which is instant and works offline. The index is `library.json` in the library folder. Lossless favourites take about 25 MB per track on average, so check the size before a full sync of a large Favourites list.

While running, yamusiq sets the window title to "yamusiq · музыка в консоли" (or "yamusiq · music in the console" in English) and restores your previous title on exit. FFmpeg runs as an orphan process: it is not a descendant of yamusiq and has no controlling terminal, so Terminal does not list it ("yamusiq ▸ ffmpeg"). yamusiq stops it itself. Terminal.app still adds the working folder, the process name and the window size by itself. To turn them off, open Settings → Profiles → Window (and Tab) → Title and untick "Working directory or document", "Active process name" and "Dimensions".

`⌥⇧D` arrives as Alt+Shift+D when the terminal sends Option as Meta (iTerm2: Profiles → Keys → "Option key: Esc+"). Without Meta, macOS sends `Î` on the US layout; yamusiq accepts that too.

On exit yamusiq also saves the session to `~/.config/yamusiq/session.json`: the open list with its tracks, the cursor, the queue, the current track and its position, the playback mode and the volume. On the next launch the same account picks up there. The track reopens paused at the same second with a fresh stream link, and `Space` plays it.

The language and stream quality are saved on exit to `~/.config/yamusiq/config.json` (or `$XDG_CONFIG_HOME/yamusiq/config.json`); `YAMUSIQ_LANG=ru|en` overrides it. The interface starts in Russian by default.

Colours are 24-bit: use a truecolor terminal (iTerm2, Ghostty, WezTerm, Kitty, Alacritty). On entering the player, yamusiq requests the same default terminal background (`#111113`, OSC 11) so terminal-owned padding can blend with the UI without changing content margins. On exit or panic it requests a reset to the terminal profile background (OSC 111). Support depends on the terminal; if the padding keeps its original colour, set the profile background to `#111113` manually. In macOS Terminal.app on Tahoe, set the profile background manually
to RGB **17, 17, 19**, with 100% opacity; automatic padding recolouring does not work.

## Features

Fullscreen TUI for your Yandex Music account. It covers search, playlists, My Wave, likes (read / like / unlike), artist views (popular / all tracks), lyrics (synced LRC or plain text) and real playback in a selectable quality up to lossless FLAC.

Home shows three personal playlists with monochrome covers and track counts. `←` / `→` select a card, `Enter` opens its tracks without autoplay, and `Shift+←` returns to the same card. A narrow panel shows only the selected card; very short cards omit the artwork.

While a stream is loading the visualiser says "buffering", and with nothing playing it says "no signal". If the track has lyrics they scroll over the centre of the visualiser, with centred rows and the current line anchored at the vertical centre. Each line sits on the background colour only under its own glyphs, so the scene shows through between lines. When lyrics are loading or unavailable, the track title is centred in the visualiser instead. Hiding lyrics with `y` leaves it free of text. The header always shows the current track between the brand and the right summary: a play/pause icon, the title, then elapsed time, a compact progress bar and duration. Long titles are truncated with an ellipsis. Playback shortcuts sit in the centre of the bottom line, with three-cell gaps on either side; narrow windows use a separate row. Synced (LRC) lyrics follow the timestamps. Plain lyrics are spread over the track length. `[` / `]` scroll by hand, and the view returns to following playback 6 s after the last scroll.

Streams come from `/get-file-info` (the desktop app's endpoint, unencrypted `raw` transport), with the legacy MP3 `download-info` as a fallback. The track card shows the quality you are actually hearing.

Track data shows a monochrome Braille cover with the title and metadata below it.
Long titles wrap. The app version appears only in the bottom-right footer.

### macOS system media controls

- Play / Pause, previous and next track, and seeking via system media controls and Touch Bar.
- Track title, artist, album, artwork and progress in Now Playing.
- Launch in the current terminal with `yamusiq` or in Terminal by double-clicking the `.app`.
- Launching the `.app` expands the window to the screen’s usable area without entering fullscreen.
- App icon in system media panels, with no separate Dock icon.

## Build from source

Requires Rust supporting edition 2024 and FFmpeg on your `PATH`:

```sh
git clone https://github.com/razil3/yamusiq.git
cd yamusiq
cargo run --release --locked
```

Once 1.3.0 is published on crates.io, Cargo installation is available:

```sh
cargo install yamusiq --version 1.3.0 --locked
```

Plain `cargo run` / `cargo install` support commands and seeking but not
the custom source icon. To build a bundle into a new output directory:

```sh
cargo build --release --locked
sh scripts/bundle.sh target/release/yamusiq dist/yamusiq.app
./dist/yamusiq.app/Contents/MacOS/yamusiq
# or: open dist/yamusiq.app
```

The deployment target is macOS 11.0 for both architectures. Its source of truth
is `.cargo/config.toml`; packaging checks it against `LSMinimumSystemVersion`
and every Mach-O slice. This checks build metadata, not actual playback on a
physical Mac running macOS 11.

Local builds and release CI use the same pipeline: `dist.sh` → `pack.sh` →
`bundle.sh`. Each archive contains `yamusiq.app` and a console launcher named
`yamusiq`. Run `./yamusiq` from an extracted archive, keeping the adjacent `.app`.

```sh
# Native bundle and archive smoke test in a temporary directory; no account/audio:
sh scripts/dist.sh --smoke-test
# All three archives; refuses to overwrite existing artifacts:
sh scripts/dist.sh --output dist/release-check
# Verify an individual bundle and safely invoke --version:
sh scripts/test-bundle.sh dist/yamusiq.app arm64 --run
```

Full builds require both Rust targets (`rustup target add aarch64-apple-darwin
x86_64-apple-darwin`). On Apple Silicon, Intel bundles receive static checks
without execution or requiring Rosetta; arm64 and universal also run `--version`.
Every archive is extracted again to verify plist values, architectures,
deployment targets, resource contents, execute bits, all-slice signatures and
the console launcher. `SHA256SUMS` is checked automatically. These scripts never
publish a release.

`bundle-smoke.yml` checks PRs and `master`: Rust tests, terminal window layout,
a native archive and rejection of deliberately damaged bundles. `release.yml`
runs on `v*` tags, requires the tag to match the Cargo version, and publishes
only verified arm64, x86_64 and universal archives. Release notes stay empty for
manual editing.

Tests: `cargo test --locked`; lint: `cargo clippy --locked --all-targets -- -D warnings`. Live API checks require a separate, deliberate invocation. Text dump of the rendered screen: `cargo test dump_screen -- --ignored --nocapture` (`YAMUSIQ_DUMP_AT=<seconds>` picks the playback position).

## Code map

- `src/service.rs` — `MusicService` trait (search, playlists, My Wave, likes, cached, artist tracks, like/unlike, lyrics, stream URL);
- `src/login.rs` — first-run sign-in: instructions, opens the Yandex sign-in page, takes the key or the whole address-bar link without echoing it, verifies it and stores it with mode 600;
- `src/yandex/` — `YandexService` for `api.music.yandex.net`. This is the unofficial API the official apps use. Includes request signing for lyrics and direct stream links;
- `src/player.rs` — `Player` trait (play, pause, seek, analysis) and `SimPlayer`, a clock-only player used when there is no audio device and in tests;
- `src/audio/` — `AudioPlayer`: FFmpeg decodes the stream to PCM, cpal plays it, and the output callback taps what reaches the speakers. A zero-padded Hann FFT turns that into 48 log-spaced bands, a level and a triggered waveform;
- `src/model.rs` — tracks, playlists and lyrics, including an LRC parser (`[mm:ss.xx] line`) with plain-text fallback;
- `src/app.rs` — state, key handling, queue, no-loop / track-loop / playlist-loop / random modes, lyric follow/scroll state;
- `src/net.rs` — six independent worker queues: playback, catalogue, radio, metadata, actions and downloads; superseded reads are skipped and long batched reads support cooperative cancellation;
- `src/timing.rs` — optional latency diagnostics containing stage names and durations only;
- `src/home.rs` — Home personal mixes: Deja Vu, Playlist of the Day, Stash;
- `src/radio.rs` — radio batches, feedback events and per-track batch metadata;
- `src/media.rs`, `src/media/macos.rs` — macOS system media commands and Now Playing metadata;
- `src/links.rs` — public track links, ID parsing and macOS clipboard integration;
- `src/likes.rs` — favourites page metadata and stable ordered ID snapshots;
- `src/i18n.rs` — Russian (default) and English UI text, Russian plurals, ЙЦУКЕН→QWERTY hotkey mapping;
- `src/ui/` — rendering: `mod.rs` (layout and panels), `visualizer.rs` (visualiser and lyrics overlay), `parts.rs` (header, bands, meters, card), `home.rs` (Home cards);
- `src/testing/` — in-memory fake catalogue with original lyrics, compiled only for tests.
