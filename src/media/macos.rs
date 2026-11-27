//! Public MediaPlayer APIs, with a windowless AppKit event pump on the main
//! thread. No event tap, key interception, Accessibility permission or GUI.
use std::ptr::NonNull;
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
    mpsc::{self, Receiver, SyncSender},
};
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2::{
    AnyThread, MainThreadMarker,
    rc::{Retained, autoreleasepool},
    runtime::AnyObject,
};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBitmapImageRep, NSColorSpace, NSEventMask,
    NSImage,
};
use objc2_foundation::{
    NSData, NSDate, NSDefaultRunLoopMode, NSDictionary, NSNumber, NSSize, NSString,
};
use objc2_media_player::{
    MPChangePlaybackPositionCommandEvent, MPMediaItemArtwork, MPMediaItemPropertyAlbumTitle,
    MPMediaItemPropertyArtist, MPMediaItemPropertyArtwork, MPMediaItemPropertyPlaybackDuration,
    MPMediaItemPropertyTitle, MPNowPlayingInfoCenter, MPNowPlayingInfoPropertyDefaultPlaybackRate,
    MPNowPlayingInfoPropertyElapsedPlaybackTime, MPNowPlayingInfoPropertyPlaybackRate,
    MPNowPlayingPlaybackState, MPRemoteCommand, MPRemoteCommandCenter, MPRemoteCommandEvent,
    MPRemoteCommandHandlerStatus,
};

use super::{Command, Publication, Snapshot, State, artwork};

fn make_artwork(bytes: &[u8]) -> Option<Retained<MPMediaItemArtwork>> {
    let data = NSData::with_bytes(bytes);
    let image = NSImage::initWithData(NSImage::alloc(), &data)?;
    let reps = image.representations();
    if (0..reps.count()).any(|i| {
        let r = reps.objectAtIndex(i);
        r.pixelsWide() > 1200 || r.pixelsHigh() > 1200
    }) || !image.isValid()
    {
        return None;
    }
    let size = image.size();
    if !size.width.is_finite()
        || !size.height.is_finite()
        || size.width <= 0.0
        || size.height <= 0.0
        || size.width > 1200.0
        || size.height > 1200.0
    {
        return None;
    }
    // The copied block keeps an immutable image alive. No I/O or app mutation.
    let handler = RcBlock::new(move |_: NSSize| NonNull::from(image.as_ref()));
    // SAFETY: the block owns the retained image and returns a valid +0 pointer.
    Some(unsafe {
        MPMediaItemArtwork::initWithBoundsSize_requestHandler(
            MPMediaItemArtwork::alloc(),
            size,
            &handler,
        )
    })
}

pub(super) fn make_preview(bytes: &[u8]) -> Option<Arc<crate::cover::Preview>> {
    let bitmap = NSBitmapImageRep::imageRepWithData(&NSData::with_bytes(bytes))?;
    let width = bitmap.pixelsWide();
    let height = bitmap.pixelsHigh();
    if width <= 0 || height <= 0 || width > 1200 || height > 1200 {
        return None;
    }
    const SIDE: usize = crate::cover::SIDE;
    let space = NSColorSpace::sRGBColorSpace();
    let mut pixels = [0u8; SIDE * SIDE];
    for y in 0..SIDE {
        for x in 0..SIDE {
            let mut luma = 0.0;
            for dy in 0..2 {
                for dx in 0..2 {
                    let px = (((x * 4 + dx * 2 + 1) as isize * width) / (SIDE * 4) as isize).min(width - 1);
                    let py = (((y * 4 + dy * 2 + 1) as isize * height) / (SIDE * 4) as isize).min(height - 1);
                    let color = bitmap.colorAtX_y(px, py)?.colorUsingColorSpace(&space)?;
                    luma += (0.2126 * color.redComponent()
                        + 0.7152 * color.greenComponent()
                        + 0.0722 * color.blueComponent())
                        * color.alphaComponent();
                }
            }
            pixels[y * SIDE + x] =
                (luma * 255.0 / 4.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    Some(Arc::new(crate::cover::Preview::from_luma(pixels)))
}

const COMMANDS: [Command; 6] = [
    Command::Play,
    Command::Pause,
    Command::Toggle,
    Command::Previous,
    Command::Next,
    Command::Seek(Duration::ZERO),
];

fn queue_command(
    kind: Command,
    tx: &SyncSender<Command>,
    available: &AtomicU8,
) -> MPRemoteCommandHandlerStatus {
    if available.load(Ordering::Acquire) & kind.bit() == 0 {
        return MPRemoteCommandHandlerStatus::NoSuchContent;
    }
    if tx.try_send(kind).is_ok() {
        MPRemoteCommandHandlerStatus::Success
    } else {
        MPRemoteCommandHandlerStatus::CommandFailed
    }
}

struct Binding {
    command: Retained<MPRemoteCommand>,
    target: Retained<AnyObject>,
    kind: Command,
}

impl Binding {
    fn new(
        command: Retained<MPRemoteCommand>,
        kind: Command,
        tx: SyncSender<Command>,
        available: Arc<AtomicU8>,
    ) -> Self {
        let handler = RcBlock::new(move |event: NonNull<MPRemoteCommandEvent>| {
            let kind = if matches!(kind, Command::Seek(_)) {
                // SAFETY: this handler belongs only to changePlaybackPositionCommand;
                // Apple supplies its corresponding event subclass for this command.
                let seconds = unsafe {
                    event
                        .cast::<MPChangePlaybackPositionCommandEvent>()
                        .as_ref()
                        .positionTime()
                };
                let Ok(position) = Duration::try_from_secs_f64(seconds) else {
                    return MPRemoteCommandHandlerStatus::CommandFailed;
                };
                Command::Seek(position)
            } else {
                kind
            };
            queue_command(kind, &tx, &available)
        });
        // SAFETY: the framework copies the block, which owns only Send data.
        // Keep its returned target alive and remove exactly that target on drop.
        let target = unsafe {
            command.setEnabled(false);
            command.addTargetWithHandler(&handler)
        };
        Self {
            command,
            target,
            kind,
        }
    }
}

impl Drop for Binding {
    fn drop(&mut self) {
        // SAFETY: this target was returned by this command's addTargetWithHandler.
        unsafe {
            self.command.setEnabled(false);
            self.command.removeTarget(Some(&self.target));
        }
    }
}

pub struct Controls {
    application: Retained<NSApplication>,
    center: Retained<MPNowPlayingInfoCenter>,
    bindings: Vec<Binding>,
    available: Arc<AtomicU8>,
    receiver: Receiver<Command>,
    publication: Publication,
    artwork_loader: artwork::Loader,
    artwork: Option<Retained<MPMediaItemArtwork>>,
    preview: Option<Arc<crate::cover::Preview>>,
}

impl Controls {
    pub fn new() -> Self {
        let mtm =
            MainThreadMarker::new().expect("media controls must start on the TUI main thread");
        autoreleasepool(|_| {
            let application = NSApplication::sharedApplication(mtm);
            application.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
            application.finishLaunching();
            let (tx, receiver) = mpsc::sync_channel(32);
            let available = Arc::new(AtomicU8::new(0));
            // SAFETY: public singleton APIs, used only on the main thread.
            let (center, commands) = unsafe {
                let commands = MPRemoteCommandCenter::sharedCommandCenter();
                (
                    MPNowPlayingInfoCenter::defaultCenter(),
                    [
                        commands.playCommand(),
                        commands.pauseCommand(),
                        commands.togglePlayPauseCommand(),
                        commands.previousTrackCommand(),
                        commands.nextTrackCommand(),
                        commands.changePlaybackPositionCommand().into_super(),
                    ],
                )
            };
            let bindings = commands
                .into_iter()
                .zip(COMMANDS)
                .map(|(command, kind)| Binding::new(command, kind, tx.clone(), available.clone()))
                .collect();
            Self {
                application,
                center,
                bindings,
                available,
                receiver,
                publication: Publication::default(),
                artwork_loader: artwork::Loader::new(),
                artwork: None,
                preview: None,
            }
        })
    }

    pub fn poll(&self) -> Vec<Command> {
        autoreleasepool(|_| {
            // Non-blocking, bounded pump; crossterm still owns terminal input.
            // Running AppKit's default mode also services main-queue callbacks.
            let past = NSDate::distantPast();
            for _ in 0..32 {
                // SAFETY: Foundation's immutable exported default-mode constant.
                let mode = unsafe { NSDefaultRunLoopMode };
                let Some(event) = self
                    .application
                    .nextEventMatchingMask_untilDate_inMode_dequeue(
                        NSEventMask::Any,
                        Some(&past),
                        mode,
                        true,
                    )
                else {
                    break;
                };
                self.application.sendEvent(&event);
            }
        });
        self.receiver.try_iter().collect()
    }

    pub fn preview(&self) -> Option<Arc<crate::cover::Preview>> {
        self.preview.clone()
    }

    pub fn update(&mut self, snapshot: &Snapshot) {
        let changed = self
            .artwork_loader
            .request(snapshot.track.as_ref().and_then(|t| t.cover_uri.as_deref()));
        if changed {
            self.artwork = None;
            self.preview = None;
        }
        let loaded = self.artwork_loader.poll();
        let refreshed = loaded.is_some();
        if let Some(bytes) = loaded {
            autoreleasepool(|_| {
                self.preview = bytes.as_ref().and_then(|bytes| make_preview(bytes));
                self.artwork = bytes.and_then(|bytes| make_artwork(&bytes));
            });
        }
        if changed || refreshed {
            self.publication.last = None;
        }

        if !self.publication.due(snapshot, Instant::now()) {
            return;
        }
        autoreleasepool(|_| {
            let mut flags = 0;
            for binding in &self.bindings {
                let enabled = snapshot.accepts(binding.kind);
                if enabled {
                    flags |= binding.kind.bit();
                }
                // SAFETY: owned command; no Objective-C object crosses threads.
                unsafe {
                    binding.command.setEnabled(enabled);
                }
            }
            self.available.store(flags, Ordering::Release);
            // SAFETY: dictionary keys are Apple's NSString constants; each value
            // is an NSString or NSNumber, retained through the copying setter.
            unsafe {
                if let Some(track) = &snapshot.track {
                    let title = NSString::from_str(&track.title);
                    let artist = NSString::from_str(&track.artist);
                    let album = NSString::from_str(&track.album);
                    let duration = NSNumber::new_f64(track.duration.as_secs_f64());
                    let elapsed =
                        NSNumber::new_f64(snapshot.position.min(track.duration).as_secs_f64());
                    let rate = NSNumber::new_f64(if snapshot.state == State::Playing {
                        1.0
                    } else {
                        0.0
                    });
                    let default_rate = NSNumber::new_f64(1.0);
                    let mut keys = vec![
                        MPMediaItemPropertyTitle,
                        MPMediaItemPropertyArtist,
                        MPMediaItemPropertyAlbumTitle,
                        MPMediaItemPropertyPlaybackDuration,
                        MPNowPlayingInfoPropertyElapsedPlaybackTime,
                        MPNowPlayingInfoPropertyPlaybackRate,
                        MPNowPlayingInfoPropertyDefaultPlaybackRate,
                    ];
                    let mut values: Vec<&AnyObject> = vec![
                        title.as_ref(),
                        artist.as_ref(),
                        album.as_ref(),
                        duration.as_ref(),
                        elapsed.as_ref(),
                        rate.as_ref(),
                        default_rate.as_ref(),
                    ];
                    if let Some(artwork) = &self.artwork {
                        keys.push(MPMediaItemPropertyArtwork);
                        values.push(artwork.as_ref());
                    }
                    let info = NSDictionary::from_slices(&keys, &values);
                    self.center.setNowPlayingInfo(Some(&info));
                } else {
                    self.center.setNowPlayingInfo(None);
                }
                self.center.setPlaybackState(match snapshot.state {
                    State::Playing => MPNowPlayingPlaybackState::Playing,
                    State::Paused => MPNowPlayingPlaybackState::Paused,
                    State::Stopped => MPNowPlayingPlaybackState::Stopped,
                });
            }
        });
    }
}

impl Drop for Controls {
    fn drop(&mut self) {
        self.available.store(0, Ordering::Release);
        // Remove callbacks before the receiver disappears, including on unwind.
        self.bindings.clear();
        autoreleasepool(|_| {
            // SAFETY: clear only this process's singleton, never another app.
            unsafe {
                self.center
                    .setPlaybackState(MPNowPlayingPlaybackState::Stopped);
                self.center.setNowPlayingInfo(None);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_bitmap_produces_a_separate_monochrome_preview() {
        autoreleasepool(|_| {
            let preview = make_preview(include_bytes!("../../assets/macos/AppIcon.png")).unwrap();
            for y in 0..16 {
                for x in 0..16 {
                    assert!((16..=224).contains(&preview.shade(x, y, 16)));
                }
            }
            assert!(make_preview(b"not an image").is_none());
        });
    }

    #[test]
    fn native_artwork_retains_its_image_and_rejects_invalid_data() {
        autoreleasepool(|_| {
            assert!(make_artwork(b"not an image").is_none());
            let artwork = make_artwork(include_bytes!("../../assets/macos/AppIcon.png")).unwrap();
            // SAFETY: owned native object; this exercises the copied image-provider block.
            let image = unsafe { artwork.imageWithSize(NSSize::new(128.0, 128.0)) }.unwrap();
            assert!(image.isValid());
            assert!(image.size().width > 0.0 && image.size().height > 0.0);
        });
    }

    #[test]
    fn callbacks_enqueue_commands_in_order_without_blocking() {
        let (tx, rx) = mpsc::sync_channel(COMMANDS.len());
        let available = AtomicU8::new(u8::MAX);
        for command in COMMANDS {
            assert_eq!(
                queue_command(command, &tx, &available),
                MPRemoteCommandHandlerStatus::Success
            );
        }
        assert_eq!(
            queue_command(Command::Play, &tx, &available),
            MPRemoteCommandHandlerStatus::CommandFailed
        );
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), COMMANDS);
        drop(rx);
        assert_eq!(
            queue_command(Command::Play, &tx, &available),
            MPRemoteCommandHandlerStatus::CommandFailed
        );
    }

    #[test]
    fn disabled_commands_do_not_reach_the_player() {
        let (tx, rx) = mpsc::sync_channel(1);
        let available = AtomicU8::new(0);
        for command in COMMANDS {
            assert_eq!(
                queue_command(command, &tx, &available),
                MPRemoteCommandHandlerStatus::NoSuchContent
            );
        }
        assert!(rx.try_recv().is_err());
    }
}
