# yamusiq 1.1.2

## Playback modes

- Renamed single-track repeat to **Loop track** / **Зациклить трек**.
- Added **Playlist loop** / **Повтор плейлиста**: playback returns to the first track after the last track in the current queue. Works with playlists, favourites and search results.
- **No loop** / **Без повтора** now stops playback at the end of the queue.
- Press `c` to cycle **No loop → Loop track → Playlist loop**. Press `a` for no loop or `r` for random order.
- Manual next/previous commands still change tracks when single-track repeat is enabled.
- Playback mode is saved between sessions.

## Playlist navigation

- Press **Shift+←** inside a playlist to return to the playlists list.
- The previously selected playlist stays highlighted, and playback continues uninterrupted.
- Added a back-navigation hint at the bottom-left of the track list.
- Back navigation also works after restoring a session, including sessions saved by earlier versions.

## Documentation

- Updated Russian and English help, keyboard shortcut documentation and preview images.

## Downloads

- `yamusiq-1.1.2-arm64.tar.gz` — Apple Silicon.
- `yamusiq-1.1.2-x86_64.tar.gz` — Intel.
- `yamusiq-1.1.2-universal.tar.gz` — both architectures in one binary.
- `SHA256SUMS` — checksums for all three archives.

Requires macOS and `ffmpeg` in `PATH`:

```sh
brew install ffmpeg
```

### Install or update

```sh
curl -fsSL https://raw.githubusercontent.com/razil3/yamusiq/master/scripts/install.sh | sh
```

### Verify downloaded archives

With all three archives and `SHA256SUMS` in the same directory:

```sh
shasum -a 256 -c SHA256SUMS
```

## Validation

- 83 tests passed; 7 ignored.
- Clippy passed with warnings denied.
- Release builds completed for arm64 and x86_64.
- Universal binary version checks passed under both architectures.
