//! First-run sign-in: explains where the Yandex Music token comes from,
//! opens the official sign-in page, takes the token (or the whole address-bar
//! link containing it) without echoing it, checks it and stores it privately.

use std::io::{IsTerminal, Write};
use std::process::{Command, Stdio};

use anyhow::{bail, Result};

use crate::config;
use crate::i18n::Lang;
use crate::yandex::YandexService;

/// Yandex OAuth for the Yandex Music app (implicit flow). After sign-in the
/// browser lands on `music.yandex.ru/#access_token=…`.
pub const AUTH_URL: &str = "https://oauth.yandex.ru/authorize?response_type=token&client_id=23cabbbdc6cd418abb4b39c32c41195d";

const ATTEMPTS: usize = 5;

/// Accepts the bare token or any text containing `access_token=…` (the whole
/// address-bar link). Returns `None` for input that can't be a token.
pub fn parse_token(input: &str) -> Option<String> {
    let s = input.trim().trim_matches(|c| c == '"' || c == '\'');
    let raw = match s.find("access_token=") {
        Some(i) => s[i + "access_token=".len()..].split(['&', '#', ' ', '\n']).next().unwrap_or(""),
        None => s,
    };
    let ok = raw.len() >= 20 && raw.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    ok.then(|| raw.to_string())
}

struct Texts {
    title: &'static str,
    intro: &'static str,
    steps: &'static [&'static str],
    privacy: &'static str,
    open_q: &'static str,
    opened: &'static str,
    manual: &'static str,
    prompt: &'static str,
    not_token: &'static str,
    rejected: &'static str,
    cancelled: &'static str,
    ok: &'static str,
    env_note: &'static str,
    no_tty: &'static str,
}

const RU: Texts = Texts {
    title: "yamusiq · вход в Яндекс Музыку",
    intro: "Чтобы слушать музыку, yamusiq нужен ваш ключ доступа Яндекс Музыки (OAuth-токен).",
    steps: &[
        "1. Откройте страницу входа Яндекса для приложения Яндекс Музыки (ссылка ниже).",
        "2. Войдите в аккаунт. Браузер перейдёт на music.yandex.ru, а в адресной строке",
        "   будет ссылка вида  https://music.yandex.ru/#access_token=y0_…&token_type=bearer…",
        "3. Скопируйте всю эту ссылку (или только ключ после access_token=) и вставьте сюда.",
        "   Если страница успела смениться и ключа в адресе нет — откройте ссылку ещё раз.",
    ],
    privacy: "Ключ хранится только на этом компьютере: {path} (доступ только у вас).\nСменить аккаунт — yamusiq --login, забыть ключ — yamusiq --logout.",
    open_q: "Открыть страницу входа в браузере? [Д/н] ",
    opened: "Страница открыта в браузере.",
    manual: "Откройте вручную:",
    prompt: "Вставьте ссылку или ключ (ввод скрыт, Enter — готово, пусто — отмена): ",
    not_token: "Не похоже на ключ: нужна ссылка с access_token=… или сам ключ. Попробуйте ещё раз.",
    rejected: "Яндекс не принял ключ. Возможно, скопирована не вся ссылка или ключ устарел — попробуйте ещё раз.",
    cancelled: "Вход отменён.",
    ok: "✓ Вход выполнен: {name}. Ключ сохранён.",
    env_note: "Внимание: задана переменная YAMUSIQ_TOKEN — она важнее сохранённого ключа. Обновите или удалите её.",
    no_tty: "Нужен ключ Яндекс Музыки, а спросить его негде (нет терминала).\nПолучите ключ здесь: {url}\nи положите в {path} или в переменную YAMUSIQ_TOKEN.",
};

const EN: Texts = Texts {
    title: "yamusiq · sign in to Yandex Music",
    intro: "To play music, yamusiq needs your Yandex Music access key (an OAuth token).",
    steps: &[
        "1. Open the Yandex sign-in page for the Yandex Music app (link below).",
        "2. Sign in. The browser lands on music.yandex.ru with an address like",
        "   https://music.yandex.ru/#access_token=y0_…&token_type=bearer…",
        "3. Copy that whole address (or just the key after access_token=) and paste it here.",
        "   If the page moved on and the key is gone from the address, open the link again.",
    ],
    privacy: "The key stays on this computer: {path} (readable only by you).\nSwitch account: yamusiq --login; forget the key: yamusiq --logout.",
    open_q: "Open the sign-in page in your browser? [Y/n] ",
    opened: "Opened in your browser.",
    manual: "Open it manually:",
    prompt: "Paste the address or the key (hidden; Enter to finish, empty to cancel): ",
    not_token: "That doesn't look like a key: paste the address with access_token=… or the key itself.",
    rejected: "Yandex did not accept the key. Maybe the address was cut short or the key is old — try again.",
    cancelled: "Sign-in cancelled.",
    ok: "✓ Signed in: {name}. Key saved.",
    env_note: "Note: YAMUSIQ_TOKEN is set and takes precedence over the saved key. Update or unset it.",
    no_tty: "A Yandex Music key is needed but there is no terminal to ask in.\nGet a key here: {url}\nand put it in {path} or the YAMUSIQ_TOKEN variable.",
};

fn texts(lang: Lang) -> &'static Texts {
    match lang {
        Lang::Ru => &RU,
        Lang::En => &EN,
    }
}

fn open_in_browser(url: &str) -> bool {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    Command::new(opener).arg(url).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

/// Interactive sign-in. `reason` explains why it is shown again (e.g. the
/// saved key stopped working). Returns a connected service.
pub fn login(lang: Lang, reason: Option<&str>) -> Result<YandexService> {
    let t = texts(lang);
    let path = config::token_path().map_or_else(|| "~/.config/yamusiq/token".into(), |p| p.display().to_string());
    if !std::io::stdin().is_terminal() {
        bail!("{}", t.no_tty.replace("{url}", AUTH_URL).replace("{path}", &path));
    }

    let mut out = std::io::stderr();
    writeln!(out, "\n  {}\n", t.title)?;
    if let Some(r) = reason {
        writeln!(out, "  {r}\n")?;
    }
    writeln!(out, "  {}\n", t.intro)?;
    for line in t.steps {
        writeln!(out, "  {line}")?;
    }
    writeln!(out, "\n  {AUTH_URL}\n")?;
    for line in t.privacy.replace("{path}", &path).lines() {
        writeln!(out, "  {line}")?;
    }
    writeln!(out)?;

    write!(out, "  {}", t.open_q)?;
    out.flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    let no = matches!(answer.trim().to_lowercase().as_str(), "n" | "no" | "н" | "нет");
    if !no && open_in_browser(AUTH_URL) {
        writeln!(out, "  {}\n", t.opened)?;
    } else {
        writeln!(out, "  {} {AUTH_URL}\n", t.manual)?;
    }

    for _ in 0..ATTEMPTS {
        let input = rpassword::prompt_password(format!("  {}", t.prompt))?;
        if input.trim().is_empty() {
            bail!("{}", t.cancelled);
        }
        let Some(token) = parse_token(&input) else {
            writeln!(out, "  {}", t.not_token)?;
            continue;
        };
        match YandexService::connect(&token) {
            Ok(service) => {
                config::save_token(&token)?;
                writeln!(out, "  {}", t.ok.replace("{name}", crate::service::MusicService::name(&service)))?;
                if std::env::var_os("YAMUSIQ_TOKEN").is_some() {
                    writeln!(out, "  {}", t.env_note)?;
                }
                return Ok(service);
            }
            Err(e) if YandexService::is_auth_error(&e) => {
                writeln!(out, "  {}", t.rejected)?;
            }
            Err(e) => return Err(e),
        }
    }
    bail!("{}", t.cancelled)
}

#[cfg(test)]
mod tests {
    use super::parse_token;

    #[test]
    fn accepts_bare_token_or_address_bar_link() {
        let key = "y0_AgAAAAAAAbcDEF-ghij_KLMNopqrstuvwxyz0123456789";
        assert_eq!(parse_token(key).as_deref(), Some(key));
        assert_eq!(parse_token(&format!("  {key}\n")).as_deref(), Some(key));
        let link = format!("https://music.yandex.ru/#access_token={key}&token_type=bearer&expires_in=31535645");
        assert_eq!(parse_token(&link).as_deref(), Some(key));
        assert_eq!(parse_token(&format!("\"{link}\"")).as_deref(), Some(key));
        assert_eq!(parse_token("https://music.yandex.ru/home"), None, "link without a key");
        assert_eq!(parse_token("hello world"), None);
        assert_eq!(parse_token("short"), None);
    }
}
