//! Command-line actions that run without the TUI.

use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Result};

use crate::library::Library;
use crate::model::Quality;
use crate::service::MusicService;
use crate::sync::Sync;

pub const USAGE: &str = "yamusiq — Яндекс Музыка в терминале

Запуск:
  yamusiq                          открыть плеер (при первом запуске — вход в Яндекс Музыку)
  yamusiq --login                  войти заново / сменить аккаунт, затем открыть плеер
  yamusiq --logout                 забыть сохранённый ключ
  yamusiq --version                показать версию

Режим скачивания (запоминается):
  --download-all                   подкачивать на устройство каждый включённый трек
  --dont-download-all              выключить этот режим

Действия (выполняются без плеера и завершаются):
  --download-fav                   синхронизировать Избранное: скачать всё, чего ещё нет
  --download-to-music              собрать медиатеку скачанного в ~/Music/yamusiq
  --remove-all-downloaded [--yes]  удалить всё скачанное (с подтверждением)

Скачанное хранится в ~/Library/yamusiq (YAMUSIQ_LIBRARY — другое место).
";

/// Flags yamusiq understands.
pub const FLAGS: [&str; 13] = [
    "--login",
    "--logout",
    "--version",
    "--download-all",
    "--dont-download-all",
    "--download-fav",
    "--download-to-music",
    "--remove-all-downloaded",
    "--yes",
    "-y",
    "--help",
    "-h",
    "-V",
];

fn gb(bytes: u64) -> String {
    format!("{:.2} ГБ", bytes as f64 / 1e9)
}

/// `--remove-all-downloaded`: asks before deleting unless `yes`.
pub fn remove_all(lib: &mut Library, yes: bool) -> Result<()> {
    let (count, bytes) = lib.stats();
    if count == 0 {
        println!("Скачанного нет — удалять нечего.");
        return Ok(());
    }
    if !yes {
        if !std::io::stdin().is_terminal() {
            bail!("для удаления без подтверждения добавьте --yes");
        }
        print!("Удалить все скачанные треки: {count} шт., {}? [y/N] ", gb(bytes));
        std::io::stdout().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim().to_lowercase().as_str(), "y" | "yes" | "д" | "да") {
            println!("Отменено.");
            return Ok(());
        }
    }
    let (n, freed) = lib.remove_all()?;
    println!("Удалено треков: {n}, освобождено {}.", gb(freed));
    Ok(())
}

/// `--download-to-music`: browsable copy of the library in ~/Music/yamusiq.
pub fn export_to_music(lib: &Library) -> Result<()> {
    let Some(home) = std::env::var_os("HOME") else { bail!("не найден HOME") };
    let dest = PathBuf::from(home).join("Music/yamusiq");
    let (count, _) = lib.stats();
    if count == 0 {
        println!("Скачанного нет — сначала сохраните треки (D, ⌥⇧D или --download-fav).");
        return Ok(());
    }
    let (added, skipped) = lib.export_to(&dest)?;
    println!("Медиатека в {}: добавлено {added}, уже было {skipped}.", dest.display());
    Ok(())
}

/// `--download-fav`: favourites sync with a progress bar in the terminal.
pub fn download_fav(service: &dyn MusicService, lib: &mut Library, quality: Quality) -> Result<()> {
    eprintln!("Загружаю список Избранного…");
    let mut sync = Sync::new(service.likes_all()?, lib);
    eprintln!("Избранное: уже скачано {}, к загрузке {}.", sync.already, sync.total);
    if sync.total == 0 {
        return Ok(());
    }
    let tty = std::io::stderr().is_terminal();
    let mut last_printed = usize::MAX;
    loop {
        sync.pump(lib, service, quality);
        for done in lib.poll() {
            sync.on_finished(&done);
        }
        let frac = sync.fraction(lib);
        if tty {
            let width = 30;
            let filled = (frac * width as f64).round() as usize;
            eprint!(
                "\r⇣ {}/{}  {}{} {:>3.0}%  ошибок: {}  ",
                sync.finished_count(),
                sync.total,
                "━".repeat(filled),
                "─".repeat(width - filled),
                frac * 100.0,
                sync.failed
            );
        } else if sync.finished_count() != last_printed {
            last_printed = sync.finished_count();
            eprintln!("⇣ {}/{}", last_printed, sync.total);
        }
        if sync.is_done() {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    if tty {
        eprintln!();
    }
    println!("Готово: сохранено {}, ошибок {}.", sync.saved, sync.failed);
    Ok(())
}
