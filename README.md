# rudolfzinovev.me

> Персональный сайт и закрытый кабинет владельца. **Rust без единого внешнего крейта.**
> A personal site and owner-only cabinet. **Rust, zero external crates.**

[![tests](https://img.shields.io/badge/tests-26%20passing-3f6b34)](#тесты)
[![deps](https://img.shields.io/badge/dependencies-0-d9a04d)](#zero-crates)
[![binary](https://img.shields.io/badge/binary-910%20KB-191713)](#performance)
[![license](https://img.shields.io/badge/license-RZ--NONCOMMERCIAL--1.0-c2542d)](LICENSE)

---

## Что это / What it is

| | |
|---|---|
| **Сайт** | Каталог проектов + заметки с тегами, поиском и RSS; два языка (RU/EN), одна бумажная тема. |
| **Кабинет** | `/cabinet` — только для владельца: Markdown-редактор с черновиками и публикацией, профиль и манифест сайта, приватная телеметрия, сессии и смена пароля, JSON-бэкап. |
| **Стек** | Rust 2024, `std::net`, системные `libsqlite3` / `libcrypto` / `libz`. Ноль крейтов, ноль JS-сборки, ноль внешних запросов из браузера. |
| **Дизайн** | Архив, который читается как книга: колонтитул, каталожные номера, поля для заметок, тёплый бумажный фон. |
| **Размер** | 910 КБ бинарник (400 КБ gzip), 9.5 КБ gzip главная страница, 1.4 КБ gzip JavaScript. |
| **Развёртывание** | Один бинарник + один файл SQLite на любом Debian/Ubuntu VPS. |

---

## Zero crates

`Cargo.toml` содержит **пустую секцию `[dependencies]`**. Всё, что обычно
приходит из crates.io, реализовано в репозитории:

| Обычно берут крейт | Здесь |
|---|---|
| `axum` / `actix-web` / `hyper` | `src/http.rs` — HTTP/1.1: keep-alive, пул потоков, разбор запроса, cookies, gzip, ETag/304, rate limiter |
| `rusqlite` | `src/sqlite.rs` — тонкая обёртка над C API `libsqlite3` (`SQLITE_TRANSIENT`, WAL, busy_timeout) |
| `argon2` / `ring` / `sha2` | `src/crypto.rs` — FFI к OpenSSL `libcrypto`: `RAND_bytes`, `SHA256`, `PKCS5_PBKDF2_HMAC` |
| `flate2` | `src/crypto.rs` — `deflateInit2_` из `libz` (gzip, уровень 6) |
| `chrono` / `time` | `src/i18n.rs` — `civil_from_days` для UTC-дат и RFC-2822 |
| `pulldown-cmark` | `src/markdown.rs` — заголовки с якорями и TOC, код, таблицы, цитаты, списки, инлайн-разметка |
| `tera` / `askama` | `src/templates.rs` — SSR-шаблоны на `format!` + экранирование |
| `clap` | `src/main.rs` — разбор аргументов в 60 строк |

Почему так: сборка занимает **~11 секунд** с нуля, `cargo build` не требует
сети, supply-chain поверхность равна нулю, а бинарник линкуется только с тем,
что уже есть в любом дистрибутиве. Подробнее — [`ARCHITECTURE.md`](ARCHITECTURE.md).

---

## Быстрый старт / Quick start

```bash
# Debian/Ubuntu: системные библиотеки, с которыми линкуется бинарник
sudo apt install build-essential libsqlite3-dev libssl-dev zlib1g-dev

cargo run --release -- serve --bind 127.0.0.1:8080 --data ./data
# → http://127.0.0.1:8080          RU
# → http://127.0.0.1:8080/en       EN
# → http://127.0.0.1:8080/login    кабинет владельца
```

При первом запуске создаётся база, владелец и 8 демонстрационных статей
(4 пары RU/EN). Если пароль не задан, он генерируется и печатается **один раз**:

```text
──────────────────────────────────────────────
 OWNER BOOTSTRAP — save this password now
   user:     owner
   password: 9f3c1a...
──────────────────────────────────────────────
```

Свой пароль на первом запуске: `RZ_PASSWORD=...` или `--password ...`.
Поменять позже — в кабинете (`/cabinet/security`). Хеш пароля для `.env`
можно получить так:

```bash
cargo run --release -- hash-password 'новый-пароль'   # pbkdf2_sha256$210000$...
```

### CLI

```text
rudolfzinovev [serve] [--bind ADDR] [--data DIR] [--owner NAME] [--password PW]
                      [--workers N] [--no-gzip] [--seed]
rudolfzinovev hash-password <password>
rudolfzinovev seed
```

### Переменные окружения

| Переменная | По умолчанию | Назначение |
|---|---|---|
| `RZ_BIND` | `0.0.0.0:8080` | адрес прослушивания (за прокси — `127.0.0.1:8080`) |
| `RZ_DATA` | `./data` | каталог с `site.sqlite` |
| `RZ_OWNER` | `owner` | логин владельца |
| `RZ_PASSWORD` | — | пароль при первичном создании владельца |
| `RZ_WORKERS` | число CPU | потоки обработки запросов |
| `RZ_GZIP` | `1` | сжатие ответов |
| `RZ_SEED` | `1` | наполнить пустую базу демо-статьями |

---

## Маршруты

| Метод | Путь | Что отдаёт |
|---|---|---|
| GET | `/`, `/en` | главная: герой-манифест, спецификация узла, свежие записи, теги |
| GET | `/projects`, `/en/projects` | каталог проектов с поиском и тегами |
| GET | `/projects/:slug` | страница проекта: каталожный номер, TOC, перевод |
| GET | `/notes`, `/en/notes` | заметки с поиском (`?q=`) и фильтром по тегу |
| GET | `/tags/:slug` | записи одного тега |
| GET | `/notes/:slug`, `/en/notes/:slug` | заметка: поля заметок, TOC, код, таблицы, related, перевод |
| GET | `/about`, `/en/about` | профиль, манифест, «сейчас», контакты, лицензия |
| GET | `/feed.xml`, `/en/feed.xml` | RSS 2.0 |
| GET | `/sitemap.xml`, `/robots.txt` | индексация |
| GET | `/login` · POST `/login` | вход владельца (CSRF + rate limit) |
| POST | `/logout` | выход, отзыв сессии |
| GET | `/cabinet` | обзор: счётчики, рендер-метрики, последние записи |
| GET | `/cabinet/articles` | таблица всех записей: тип, язык, статус, публикация, удаление |
| GET/POST | `/cabinet/articles/new` | Markdown-редактор новой записи |
| GET/POST | `/cabinet/articles/:id` | редактирование записи |
| POST | `/cabinet/articles/:id/status` \| `/delete` | публикация/черновик, удаление |
| GET/POST | `/cabinet/profile` | имя, роли RU/EN, манифест, «сейчас», контакты |
| GET | `/cabinet/telemetry` | 14 дней, топ-пути, среднее и p95 время рендера, боты |
| GET | `/cabinet/security` | сессии, отзыв, смена пароля |
| GET | `/cabinet/backup` · `/cabinet/backup.json` | JSON-снимок статей, профиля и агрегатов |
| GET | `/assets/style.css`, `/assets/app.js` | статика с `immutable` и ETag |
| GET | `/healthz` | проверка живости для systemd/Caddy/Docker |

Весь публичный контент работает **без JavaScript**: навигация — ссылки,
поиск — обычный `GET`-форм, кабинет — `POST`-формы.

---

## Второй бэкенд: `axum/`

В репозитории два сервера для одного сайта. Основной (`src/`) — без единой
внешней зависимости; альтернативный (`axum/`) — на axum + tokio, он
переиспользует всё ядро и заменяет только транспорт. Сборка второго:
`cargo build -p rudolfzinovev-axum`. Подробности и чек-лист первой проверки —
в [`axum/README.md`](axum/README.md).

## Дизайн

Сайт — это **архив, который читается как книга**. Не лендинг, не дашборд и не
блог-шаблон: у каждой страницы есть каталожный номер, поля для заметок и
типографика бумажного издания.

- **Колонтитул вместо навбара.** Сверху одна строка: марка `❦`, имя, два
  раздела (активный помечен `aria-current`) и переключатель языка. Ниже
  — сразу текст, без обвязки.
- **Титульный лист** на главной: роль, имя в две строки (фамилия тише),
  черта, манифест, счётчик записей и ссылки. Первый экран — утверждение, а не
  каталог.
- **Каталожные номера.** Каждая запись пронумерована (`01`, `03`…) — номер
  стоит и в списке, и в шапке статьи («запись 01»), и в подвале статьи. Это
  единственный способ отличить архив от ленты.
- **Два раздела, один аппарат.** `/projects` — каталог того, что можно
  запустить; `/notes` — личные записи. Обе страницы — один и тот же
  шаблон с разной подписью; проект и заметка различаются каталожным
  словом («проект 01» / «запись 01»), хлебной крошкой и адресом.
- **Год как второй уровень каталога.** Лента разбита на блоки по годам:
  `2025` моноширинной строкой с линейкой, под ней — записи. Номер отвечает
  на «которая по счёту», год — на «когда».
- **Оглавление в поле.** На широких экранах оно живёт справа от текста и
  липнет к верху; на узких его нет вовсе — все заголовки и так в тексте.
  Первый абзац статьи начинается примерно на 320 px от верха страницы.
- **Точечные выноски.** Строка списка = номер · заголовок · `……` · дата.
  Выноска тянется сама, поэтому даты всегда выровнены по правому краю.
- **Поля для заметок.** `> [!note] текст` в Markdown уходит в правое поле
  (≥ 1180 px), а на узких экранах остаётся в потоке серым курсивом. Так
  выглядят побочные мысли, которые не должны перебивать основной текст.
- **Оглавление как в книге**: главы с точечными выносками и номерами,
  буквица на первом абзаце, маркер `§` у подзаголовков.
- **Одна антиква** (Iowan Old Style / Palatino / Georgia) для текста,
  системный гротеск для интерфейса, моноширинный для номеров, дат и кода.
  Ноль веб-шрифтов, ноль изображений, ноль иконок.
- **Одна тёплая подложка, без переключателя.** Тёмной темы нет: фон
  `#fcfbf7` — почти белый, с едва заметной желтизной. Никаких
  текстур, теней и второго слоя.
- **Один акцент**: `#8b4a1c` (жжёная сиена) — только ссылки, активное
  состояние и каталожный номер. Ошибки — `#a53f28`, тоже из бумажной гаммы.

## Производительность

| Метрика | Значение |
|---|---|
| Ответ сервера (SSR + БД) | **0.7–1.1 мс** (TTFB на loopback) |
| Главная, gzip | **9.6 КБ** (весь документ, CSS внутри) |
| Заметка, gzip | **11.0 КБ** |
| CSS, gzip | 5.7 КБ (инлайн, один запрос на документ) |
| JS, gzip | **1.4 КБ** (progressive enhancement, без фреймворков) |
| Бинарник | 910 КБ (400 КБ gzip) |
| Внешние запросы браузера | **0** |

Приёмы: инлайн CSS в `<head>`, `ETag` + `304` на каждый HTML-ответ,
`Content-Encoding: gzip` на лету через `libz`, keep-alive с таймаутом 15 с,
`Cache-Control: immutable` для статики, отсутствие веб-шрифтов и трекеров.
Телеметрия показывает `avg`/`p95` рендера в микросекундах — регрессии видны
в кабинете, без профайлера.

Проверить самому:

```bash
curl -sD- -o/dev/null http://127.0.0.1:8080/ | grep -Ei 'etag|server-timing|content-encoding'
curl -s -H 'Accept-Encoding: gzip' -o /dev/null -w '%{size_download} bytes\n' http://127.0.0.1:8080/
```

---

## Безопасность

- Пароль: `PBKDF2-HMAC-SHA256`, 210 000 итераций, 16-байтная соль из
  `OpenSSL RAND_bytes`; сравнение — `CRYPTO_memcmp`.
- Сессии: 256-битный токен из CSPRNG, в базе только `SHA-256` хеш токена,
  cookie `HttpOnly; SameSite=Strict`, срок 30 дней, отзыв по одной сессии
  или всех сразу, продление `last_seen` не чаще раза в минуту.
- CSRF: `Origin`/`Referer` должны совпадать с `Host`, плюс проверка
  `Sec-Fetch-Site`; для формы входа — одноразовый токен в cookie.
- CSP: `default-src 'none'`, скрипт разрешён **по SHA-256 хешу** (`sha256-…`),
  без `unsafe-eval`; плюс `X-Frame-Options: DENY`, `nosniff`,
  `Referrer-Policy`, `Permissions-Policy`.
- Rate limiting: 6 попыток входа за 10 минут, затем блокировка на 10 минут;
  отдельный лимит на мутирующие запросы кабинета.
- Весь Markdown экранируется; ссылки допускаются только `http(s)`, `mailto`,
  относительные и `#anchor` — XSS через `javascript:` невозможен.
- Телеметрия хранит дневной хеш посетителя, без полного IP и без внешних
  счётчиков.

---

## Развёртывание на VPS

### Вариант 1 — systemd + Caddy (рекомендуется)

```bash
git clone <repo> && cd rudolfzinovev.me
sudo ./deploy/install-vps.sh          # идемпотентно
```

Скрипт: ставит `build-essential libsqlite3-dev libssl-dev zlib1g-dev`,
собирает релиз, создаёт пользователя `rz`, каталог `/srv/rudolfzinovev`,
генерирует `.env` с паролем (печатает один раз), ставит hardened
`systemd`-юнит, поднимает сервис, обновляет `Caddyfile` и создаёт cron
ночного `sqlite3 .backup` с ротацией 7 дней.

Файлы в репозитории:

- `deploy/rudolfzinovev.service` — `ProtectSystem=strict`, `PrivateTmp`,
  `MemoryDenyWriteExecute`, `SystemCallFilter`, запись только в `data/`;
- `deploy/Caddyfile` — автоматический TLS, HTTP/2 + HTTP/3, health-check;
- `deploy/nginx.conf` — альтернатива с certbot;
- `deploy/pull-and-run.fish` — локальный запуск на маке одной командой;
- `.env.example` — все переменные с комментариями.

### Вариант 2 — Docker

```bash
cp .env.example .env      # RZ_BIND=0.0.0.0:8080
docker compose up -d --build
```

Мультистейдж на `rust:1-alpine` → `alpine:3.20`, `read_only`, `cap_drop: ALL`,
healthcheck на `/healthz`, данные в `./data`.

### GitHub и журнал проекта

У каждого проекта может быть свой репозиторий и своя история. Они не
смешиваются — на странице проекта это два разных блока:

- **«Из репозитория»** — автоматически: релизы, коммиты, дата правки
  README, звёзды, язык, лицензия. Моноширинная лента с датами, править
  нельзя.
- **«Журнал проекта»** — только своими руками: `Note` (наблюдение),
  `Decision` (принятое решение), `Milestone` (этап). Сериф, буквица,
  метка `N` / `D` / `M` на поле.

Привязка задаётся в кабинете: `/cabinet/projects/<id>` → `owner/name` или
ссылка на GitHub. Кэш обновляется не чаще раза в 30 минут и всегда с
запасным вариантом: если GitHub недоступен, страница показывает последний
снимок и честно об этом пишет — никаких 500 и пустых блоков.

Issues и Pull Requests по умолчанию выключены. Включаются галочкой и
показываются только счётчиком открытых issues.

Проверить репозиторий без запуска сервера:

```bash
./target/release/rudolfzinovev github RUdolf1517/tzim
```

### Локальный запуск на маке (fish)

Скрипт самодостаточный: репозиторий для него не нужен. Он скачивает ветку
сам — клонирует в `~/rudolfzinovev.me` (или обновляет уже склонированный),
собирает релиз, освобождает порт, запускает сервер, ждёт `/healthz` и
открывает браузер.

Запуск без сохранения файла на диск:

```fish
curl -fsSL https://raw.githubusercontent.com/RUdolf1517/rudolfzinovev.me/arena/b9b8d4dd-rudolfzinovev-me/deploy/pull-and-run.fish | fish
```

С флагами (аргументы идут после `-`):

```fish
curl -fsSL https://raw.githubusercontent.com/RUdolf1517/rudolfzinovev.me/arena/b9b8d4dd-rudolfzinovev-me/deploy/pull-and-run.fish | fish -s -b main
```

Если хочется файлом — каталог должен существовать, иначе `curl -o` падает с
`(56) Failure writing output to destination`:

```fish
mkdir -p ~/bin
curl -fsSL https://raw.githubusercontent.com/RUdolf1517/rudolfzinovev.me/arena/b9b8d4dd-rudolfzinovev-me/deploy/pull-and-run.fish -o ~/bin/pull-and-run.fish
chmod +x ~/bin/pull-and-run.fish
~/bin/pull-and-run.fish
```

Полезные флаги: `-b` ветка, `-p 9090` порт, `-t ~/src/rz` другой каталог для
клона, `-n` без сборки (только перезапуск), `-s` сид демо-данных, `-o ~/rzdata`
свой каталог данных, `-d` в фоне без ожидания Ctrl-C, `-k` упасть, если порт
уже занят, `-h` справка.

Ветку скрипт проверяет через `ls-remote`: если ветки в origin нет (например её
уже влили в `main` и удалили), он не падает, а берёт `main`. Незакоммиченные
правки в клоне он не затирает — предупреждает и остаётся на текущей ветке.

Сервер слушает **только** `127.0.0.1` — снаружи машины он не виден, поэтому
локальный запуск не конфликтует с nginx на VPS. Пароль владельца берётся из
`RZ_PASSWORD`, затем из `.env`, затем из `data/dev-password` (создаётся один
раз, режим 0600) — печатается в конце. Пароль передаётся серверу через
окружение, а не через argv, чтобы не светиться в `ps`.

На macOS нужен Homebrew: скрипт сам поставит `sqlite` и `openssl@3`, если их нет,
и выставит `LIBRARY_PATH`/`PKG_CONFIG_PATH` — без правки `RUSTFLAGS`, чтобы не
ломать кэш сборки.

Перед первым запуском стоит проверить синтаксис — скрипт написан без доступа к
fish-интерпретатору:

```fish
fish -n ~/bin/pull-and-run.fish && ~/bin/pull-and-run.fish
```

### Обновление

```bash
git pull && cargo build --release --locked
sudo install -m 0755 target/release/rudolfzinovev /srv/rudolfzinovev/
sudo systemctl restart rudolfzinovev
```

База и сессии не затрагиваются. Схема применяется идемпотентно при старте.

### Бэкап

```bash
sqlite3 /srv/rudolfzinovev/data/site.sqlite ".backup '/tmp/site-backup.sqlite'"
# или из кабинета: /cabinet/backup.json
```

---

## Тесты

```bash
cargo test          # 42 теста: HTTP-разбор, rate limiter, SQL, сессии, шаблоны, RSS,
                    # даты, склонения, JSON-парсер, GitHub-клиент, журнал проекта
```

Тесты поднимают настоящую SQLite во временном каталоге и проходят полный
сценарий: вход → cookie → CSRF-проверка → правка профиля.

---

## Структура

```text
src/
  main.rs       CLI, конфигурация, запуск сервера
  github.rs     GitHub REST через libssl: релизы, коммиты, README
  json.rs       разбор JSON без крейтов
  app.rs        маршрутизация, авторизация, CSRF, телеметрия, кабинет
  http.rs       HTTP/1.1, keep-alive, gzip, ETag, cookies, rate limiter
  db.rs         схема, миграции, статьи, профиль, сессии, бэкап, демо-контент
  models.rs     Article, Profile, SessionInfo, Telemetry, Lang
  templates.rs  SSR-шаблоны: публичные страницы, кабинет, RSS, sitemap
  markdown.rs   Markdown → HTML с якорями, TOC, таблицами и экранированием
  i18n.rs       словари RU/EN, UTC-даты, RFC-2822
  crypto.rs     FFI: OpenSSL (PBKDF2, SHA-256, CSPRNG) и zlib (gzip)
  sqlite.rs     FFI: libsqlite3 (prepared statements, WAL)
assets/         style.css (инлайн), app.js (< 2 КБ gzip), favicon.svg
deploy/         systemd, Caddyfile, nginx.conf, install-vps.sh, pull-and-run.fish
```

---

## Лицензия

**RZ-NONCOMMERCIAL-1.0** (см. [`LICENSE`](LICENSE)).

Код можно читать, изучать, запускать для себя и изменять для личного
некоммерческого использования. **Любое коммерческое использование и любое
извлечение выгоды — включая рекламу, платный доступ, консалтинг, размещение
на коммерческих площадках и перепродажу — запрещено.**
