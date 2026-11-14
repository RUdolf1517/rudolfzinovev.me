//! Bilingual localization (RU + EN) and UTC date formatting.
//! Every UI string lives here as a static struct so rendering never allocates
//! translation strings.

pub use crate::models::Lang;

/// All user-visible chrome strings. The dictionary is deliberately complete for
/// both languages even where a given template does not use every key.
#[allow(dead_code)]
pub struct T {
    pub site_role: &'static str,
    pub nav_index: &'static str,
    pub nav_notes: &'static str,
    pub nav_projects: &'static str,
    pub nav_about: &'static str,
    pub nav_cabinet: &'static str,
    pub nav_rss: &'static str,
    pub skip_link: &'static str,
    pub lang_switch: &'static str,

    pub hero_kicker: &'static str,
    pub hero_cta_notes: &'static str,
    pub hero_cta_projects: &'static str,
    pub hero_status: &'static str,
    pub hero_local_time: &'static str,
    pub hero_render: &'static str,

    pub latests: &'static str,
    pub latests_sub: &'static str,
    pub all_articles: &'static str,
    pub all_projects: &'static str,
    pub empty_state: &'static str,
    pub read_min: &'static str,
    pub min_short: &'static str,
    pub published_on: &'static str,
    pub updated_on: &'static str,
    pub views_count: &'static str,
    pub featured: &'static str,
    pub field_kind: &'static str,
    pub kind_note: &'static str,
    pub kind_project: &'static str,
    pub notes_intro: &'static str,
    pub projects_intro: &'static str,
    pub project_meta: &'static str,
    pub tags: &'static str,
    pub all_tags: &'static str,
    pub filter_by_tag: &'static str,
    pub search_placeholder: &'static str,
    pub search_label: &'static str,
    pub search_results: &'static str,
    pub search_empty: &'static str,
    pub search_submit: &'static str,
    pub clear: &'static str,

    pub toc: &'static str,
    pub back_to_articles: &'static str,
    pub back_to_projects: &'static str,
    pub read_other_lang: &'static str,
    pub related: &'static str,
    pub prev_next: &'static str,
    pub article_meta: &'static str,

    pub about_title: &'static str,
    pub about_manifesto: &'static str,
    pub about_now: &'static str,
    pub about_contact: &'static str,
    pub about_stack: &'static str,
    pub about_stack_text: &'static str,
    pub about_license: &'static str,
    pub about_license_text: &'static str,

    pub not_found_title: &'static str,
    pub not_found_text: &'static str,
    pub not_found_cta: &'static str,
    pub server_error_title: &'static str,
    pub server_error_text: &'static str,

    pub footer_built: &'static str,
    pub footer_license: &'static str,
    pub footer_views: &'static str,
    pub footer_admin: &'static str,

    pub login_title: &'static str,
    pub login_user: &'static str,
    pub login_password: &'static str,
    pub login_submit: &'static str,
    pub login_error: &'static str,
    pub login_rate_limited: &'static str,
    pub login_hint: &'static str,

    pub cabinet: &'static str,
    pub cabinet_overview: &'static str,
    pub cabinet_articles: &'static str,
    pub cabinet_editor: &'static str,
    pub cabinet_profile: &'static str,
    pub cabinet_telemetry: &'static str,
    pub cabinet_security: &'static str,
    pub cabinet_backup: &'static str,
    pub cabinet_logout: &'static str,
    pub cabinet_owner: &'static str,
    pub cabinet_new_article: &'static str,
    pub cabinet_save: &'static str,
    pub cabinet_delete: &'static str,
    pub cabinet_confirm_delete: &'static str,
    pub cabinet_publish: &'static str,
    pub cabinet_unpublish: &'static str,
    pub cabinet_preview: &'static str,
    pub cabinet_saved: &'static str,
    pub cabinet_status: &'static str,
    pub cabinet_draft: &'static str,
    pub cabinet_lang_pair: &'static str,
    pub field_translation_key: &'static str,
    pub cabinet_sessions: &'static str,
    pub cabinet_revoke: &'static str,
    pub cabinet_revoke_others: &'static str,
    pub cabinet_change_password: &'static str,
    pub cabinet_current_password: &'static str,
    pub cabinet_new_password: &'static str,
    pub cabinet_repeat_password: &'static str,
    pub cabinet_password_mismatch: &'static str,
    pub cabinet_password_changed: &'static str,
    pub cabinet_wrong_password: &'static str,
    pub cabinet_export: &'static str,
    pub cabinet_export_hint: &'static str,
    pub cabinet_download: &'static str,
    pub cabinet_stats: &'static str,

    pub field_title: &'static str,
    pub field_slug: &'static str,
    pub field_summary: &'static str,
    pub field_body: &'static str,
    pub field_tags: &'static str,
    pub field_lang: &'static str,
    pub field_body_hint: &'static str,
    pub field_tags_hint: &'static str,
}

pub static RU: T = T {
    site_role: "инженер-программист",
    nav_index: "Индекс",
    nav_notes: "Заметки",
    nav_projects: "Проекты",
    nav_about: "О себе",
    nav_cabinet: "Кабинет",
    nav_rss: "RSS",
    skip_link: "Перейти к содержимому",
    lang_switch: "Сменить язык",

    hero_kicker: "инженер-программист",
    hero_cta_notes: "Читать заметки",
    hero_cta_projects: "Смотреть проекты",
    hero_status: "статус",
    hero_local_time: "время сборки страницы",
    hero_render: "рендер",

    latests: "Свежие записи",
    latests_sub: "Заметки о системах, коде и замерах.",
    all_articles: "Все заметки",
    all_projects: "Каталог проектов",
    projects_intro: "Вещи, которые можно запустить: серверы, утилиты и эксперименты, доведённые до рабочего состояния.",
    notes_intro: "Личные записи без редактора и дедлайна: системы, код, замеры.",
    project_meta: "проект",
    field_kind: "Тип записи",
    kind_note: "заметка",
    kind_project: "проект",
    empty_state: "Пока пусто. Первая запись появится из кабинета.",
    read_min: "мин чтения",
    min_short: "мин",
    published_on: "опубликовано",
    updated_on: "обновлено",
    views_count: "просмотров",
    featured: "избранное",
    tags: "теги",
    all_tags: "Все теги",
    filter_by_tag: "Фильтр по тегу",
    search_placeholder: "поиск по статьям…",
    search_label: "Поиск",
    search_results: "Результаты поиска",
    search_empty: "Ничего не найдено. Попробуйте другой запрос.",
    search_submit: "Найти",
    clear: "сбросить",

    toc: "Оглавление",
    back_to_articles: "← к списку заметок",
    back_to_projects: "← к каталогу проектов",
    read_other_lang: "Читать на английском",
    related: "Похожие записи",
    prev_next: "Навигация",
    article_meta: "метаданные",

    about_title: "О себе",
    about_manifesto: "Манифест",
    about_now: "Сейчас",
    about_contact: "Связь",
    about_stack: "Стек",
    about_stack_text: "Rust без внешних крейтов, системные libsqlite3 / libcrypto / libz, SSR-шаблоны, ноль внешних запросов из браузера.",
    about_license: "Лицензия",
    about_license_text: "RZ-NONCOMMERCIAL-1.0: запрещено любое коммерческое использование и монетизация.",

    not_found_title: "Страница не найдена",
    not_found_text: "Такого адреса нет. Возможно, страница переехала или ещё не написана.",
    not_found_cta: "На главную",
    server_error_title: "Что-то сломалось",
    server_error_text: "Внутренняя ошибка. Запись в журнале уже сделана.",

    footer_built: "собрано на Rust",
    footer_license: "некоммерческая лицензия",
    footer_views: "просмотров",
    footer_admin: "вход для владельца",

    login_title: "Вход в кабинет",
    login_user: "Пользователь",
    login_password: "Пароль",
    login_submit: "Войти",
    login_error: "Неверный логин или пароль.",
    login_rate_limited: "Слишком много попыток. Подождите минуту.",
    login_hint: "Доступ только для владельца сайта.",

    cabinet: "Кабинет",
    cabinet_overview: "Обзор",
    cabinet_articles: "Записи",
    cabinet_editor: "Редактор",
    cabinet_profile: "Профиль",
    cabinet_telemetry: "Телеметрия",
    cabinet_security: "Безопасность",
    cabinet_backup: "Бэкап",
    cabinet_logout: "Выйти",
    cabinet_owner: "владелец",
    cabinet_new_article: "Новая запись",
    cabinet_save: "Сохранить",
    cabinet_delete: "Удалить",
    cabinet_confirm_delete: "Удалить без возможности восстановления?",
    cabinet_publish: "Опубликовать",
    cabinet_unpublish: "В черновик",
    cabinet_preview: "Предпросмотр",
    cabinet_saved: "Сохранено.",
    cabinet_status: "Статус",
    cabinet_draft: "черновик",
    cabinet_lang_pair: "Языковая пара",
    field_translation_key: "Ключ перевода",
    cabinet_sessions: "Активные сессии",
    cabinet_revoke: "Завершить",
    cabinet_revoke_others: "Завершить все другие сессии",
    cabinet_change_password: "Смена пароля",
    cabinet_current_password: "Текущий пароль",
    cabinet_new_password: "Новый пароль",
    cabinet_repeat_password: "Повторите пароль",
    cabinet_password_mismatch: "Пароли не совпадают или слишком короткие (минимум 12 символов).",
    cabinet_password_changed: "Пароль изменён, другие сессии завершены.",
    cabinet_wrong_password: "Текущий пароль неверен.",
    cabinet_export: "Экспорт данных",
    cabinet_export_hint: "JSON-снимок статей, профиля и агрегатов телеметрии.",
    cabinet_download: "Скачать JSON",
    cabinet_stats: "Сводка",

    field_title: "Заголовок",
    field_slug: "Слаг (URL)",
    field_summary: "Краткое описание",
    field_body: "Markdown",
    field_tags: "Теги (через запятую)",
    field_lang: "Язык",
    field_body_hint: "# заголовок, ```code```, | таблицы |, > цитаты, списки, **акценты**",
    field_tags_hint: "rust, sqlite, http",
};

pub static EN: T = T {
    site_role: "software engineer",
    nav_index: "Index",
    nav_notes: "Notes",
    nav_projects: "Projects",
    nav_about: "About",
    nav_cabinet: "Cabinet",
    nav_rss: "RSS",
    skip_link: "Skip to content",
    lang_switch: "Switch language",

    hero_kicker: "software engineer",
    hero_cta_notes: "Read the notes",
    hero_cta_projects: "See the projects",
    hero_status: "status",
    hero_local_time: "page build time",
    hero_render: "render",

    latests: "Latest entries",
    latests_sub: "Notes on systems, code and measurements.",
    all_articles: "All notes",
    all_projects: "Project catalogue",
    projects_intro: "Things you can run: servers, utilities and experiments taken all the way to a working state.",
    notes_intro: "Personal entries without an editor or a deadline: systems, code, measurements.",
    project_meta: "project",
    field_kind: "Entry kind",
    kind_note: "note",
    kind_project: "project",
    empty_state: "Nothing here yet. The first entry comes from the cabinet.",
    read_min: "min read",
    min_short: "min",
    published_on: "published",
    updated_on: "updated",
    views_count: "views",
    featured: "featured",
    tags: "tags",
    all_tags: "All tags",
    filter_by_tag: "Filter by tag",
    search_placeholder: "search the archive…",
    search_label: "Search",
    search_results: "Search results",
    search_empty: "No matches. Try a different query.",
    search_submit: "Search",
    clear: "clear",

    toc: "Contents",
    back_to_articles: "← all notes",
    back_to_projects: "← project catalogue",
    read_other_lang: "Read in Russian",
    related: "Related entries",
    prev_next: "Navigation",
    article_meta: "metadata",

    about_title: "About",
    about_manifesto: "Manifesto",
    about_now: "Now",
    about_contact: "Contact",
    about_stack: "Stack",
    about_stack_text: "Rust with zero external crates, system libsqlite3 / libcrypto / libz, SSR templates, no third-party browser requests.",
    about_license: "License",
    about_license_text: "RZ-NONCOMMERCIAL-1.0: all commercial use and monetization is prohibited.",

    not_found_title: "Page not found",
    not_found_text: "No such address. The page may have moved or was never written.",
    not_found_cta: "Back to index",
    server_error_title: "Something broke",
    server_error_text: "Internal error. It has been written to the log.",

    footer_built: "built with Rust",
    footer_license: "non-commercial license",
    footer_views: "views",
    footer_admin: "owner sign-in",

    login_title: "Cabinet sign-in",
    login_user: "User",
    login_password: "Password",
    login_submit: "Sign in",
    login_error: "Wrong user or password.",
    login_rate_limited: "Too many attempts. Wait a minute.",
    login_hint: "Owner access only.",

    cabinet: "Cabinet",
    cabinet_overview: "Overview",
    cabinet_articles: "Entries",
    cabinet_editor: "Editor",
    cabinet_profile: "Profile",
    cabinet_telemetry: "Telemetry",
    cabinet_security: "Security",
    cabinet_backup: "Backup",
    cabinet_logout: "Sign out",
    cabinet_owner: "owner",
    cabinet_new_article: "New entry",
    cabinet_save: "Save",
    cabinet_delete: "Delete",
    cabinet_confirm_delete: "Delete permanently?",
    cabinet_publish: "Publish",
    cabinet_unpublish: "To draft",
    cabinet_preview: "Preview",
    cabinet_saved: "Saved.",
    cabinet_status: "Status",
    cabinet_draft: "draft",
    cabinet_lang_pair: "Language pair",
    field_translation_key: "Translation key",
    cabinet_sessions: "Active sessions",
    cabinet_revoke: "Revoke",
    cabinet_revoke_others: "Revoke all other sessions",
    cabinet_change_password: "Change password",
    cabinet_current_password: "Current password",
    cabinet_new_password: "New password",
    cabinet_repeat_password: "Repeat password",
    cabinet_password_mismatch: "Passwords differ or are too short (12 characters minimum).",
    cabinet_password_changed: "Password changed, other sessions were revoked.",
    cabinet_wrong_password: "Current password is wrong.",
    cabinet_export: "Data export",
    cabinet_export_hint: "JSON snapshot of articles, profile and telemetry aggregates.",
    cabinet_download: "Download JSON",
    cabinet_stats: "Summary",

    field_title: "Title",
    field_slug: "Slug (URL)",
    field_summary: "Summary",
    field_body: "Markdown",
    field_tags: "Tags (comma separated)",
    field_lang: "Language",
    field_body_hint: "# headings, ```code```, | tables |, > quotes, lists, **emphasis**",
    field_tags_hint: "rust, sqlite, http",
};

pub fn t(lang: Lang) -> &'static T {
    match lang {
        Lang::Ru => &RU,
        Lang::En => &EN,
    }
}

/// Convert Unix epoch seconds into `YYYY-MM-DD` (UTC, no chrono dependency).
pub fn format_date_utc(ts: i64) -> String {
    let (y, m, d, _, _, _) = split_utc(ts);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Convert Unix epoch seconds into `YYYY-MM-DD HH:MM UTC`.
pub fn format_datetime_utc(ts: i64) -> String {
    let (y, m, d, hh, mm, _) = split_utc(ts);
    format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02} UTC")
}

/// RFC-2822 UTC timestamp for RSS `<pubDate>`.
pub fn format_rfc2822_utc(ts: i64) -> String {
    let (y, m, d, hh, mm, ss) = split_utc(ts);
    let days = ts.div_euclid(86_400);
    let wd = (days + 4).rem_euclid(7) as usize;
    const WDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let mon = MONTHS[(m.saturating_sub(1) as usize).min(11)];
    format!("{}, {d:02} {mon} {y:04} {hh:02}:{mm:02}:{ss:02} +0000", WDAYS[wd])
}

pub fn split_utc(ts: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = ts.div_euclid(86_400);
    let secs_of_day = ts.rem_euclid(86_400) as u32;
    let hh = secs_of_day / 3600;
    let mm = (secs_of_day % 3600) / 60;
    let ss = secs_of_day % 60;

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (year, m, d, hh, mm, ss)
}

/// Russian plural form for a count: (1, 2-4, 5+) → "запись / записи / записей".
pub fn ru_plural<'a>(n: i64, one: &'a str, few: &'a str, many: &'a str) -> &'a str {
    let n = n.abs();
    let tail100 = n % 100;
    if (11..=14).contains(&tail100) {
        return many;
    }
    match n % 10 {
        1 => one,
        2..=4 => few,
        _ => many,
    }
}

pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_is_1970() {
        assert_eq!(format_date_utc(0), "1970-01-01");
        assert_eq!(format_date_utc(1_700_000_000), "2023-11-14");
        assert_eq!(format_rfc2822_utc(1_700_000_000), "Tue, 14 Nov 2023 22:13:20 +0000");
    }

    #[test]
    fn russian_plurals() {
        let f = |n| ru_plural(n, "запись", "записи", "записей");
        assert_eq!(f(1), "запись");
        assert_eq!(f(3), "записи");
        assert_eq!(f(5), "записей");
        assert_eq!(f(11), "записей");
        assert_eq!(f(21), "запись");
        assert_eq!(f(104), "записи");
    }

    #[test]
    fn dictionaries_are_complete() {
        assert_eq!(t(Lang::Ru).nav_notes, "Заметки");
        assert_eq!(t(Lang::En).nav_notes, "Notes");
        assert_eq!(t(Lang::Ru).nav_projects, "Проекты");
        assert_eq!(t(Lang::En).nav_projects, "Projects");
    }
}
