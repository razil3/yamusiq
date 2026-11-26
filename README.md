# yamusiq

Русская версия: [README.ru.md](README.ru.md).

![yamusiq — the player in the terminal](docs/preview-en.png)

Keyboard-first Yandex Music terminal client for macOS arm64, built with Rust and ratatui.

## Current status

Fullscreen TUI for your Yandex Music account. It covers search, playlists, My Wave, likes (read / like / unlike), artist views (popular / all tracks), lyrics (synced LRC or plain text) and real playback in a selectable quality up to lossless FLAC. There is no offline demo: on first start yamusiq walks you through signing in.

Code map:

- `src/service.rs` — `MusicService` trait (search, playlists, My Wave, likes, cached, artist tracks, like/unlike, lyrics, stream URL);
- `src/login.rs` — first-run sign-in: instructions, opens the Yandex sign-in page, takes the key or the whole address-bar link without echoing it, verifies it and stores it with mode 600;
- `src/yandex/` — `YandexService` for `api.music.yandex.net`. This is the unofficial API the official apps use. Includes request signing for lyrics and direct stream links;
- `src/player.rs` — `Player` trait (play, pause, seek, analysis) and `SimPlayer`, a clock-only player used when there is no audio device and in tests;
- `src/audio/` — `AudioPlayer`: FFmpeg decodes the stream to PCM, cpal plays it, and the output callback taps what reaches the speakers. A zero-padded Hann FFT turns that into 48 log-spaced bands, a level and a triggered waveform;
- `src/model.rs` — tracks, playlists and lyrics, including an LRC parser (`[mm:ss.xx] line`) with plain-text fallback;
- `src/app.rs` — state, key handling, queue and ALL / LOOP / RAND logic, lyric follow/scroll state;
- `src/i18n.rs` — Russian (default) and English UI text, Russian plurals, ЙЦУКЕН→QWERTY hotkey mapping;
- `src/ui/` — rendering: `mod.rs` (layout and panels), `visualizer.rs` (visualiser and lyrics overlay), `parts.rs` (header, bands, meters, card);
- `src/testing/` — in-memory fake catalogue with original lyrics, compiled only for tests.

While a stream is loading the visualiser says "buffering", and with nothing playing it says "no signal". If the track has lyrics they rise line by line on the right, over the visualiser. Each line sits on the background colour only under its own glyphs, so the scene shows through between lines. Synced (LRC) lyrics follow the timestamps. Plain lyrics are spread over the track length. `[` / `]` scroll by hand, and the view returns to following playback 6 s after the last scroll.

Streams come from `/get-file-info` (the desktop app's endpoint, unencrypted `raw` transport), with the legacy MP3 `download-info` as a fallback. The track card shows the quality you are actually hearing.

Known limits: network calls run on the UI thread, so the screen pauses briefly while a list or stream loads. My Wave loads one batch of tracks.

## Run

```sh
cargo run              # open the player (first start: sign in)
cargo run -- --login   # sign in again / switch account, then open the player
cargo run -- --logout  # forget the saved key
cargo run -- --help    # all command-line options
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
| `s` / `S` | artist of the playing (or selected) track: popular tracks / all tracks |
| `o` | stream quality: best (FLAC lossless) → high (AAC 256) → normal (AAC 192) → data saver (HE-AAC 64); applies immediately and is remembered |
| `j` `k` / `↑` `↓`, `Tab`, `Enter` | move, switch focus, open / play |
| `Space`, `n`, `b` | play-pause, next, previous |
| `←` / `→` | seek −5 s / +5 s |
| `y` | show / hide lyrics |
| `[` / `]` | scroll lyrics manually |
| `L` | like / unlike |
| `D` | save the track to the local library; press again to delete the saved file |
| `⌥⇧D` | sync all favourites into the library (again to stop); progress bar in the bottom line |
| `a` / `c` / `r` | mode: play all / loop track / random |
| `+` `-`, `v` | volume, visualiser motion on/off |
| `i` | interface language RU ↔ EN |
| `?` / `h` / `F1` | help overlay |
| `q` / `Esc` / `Ctrl-C` | quit |

Hotkeys also work with the Russian keyboard layout active (e.g. `а` = `f`, `ц` = `w`, `х` = `[`); search input is taken verbatim, case-insensitive, with «ё» = «е».

Local library: `D` saves a track as a tagged audio file under `~/Library/yamusiq/<Artist>/<Artist> — <Title>.<ext>`. It is deliberately kept out of `~/Music`; set `YAMUSIQ_LIBRARY` to change the location. A library from earlier versions in `~/Music/yamusiq` is moved there on first start. The file is saved in the selected quality and remuxed without re-encoding, so FLAC stays bit-exact. Saved tracks show `⇣` next to the title, appear under "Saved" (`d`) and play from disk, which is instant and works offline. The index is `library.json` in the library folder. Lossless favourites take about 25 MB per track on average, so check the size before a full sync of a large Favourites list.

While running, yamusiq sets the window title to "yamusiq · музыка в консоли" (or "yamusiq · music in the console" in English) and restores your previous title on exit. FFmpeg runs as an orphan process: it is not a descendant of yamusiq and has no controlling terminal, so Terminal does not list it ("yamusiq ▸ ffmpeg"). yamusiq stops it itself. Terminal.app still adds the working folder, the process name and the window size by itself. To turn them off, open Settings → Profiles → Window (and Tab) → Title and untick "Working directory or document", "Active process name" and "Dimensions".

`⌥⇧D` arrives as Alt+Shift+D when the terminal sends Option as Meta (iTerm2: Profiles → Keys → "Option key: Esc+"). Without Meta, macOS sends `Î` on the US layout; yamusiq accepts that too.

On exit yamusiq also saves the session to `~/.config/yamusiq/session.json`: the open list with its tracks, the cursor, the queue, the current track and its position, the playback mode and the volume. On the next launch the same account picks up there. The track reopens paused at the same second with a fresh stream link, and `Space` plays it.

The language and stream quality are saved on exit to `~/.config/yamusiq/config.json` (or `$XDG_CONFIG_HOME/yamusiq/config.json`); `YAMUSIQ_LANG=ru|en` overrides it. The interface starts in Russian by default.

Colours are 24-bit: use a truecolor terminal (iTerm2, Ghostty, WezTerm, Kitty, Alacritty).

Tests: `cargo test`. Live API checks (need a token): `cargo test live_ -- --ignored --nocapture`. Text dump of the rendered screen: `cargo test dump_screen -- --ignored --nocapture` (`YAMUSIQ_DUMP_AT=<seconds>` picks the playback position).

## Install

Download the archive for your Mac and put the binary on your PATH:

```sh
curl -fsSL https://raw.githubusercontent.com/razil3/yamusiq/master/scripts/install.sh | sh
```

The script picks `arm64` or `x86_64`, verifies the archive against `SHA256SUMS`
and installs to `/usr/local/bin`, or `~/.local/bin` when the former is not
writable. Pin a release with `YAMUSIQ_VERSION=v1.0.0`. `ffmpeg` is still needed
for playback.

## Next

`0.2.0` moves network work off the UI thread, so lists and streams load behind a
`buffering` state instead of freezing the screen. `0.2.1` then makes My Wave
real: batches are pumped as the queue runs out, playback reports back to the
rotor station (started, finished, skipped, liked), and the wave moves forward
instead of looping over one batch.

## Platform

Development target: macOS on Apple Silicon (CoreAudio via cpal). FFmpeg is required for playback.
