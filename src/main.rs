mod app;
mod radio;
mod links;
mod likes;
mod timing;
mod audio;
mod cli;
mod config;
mod detach;
mod i18n;
mod library;
mod login;
mod model;
mod net;
mod player;
mod service;
mod session;
mod sync;
#[cfg(test)]
mod testing;
mod ui;
mod yandex;

use std::io::{self, stdout, Write};
use std::time::Duration;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use crossterm::{
    event::{self, Event, EnableBracketedPaste, DisableBracketedPaste},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen, SetTitle},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use app::App;
use audio::AudioPlayer;
use config::Config;
use i18n::Lang;
use player::{Player, SimPlayer};
use yandex::YandexService;

/// xterm "push/pop window title": keeps the user's own title to restore on exit.
const PUSH_TITLE: &str = "\x1b[22;0t";
const POP_TITLE: &str = "\x1b[23;0t";
const RESET_BACKGROUND: &str = "\x1b]111\x1b\\";
static BACKGROUND_CHANGED: AtomicBool = AtomicBool::new(false);

fn background_sequence(color: ratatui::style::Color) -> Option<String> {
    match color {
        ratatui::style::Color::Rgb(r, g, b) => Some(format!("\x1b]11;rgb:{r:02x}/{g:02x}/{b:02x}\x1b\\")),
        _ => None,
    }
}

fn reset_background(out: &mut impl Write, changed: &AtomicBool) -> io::Result<()> {
    if changed.swap(false, Ordering::AcqRel) { out.write_all(RESET_BACKGROUND.as_bytes())?; }
    Ok(())
}


fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(stdout(), DisableBracketedPaste, LeaveAlternateScreen, crossterm::cursor::Show);
    let _ = reset_background(&mut stdout(), &BACKGROUND_CHANGED);
    let _ = write!(stdout(), "{POP_TITLE}");
    let _ = stdout().flush();
}

/// Signs in with the saved key, or asks for one (first run, or the saved key
/// stopped working). Network problems are reported, not papered over.
fn connect(lang: Lang, force_login: bool) -> Result<YandexService> {
    if force_login {
        return login::login(lang, None);
    }
    let Some(token) = config::token() else {
        return login::login(lang, None);
    };
    eprintln!("yamusiq: подключаюсь к Яндекс Музыке…");
    match YandexService::connect(&token) {
        Ok(service) => Ok(service),
        Err(e) if YandexService::is_auth_error(&e) => {
            let reason = match lang {
                Lang::Ru => "Сохранённый ключ больше не принимается (истёк или отозван) — нужен новый.",
                Lang::En => "The saved key is no longer accepted (expired or revoked) — a new one is needed.",
            };
            login::login(lang, Some(reason))
        }
        Err(e) => Err(e.context("нет связи с Яндекс Музыкой")),
    }
}

fn open_library() -> library::Library {
    let Some(root) = library::Library::default_root() else { return library::Library::disabled() };
    if let Some(old) = library::Library::migrate_legacy(&root) {
        eprintln!("yamusiq: медиатека перенесена из {} в {}", old.display(), root.display());
    }
    library::Library::open(root)
}

fn main() {
    if let Err(e) = real_main() {
        eprintln!("yamusiq: {e:#}");
        std::process::exit(1);
    }
}

fn real_main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |f: &str| args.iter().any(|a| a == f);
    if let Some(bad) = args.iter().find(|a| !cli::FLAGS.contains(&a.as_str())) {
        eprintln!("yamusiq: неизвестный аргумент {bad}\n\n{}", cli::USAGE);
        std::process::exit(2);
    }
    if has("--help") || has("-h") {
        print!("{}", cli::USAGE);
        return Ok(());
    }
    if has("--version") || has("-V") {
        println!("yamusiq {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if has("--download-all") && has("--dont-download-all") {
        eprintln!("yamusiq: --download-all и --dont-download-all взаимоисключающие");
        std::process::exit(2);
    }

    let mut config = Config::load();
    let lang = config.resolve();
    if has("--logout") {
        let had = config::delete_token()?;
        println!("{}", if had { "Ключ удалён. При следующем запуске yamusiq попросит войти." } else { "Сохранённого ключа не было." });
        if std::env::var_os("YAMUSIQ_TOKEN").is_some() {
            println!("Переменная YAMUSIQ_TOKEN всё ещё задана — удалите её, чтобы выйти полностью.");
        }
        return Ok(());
    }

    // Mode switches are remembered in the config.
    if has("--download-all") || has("--dont-download-all") {
        let on = has("--download-all");
        config.auto_download = Some(on);
        config.save()?;
        eprintln!("yamusiq: режим «скачивать каждый включённый трек» {}", if on { "включён" } else { "выключен" });
    }

    // One-shot actions run without the player and exit.
    let actions = ["--remove-all-downloaded", "--download-fav", "--download-to-music"];
    if actions.iter().any(|a| has(a)) {
        let mut lib = open_library();
        if !lib.enabled() {
            anyhow::bail!("не удалось определить папку медиатеки");
        }
        if has("--remove-all-downloaded") {
            cli::remove_all(&mut lib, has("--yes") || has("-y"))?;
        }
        if has("--download-fav") {
            let service = connect(lang, has("--login"))?;
            cli::download_fav(&service, &mut lib, config.quality())?;
        }
        if has("--download-to-music") {
            cli::export_to_music(&lib)?;
        }
        return Ok(());
    }

    let service = connect(lang, has("--login"))?;
    let (player, problem): (Box<dyn Player>, Option<String>) = match AudioPlayer::new() {
        Ok(p) => (Box::new(p), None),
        // Browsing still works without an audio device; playback keeps time only.
        Err(e) => (Box::new(SimPlayer::default()), Some(format!("{e:#}"))),
    };
    let mut library = open_library();
    // Downloads run without a controlling terminal (so the title shows
    // yamusiq, not ffmpeg); the app stops them itself on exit.
    library.set_detached(true);

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_hook(info);
    }));

    write!(stdout(), "{PUSH_TITLE}")?;
    let result = (|| {
        enable_raw_mode()?;
        execute!(stdout(), EnterAlternateScreen, EnableBracketedPaste)?;
        if let Some(sequence) = background_sequence(app::PALETTE.bg) {
            // Default terminal background also colours terminal-owned padding
            // where OSC 11 is supported. Mark before writing so partial setup
            // failures still attempt the profile-background reset.
            BACKGROUND_CHANGED.store(true, Ordering::Release);
            write!(stdout(), "{sequence}")?;
            stdout().flush()?;
        }
        let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
        run(&mut terminal, Box::new(service), player, library, problem)
    })();
    restore_terminal();
    result
}

fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    service: Box<dyn service::MusicService>,
    player: Box<dyn Player>,
    library: library::Library,
    problem: Option<String>,
) -> Result<()> {
    let config = Config::load();
    let lang = config.resolve();
    let mut app = App::new(service, player, lang);
    app.quality_pref = config.quality();
    app.auto_download = config.auto_download.unwrap_or(false);
    app.library = library;
    if let Some(session) = session::Session::load() {
        app.restore(session);
    }
    if let Some(problem) = problem {
        app.status = app::Status::AudioDown(problem);
    }
    let mut titled = None;
    while !app.should_quit {
        // Window title follows the interface language.
        if titled != Some(app.lang) {
            execute!(terminal.backend_mut(), SetTitle(app.lang.t().window_title))?;
            titled = Some(app.lang);
        }
        terminal.draw(|frame| ui::draw(frame, &app))?;
        if event::poll(Duration::from_millis(80))? {
            match event::read()? {
                Event::Key(key) => app.on_key(key),
                Event::Paste(text) => app.on_paste(&text),
                _ => {}
            }
        }
        app.tick();
    }
    // Remember where we were; a failed write must not spoil the exit.
    let _ = app.snapshot().save();
    let updated = Config::from_state(app.lang, app.quality_pref, app.auto_download);
    if updated != config {
        // Losing a preference is not worth failing the exit over.
        let _ = updated.save();
    }
    Ok(())
}

#[cfg(test)]
mod terminal_tests {
    use super::*;

    #[test]
    fn background_uses_the_same_rgb_as_the_ui() {
        assert_eq!(background_sequence(app::PALETTE.bg).unwrap(), "\x1b]11;rgb:11/11/13\x1b\\");
        assert!(background_sequence(ratatui::style::Color::Reset).is_none());
    }

    #[test]
    fn cleanup_resets_the_profile_background_only_once() {
        let changed = AtomicBool::new(true);
        let mut out = Vec::new();
        reset_background(&mut out, &changed).unwrap();
        reset_background(&mut out, &changed).unwrap();
        assert_eq!(out, RESET_BACKGROUND.as_bytes());
        assert!(!changed.load(Ordering::Acquire));
        let mut unchanged = Vec::new();
        reset_background(&mut unchanged, &AtomicBool::new(false)).unwrap();
        assert!(unchanged.is_empty(), "do not reset a background the app never changed");
    }
}
