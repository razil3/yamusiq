//! Interface localisation (RU default, EN fallback).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Ru,
    En,
}

impl Lang {
    pub fn id(self) -> &'static str {
        match self {
            Self::Ru => "ru",
            Self::En => "en",
        }
    }

    /// Accepts "ru", "en", and locale-style values such as "en_US.UTF-8".
    pub fn from_id(id: &str) -> Option<Self> {
        let id = id.to_lowercase();
        if id.starts_with("ru") {
            Some(Self::Ru)
        } else if id.starts_with("en") {
            Some(Self::En)
        } else {
            None
        }
    }

    pub fn toggle(self) -> Self {
        match self {
            Self::Ru => Self::En,
            Self::En => Self::Ru,
        }
    }

    pub fn t(self) -> &'static Texts {
        match self {
            Self::Ru => &RU,
            Self::En => &EN,
        }
    }

    /// Picks the grammatical form for `n`: `[one, few, many]`.
    pub fn plural(self, n: usize, forms: [&'static str; 3]) -> &'static str {
        match self {
            Self::Ru => {
                let (m10, m100) = (n % 10, n % 100);
                if m10 == 1 && m100 != 11 {
                    forms[0]
                } else if (2..=4).contains(&m10) && !(12..=14).contains(&m100) {
                    forms[1]
                } else {
                    forms[2]
                }
            }
            Self::En => {
                if n == 1 { forms[0] } else { forms[2] }
            }
        }
    }

    pub fn tracks(self, n: usize) -> String {
        format!("{n} {}", self.plural(n, self.t().tracks))
    }

    pub fn playlists(self, n: usize) -> String {
        format!("{n} {}", self.plural(n, self.t().playlists))
    }
}

/// Replaces each `{}` in `template` with the next argument.
pub fn fill(template: &str, args: &[&str]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut args = args.iter();
    let mut rest = template;
    while let Some(i) = rest.find("{}") {
        out.push_str(&rest[..i]);
        out.push_str(args.next().copied().unwrap_or(""));
        rest = &rest[i + 2..];
    }
    out.push_str(rest);
    out
}

pub struct Texts {
    pub lang_name: &'static str,
    pub sections: [&'static str; 5],
    pub modes: [&'static str; 3],
    pub states: [&'static str; 3],
    pub tracks: [&'static str; 3],
    pub playlists: [&'static str; 3],

    pub lbl_library: &'static str,
    pub lbl_visualizer: &'static str,
    pub lbl_bands: &'static str,
    pub bands: [&'static str; 6],
    pub lbl_queue: &'static str,
    pub lbl_liked: &'static str,
    pub lbl_cached: &'static str,
    pub lbl_signal: &'static str,
    pub lbl_info: &'static str,
    pub lbl_progress: &'static str,
    pub lbl_volume: &'static str,
    pub lbl_help: &'static str,
    pub info_keys: [&'static str; 6],
    pub tagline: &'static str,
    pub tbl_track: &'static str,
    pub tbl_playlist: &'static str,
    pub tbl_time: &'static str,

    pub no_signal: &'static str,
    pub lyrics_none: &'static str,
    pub lyrics_hidden: &'static str,
    pub lyrics_manual: &'static str,
    pub lyrics_plain: &'static str,

    pub list_overview: &'static str,
    pub list_wave: &'static str,
    pub list_likes: &'static str,
    pub list_cached: &'static str,
    pub list_playlists: &'static str,
    pub list_search: &'static str,
    pub list_playlist: &'static str,
    pub list_artist: &'static str,
    pub list_artist_all: &'static str,
    pub hint_list_artist: &'static str,

    pub hint_library: &'static str,
    pub hint_visualizer: &'static str,
    pub hint_list: &'static str,
    pub hint_list_playlists: &'static str,
    pub hint_bands: &'static str,
    pub hint_queue: &'static str,
    pub hint_liked: &'static str,
    pub hint_cached: &'static str,
    pub hint_signal: &'static str,
    pub hint_info: &'static str,
    pub hint_progress: &'static str,
    pub hint_volume: &'static str,
    pub hint_search: &'static str,
    pub hint_footer: &'static str,
    pub hint_help: &'static str,

    pub empty_list: [&'static str; 2],
    pub not_playing: &'static str,
    pub not_playing_hint: &'static str,
    pub search_prompt: &'static str,

    pub st_ready: &'static str,
    pub st_search_open: &'static str,
    pub st_search_cancelled: &'static str,
    pub st_mode: &'static str,
    pub st_home: &'static str,
    pub st_failed: &'static str,
    pub st_playing: &'static str,
    pub st_stopped: &'static str,
    pub st_paused: &'static str,
    pub st_resumed: &'static str,
    pub st_nothing_to_like: &'static str,
    pub st_like_failed: &'static str,
    pub st_liked: &'static str,
    pub st_unliked: &'static str,
    pub st_artist_need_track: &'static str,
    pub st_lang: &'static str,
    pub st_lyrics_on: &'static str,
    pub st_lyrics_off: &'static str,
    pub st_stream_failed: &'static str,
    pub st_session_restored: &'static str,
    pub st_sync_started: &'static str,
    pub st_sync_nothing: &'static str,
    pub st_sync_cancelled: &'static str,
    pub st_sync_done: &'static str,
    pub st_sync_failed: &'static str,
    pub auto_label: &'static str,
    pub st_saving: &'static str,
    pub st_saved: &'static str,
    pub st_save_failed: &'static str,
    pub st_save_error: &'static str,
    pub st_confirm_remove: &'static str,
    pub st_removed: &'static str,
    pub st_library_disabled: &'static str,
    pub no_audio_to_save: &'static str,
    pub file_saved: &'static str,
    pub file_saving: &'static str,
    pub from_disk: &'static str,
    pub lossless_missing: &'static str,
    pub st_quality: &'static str,
    pub qualities: [&'static str; 4],
    pub st_audio_down: &'static str,
    pub buffering: &'static str,
    pub window_title: &'static str,
    pub fft_label: &'static str,

    pub help: &'static [(&'static str, &'static str)],
}

pub static RU: Texts = Texts {
    lang_name: "Русский",
    sections: ["Главная", "Моя волна", "Плейлисты", "Избранное", "Сохранённое"],
    modes: ["ВСЁ", "ПОВТОР", "СЛУЧАЙНО"],
    states: ["ИГРАЕТ", "ПАУЗА", "СТОП"],
    tracks: ["трек", "трека", "треков"],
    playlists: ["плейлист", "плейлиста", "плейлистов"],

    lbl_library: "БИБЛИОТЕКА",
    lbl_visualizer: "ВИЗУАЛИЗАТОР",
    lbl_bands: "ЧАСТОТЫ",
    bands: ["32 Гц", "125 Гц", "500 Гц", "2 кГц", "8 кГц", "16 кГц"],
    lbl_queue: "ОЧЕРЕДЬ: {}",
    lbl_liked: "ИЗБРАННОЕ: {}%",
    lbl_cached: "СОХРАНЕНО: {}%",
    lbl_signal: "УРОВЕНЬ СИГНАЛА",
    lbl_info: "ДАННЫЕ ТРЕКА",
    lbl_progress: "ПРОГРЕСС ТРЕКА",
    lbl_volume: "ГРОМКОСТЬ: {}%",
    lbl_help: "СПРАВКА",
    info_keys: ["Альбом", "Длительность", "Качество", "Режим", "Статус", "Файл"],
    tagline: "музыка в консоли",
    tbl_track: "ИСПОЛНИТЕЛЬ — ТРЕК",
    tbl_playlist: "ПЛЕЙЛИСТ",
    tbl_time: "ВРЕМЯ",

    no_signal: "нет сигнала",
    lyrics_none: "без текста",
    lyrics_hidden: "текст скрыт · y",
    lyrics_manual: "ручная прокрутка",
    lyrics_plain: "текст без таймкодов",

    list_overview: "ОБЗОР",
    list_wave: "МОЯ ВОЛНА",
    list_likes: "ИЗБРАННОЕ",
    list_cached: "СОХРАНЁННОЕ",
    list_playlists: "ПЛЕЙЛИСТЫ",
    list_search: "ПОИСК: {}",
    list_playlist: "ПЛЕЙЛИСТ: {}",
    list_artist: "ИСПОЛНИТЕЛЬ: {} · популярное",
    list_artist_all: "ИСПОЛНИТЕЛЬ: {} · все треки",
    hint_list_artist: "↑↓ выбор · ⏎ играть · s популярное · S все треки",

    hint_library: "↑↓ · ⏎ открыть",
    hint_visualizer: "y текст · [ ] прокрутка · v анимация",
    hint_list: "↑↓ выбор · ⏎ играть · D сохранить · f поиск",
    hint_list_playlists: "↑↓ выбор · ⏎ открыть · f поиск",
    hint_bands: "v анимация",
    hint_queue: "n/b трек",
    hint_liked: "L ♥",
    hint_cached: "D сохранить · d все",
    hint_signal: "v анимация",
    hint_info: "L ♥ · o качество · a/c/r",
    hint_progress: "␣ пауза · ←/→ перемотка · n/b трек",
    hint_volume: "+/− громкость",
    hint_search: "⏎ найти · Esc отмена · ⌫ стереть",
    hint_footer: "? справка · Tab фокус · y текст · i язык · q выход",
    hint_help: "? / Esc закрыть",

    empty_list: ["Выберите раздел в библиотеке (⏎)", "или нажмите f для поиска"],
    not_playing: "Ничего не играет",
    not_playing_hint: "Выберите трек и нажмите ⏎",
    search_prompt: "ПОИСК ›",

    st_ready: "Готово · ? — справка",
    st_search_open: "Введите запрос",
    st_search_cancelled: "Поиск отменён",
    st_mode: "Режим воспроизведения: {}",
    st_home: "Главная",
    st_failed: "{}: ошибка — {}",
    st_playing: "▶ {} — {}",
    st_stopped: "Остановлено",
    st_paused: "Пауза",
    st_resumed: "Продолжаем",
    st_nothing_to_like: "Нечего добавлять в избранное",
    st_like_failed: "Не удалось изменить избранное: {}",
    st_liked: "♥ В избранном: {}",
    st_unliked: "♡ Убрано из избранного: {}",
    st_artist_need_track: "Сначала включите или выберите трек",
    st_lang: "Язык: {}",
    st_lyrics_on: "Текст песни показан",
    st_lyrics_off: "Текст песни скрыт",
    st_stream_failed: "Поток недоступен: {}",
    st_session_restored: "Продолжим с «{}» — ␣ чтобы играть",
    st_sync_started: "⇣ Синхронизация Избранного: {} к загрузке, {} уже сохранено · ⌥⇧D — стоп",
    st_sync_nothing: "Избранное уже синхронизировано ({} треков)",
    st_sync_cancelled: "Синхронизация остановлена — текущие загрузки завершатся",
    st_sync_done: "⇣ Синхронизация завершена: сохранено {}, ошибок {}",
    st_sync_failed: "Синхронизация: {}",
    auto_label: "⇣ авто",
    st_saving: "⇣ Сохраняю «{}»…",
    st_saved: "⇣ Сохранено в медиатеку: {}",
    st_save_failed: "Не удалось сохранить «{}»",
    st_save_error: "Сохранение: {}",
    st_confirm_remove: "«{}» уже в медиатеке — D ещё раз, чтобы удалить файл",
    st_removed: "Удалено из медиатеки: {}",
    st_library_disabled: "Медиатека недоступна",
    no_audio_to_save: "у трека нет доступного аудио",
    file_saved: "сохранён",
    file_saving: "загрузка {}%",
    from_disk: "с диска",
    lossless_missing: "lossless недоступен",
    st_quality: "Качество: {}",
    qualities: ["лучшее (lossless)", "высокое (AAC 256)", "обычное (AAC 192)", "экономное (HE-AAC 64)"],
    st_audio_down: "Звук недоступен ({}) — можно смотреть, но не слушать",
    buffering: "буферизация",
    window_title: "yamusiq · музыка в консоли",
    fft_label: "спектр",

    help: &[
        ("f", "поиск по трекам, артистам, альбомам"),
        ("w  p  l  d", "волна · плейлисты · избранное · сохранённое"),
        ("D", "сохранить трек в медиатеку; ещё раз — удалить файл"),
        ("⌥⇧D", "синхронизировать всё Избранное (ещё раз — стоп)"),
        ("↑↓ / j k", "перемещение по списку"),
        ("Tab", "переключить фокус: библиотека ↔ список"),
        ("⏎", "открыть раздел / играть трек"),
        ("␣", "пауза / продолжить"),
        ("n  b", "следующий / предыдущий трек"),
        ("←  →", "перемотка на 5 секунд"),
        ("L", "добавить в избранное / убрать"),
        ("s  S", "артист трека: популярное · все треки"),
        ("a  c  r", "режим: все · повтор трека · случайно"),
        ("+  −", "громкость"),
        ("o", "качество: лучшее → высокое → обычное → экономное"),
        ("y", "показать / скрыть текст песни"),
        ("[  ]", "прокрутить текст вручную"),
        ("v  i", "анимация визуализатора · язык RU/EN"),
        ("q / Ctrl-C", "выход"),
        ("", "Клавиши работают и в русской раскладке"),
    ],
};

pub static EN: Texts = Texts {
    lang_name: "English",
    sections: ["Home", "My Wave", "Playlists", "Likes", "Saved"],
    modes: ["ALL", "LOOP", "RANDOM"],
    states: ["PLAYING", "PAUSED", "STOPPED"],
    tracks: ["track", "tracks", "tracks"],
    playlists: ["playlist", "playlists", "playlists"],

    lbl_library: "LIBRARY",
    lbl_visualizer: "VISUALIZER",
    lbl_bands: "FREQUENCIES",
    bands: ["32 Hz", "125 Hz", "500 Hz", "2 kHz", "8 kHz", "16 kHz"],
    lbl_queue: "QUEUE: {}",
    lbl_liked: "LIKED: {}%",
    lbl_cached: "SAVED: {}%",
    lbl_signal: "SIGNAL LEVEL",
    lbl_info: "TRACK DATA",
    lbl_progress: "TRACK PROGRESS",
    lbl_volume: "VOLUME: {}%",
    lbl_help: "HELP",
    info_keys: ["Album", "Duration", "Quality", "Mode", "Status", "File"],
    tagline: "music in the console",
    tbl_track: "ARTIST — TITLE",
    tbl_playlist: "PLAYLIST",
    tbl_time: "TIME",

    no_signal: "no signal",
    lyrics_none: "no lyrics",
    lyrics_hidden: "lyrics hidden · y",
    lyrics_manual: "manual scroll",
    lyrics_plain: "unsynced lyrics",

    list_overview: "OVERVIEW",
    list_wave: "MY WAVE",
    list_likes: "LIKES",
    list_cached: "SAVED",
    list_playlists: "PLAYLISTS",
    list_search: "SEARCH: {}",
    list_playlist: "PLAYLIST: {}",
    list_artist: "ARTIST: {} · popular",
    list_artist_all: "ARTIST: {} · all tracks",
    hint_list_artist: "↑↓ move · ⏎ play · s popular · S all tracks",

    hint_library: "↑↓ · ⏎ open",
    hint_visualizer: "y lyrics · [ ] scroll · v motion",
    hint_list: "↑↓ move · ⏎ play · D save · f search",
    hint_list_playlists: "↑↓ move · ⏎ open · f search",
    hint_bands: "v motion",
    hint_queue: "n/b skip",
    hint_liked: "L ♥",
    hint_cached: "D save · d all",
    hint_signal: "v motion",
    hint_info: "L ♥ · o quality · a/c/r",
    hint_progress: "␣ pause · ←/→ seek · n/b skip",
    hint_volume: "+/− volume",
    hint_search: "⏎ search · Esc cancel · ⌫ erase",
    hint_footer: "? help · Tab focus · y lyrics · i lang · q quit",
    hint_help: "? / Esc close",

    empty_list: ["Pick a section in the library (⏎)", "or press f to search"],
    not_playing: "Nothing playing",
    not_playing_hint: "Pick a track and press ⏎",
    search_prompt: "SEARCH ›",

    st_ready: "Ready · ? for help",
    st_search_open: "Type a query",
    st_search_cancelled: "Search cancelled",
    st_mode: "Playback mode: {}",
    st_home: "Home",
    st_failed: "{}: failed — {}",
    st_playing: "▶ {} — {}",
    st_stopped: "Stopped",
    st_paused: "Paused",
    st_resumed: "Resumed",
    st_nothing_to_like: "Nothing to like",
    st_like_failed: "Like failed: {}",
    st_liked: "♥ Liked: {}",
    st_unliked: "♡ Unliked: {}",
    st_artist_need_track: "Play or select a track first",
    st_lang: "Language: {}",
    st_lyrics_on: "Lyrics shown",
    st_lyrics_off: "Lyrics hidden",
    st_stream_failed: "Stream unavailable: {}",
    st_session_restored: "Picking up «{}» — ␣ to play",
    st_sync_started: "⇣ Syncing favourites: {} to fetch, {} already saved · ⌥⇧D to stop",
    st_sync_nothing: "Favourites already synced ({} tracks)",
    st_sync_cancelled: "Sync stopped — running downloads will finish",
    st_sync_done: "⇣ Sync finished: {} saved, {} failed",
    st_sync_failed: "Sync: {}",
    auto_label: "⇣ auto",
    st_saving: "⇣ Saving «{}»…",
    st_saved: "⇣ Saved to library: {}",
    st_save_failed: "Could not save «{}»",
    st_save_error: "Save: {}",
    st_confirm_remove: "«{}» is in the library — D again to delete the file",
    st_removed: "Removed from library: {}",
    st_library_disabled: "Library unavailable",
    no_audio_to_save: "no audio available for this track",
    file_saved: "saved",
    file_saving: "downloading {}%",
    from_disk: "from disk",
    lossless_missing: "lossless unavailable",
    st_quality: "Quality: {}",
    qualities: ["best (lossless)", "high (AAC 256)", "normal (AAC 192)", "data saver (HE-AAC 64)"],
    st_audio_down: "No audio ({}) — browsing only",
    buffering: "buffering",
    window_title: "yamusiq · music in the console",
    fft_label: "spectrum",

    help: &[
        ("f", "search tracks, artists, albums"),
        ("w  p  l  d", "wave · playlists · likes · saved"),
        ("D", "save track to the library; again — delete the file"),
        ("⌥⇧D", "sync all favourites (again — stop)"),
        ("↑↓ / j k", "move in list"),
        ("Tab", "switch focus: library ↔ list"),
        ("⏎", "open section / play track"),
        ("␣", "pause / resume"),
        ("n  b", "next / previous track"),
        ("←  →", "seek 5 seconds"),
        ("L", "like / unlike"),
        ("s  S", "track's artist: popular · all tracks"),
        ("a  c  r", "mode: all · loop track · random"),
        ("+  −", "volume"),
        ("o", "quality: best → high → normal → data saver"),
        ("y", "show / hide lyrics"),
        ("[  ]", "scroll lyrics manually"),
        ("v  i", "visualizer motion · language RU/EN"),
        ("q / Ctrl-C", "quit"),
        ("", "Keys also work in the Russian layout"),
    ],
};

/// Maps a character typed in the Russian ЙЦУКЕН layout to the key in the
/// same physical position on QWERTY, so hotkeys work without switching layout.
pub fn latin_key(c: char) -> char {
    const RU: &str = "йцукенгшщзхъфывапролджэячсмитьбюё";
    const EN: &str = "qwertyuiop[]asdfghjkl;'zxcvbnm,.`";
    let lower = c.to_lowercase().next().unwrap_or(c);
    match RU.chars().position(|r| r == lower) {
        Some(i) => {
            let l = EN.chars().nth(i).unwrap_or(c);
            if c.is_uppercase() { l.to_ascii_uppercase() } else { l }
        }
        None => c,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn russian_plurals() {
        let l = Lang::Ru;
        assert_eq!(l.tracks(1), "1 трек");
        assert_eq!(l.tracks(3), "3 трека");
        assert_eq!(l.tracks(5), "5 треков");
        assert_eq!(l.tracks(11), "11 треков");
        assert_eq!(l.tracks(12), "12 треков");
        assert_eq!(l.tracks(21), "21 трек");
        assert_eq!(l.tracks(24), "24 трека");
        assert_eq!(l.tracks(0), "0 треков");
        assert_eq!(Lang::En.tracks(1), "1 track");
        assert_eq!(Lang::En.tracks(2), "2 tracks");
    }

    #[test]
    fn fill_replaces_in_order() {
        assert_eq!(fill("{} — {}", &["a", "b"]), "a — b");
        assert_eq!(fill("x {} y", &[]), "x  y");
    }

    #[test]
    fn russian_layout_maps_to_qwerty() {
        assert_eq!(latin_key('а'), 'f');
        assert_eq!(latin_key('ц'), 'w');
        assert_eq!(latin_key('Д'), 'L');
        assert_eq!(latin_key('й'), 'q');
        assert_eq!(latin_key('ш'), 'i');
        assert_eq!(latin_key('н'), 'y');
        assert_eq!(latin_key('х'), '[');
        assert_eq!(latin_key('ъ'), ']');
        assert_eq!(latin_key('f'), 'f');
        assert_eq!(latin_key('+'), '+');
    }

    #[test]
    fn both_languages_have_same_help_size() {
        assert_eq!(RU.help.len(), EN.help.len());
    }
}
