# rudolfzinovev.me

> Персональный сайт и закрытый кабинет владельца. **Rust без единого внешнего крейта.**
> A personal site and owner-only cabinet. **Rust, zero external crates.**

[![tests](https://img.shields.io/badge/tests-24%20passing-3f6b34)](#тесты)
[![deps](https://img.shields.io/badge/dependencies-0-d9a04d)](#zero-crates)
[![binary](https://img.shields.io/badge/binary-907%20KB-191713)](#performance)
[![license](https://img.shields.io/badge/license-RZ--NONCOMMERCIAL--1.0-c2542d)](LICENSE)

---

## Что это / What it is

| | |
|---|---|
| **Сайт** | Статьи с тегами, поиском, RSS и оглавлением; два языка (RU/EN), две темы (Obsidian / Paper). |
| **Кабинет** | `/cabinet` — только для владельца: Markdown-редактор с черновиками и публикацией, профиль и манифест сайта, приватная телеметрия, сессии и смена пароля, JSON-бэкап. |
| **Стек** | Rust 2024, `std::net`, системные `libsqlite3` / `libcrypto` / `libz`. Ноль крейтов, ноль JS-сборки, ноль внешних запросов из браузера. |
| **Размер** | 907 КБ бинарник (399 КБ gzip), 11 КБ gzip главная страница, < 2 КБ gzip JavaScript. |
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
| GET | `/articles`, `/en/articles` | индекс с поиском (`?q=`) и фильтром по тегу |
| GET | `/tags/:slug` | записи одного тега |
| GET | `/articles/:slug`, `/en/articles/:slug` | статья: TOC, код, таблицы, related, ссылка на перевод |
| GET | `/about`, `/en/about` | профиль, манифест, «сейчас», контакты, лицензия |
| GET | `/feed.xml`, `/en/feed.xml` | RSS 2.0 |
| GET | `/sitemap.xml`, `/robots.txt` | индексация |
| GET | `/login` · POST `/login` | вход владельца (CSRF + rate limit) |
| POST | `/logout` | выход, отзыв сессии |
| GET | `/cabinet` | обзор: счётчики, рендер-метрики, последние записи |
| GET | `/cabinet/articles` | таблица всех записей: статус, публикация, удаление |
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
поиск — обычный `GET`-форм, тема — cookie `rz_theme`, кабинет — `POST`-формы.

---

## Дизайн

**Instrument-panel brutalism**: палитра измерительного прибора, а не «матрица».

- **OBSIDIAN** (по умолчанию) — обсидиановый фон `#0b0d0f`, тёплый вольфрамовый
  текст `#e8e3d7`, янтарный акцент `#d9a04d`, окисленная медь `#c2542d`.
- **PAPER** — архивная бумага `#efeade`, типографская тушь `#191713`,
  киноварный штамп `#9a4b1e`.
- Ноль скруглений, ноль теней-градиентов, видимая сетка 32 px, координатные
  индексы секций (`01`, `02`), моноширинная телеметрия в шапке, спецификация
  узла как «шильдик прибора».
- Шрифты — только системные: `ui-monospace` и системный гротеск.
  Ноль внешних запросов, ноль веб-шрифтов.

---

## Производительность

| Метрика | Значение |
|---|---|
| Ответ сервера (SSR + БД) | **0.7–1.1 мс** (TTFB на loopback) |
| Главная, gzip | **11.3 КБ** (весь документ, CSS внутри) |
| Статья, gzip | **12.2 КБ** |
| CSS, gzip | 6.4 КБ (инлайн, один запрос на документ) |
| JS, gzip | **2.0 КБ** (progressive enhancement, без фреймворков) |
| Бинарник | 907 КБ (399 КБ gzip) |
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
- `.env.example` — все переменные с комментариями.

### Вариант 2 — Docker

```bash
cp .env.example .env      # RZ_BIND=0.0.0.0:8080
docker compose up -d --build
```

Мультистейдж на `rust:1-alpine` → `alpine:3.20`, `read_only`, `cap_drop: ALL`,
healthcheck на `/healthz`, данные в `./data`.

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
cargo test          # 24 теста: HTTP-разбор, rate limiter, SQL, сессии, шаблоны, RSS, даты
```

Тесты поднимают настоящую SQLite во временном каталоге и проходят полный
сценарий: вход → cookie → CSRF-проверка → правка профиля.

---

## Структура

```text
src/
  main.rs       CLI, конфигурация, запуск сервера
  app.rs        маршрутизация, авторизация, CSRF, телеметрия, кабинет
  http.rs       HTTP/1.1, keep-alive, gzip, ETag, cookies, rate limiter
  db.rs         схема, миграции, статьи, профиль, сессии, бэкап, демо-контент
  models.rs     Article, Profile, SessionInfo, Telemetry, Lang
  templates.rs  SSR-шаблоны: публичные страницы, кабинет, RSS, sitemap
  markdown.rs   Markdown → HTML с якорями, TOC, таблицами и экранированием
  i18n.rs       словари RU/EN, UTC-даты, RFC-2822, темы
  crypto.rs     FFI: OpenSSL (PBKDF2, SHA-256, CSPRNG) и zlib (gzip)
  sqlite.rs     FFI: libsqlite3 (prepared statements, WAL)
assets/         style.css (инлайн), app.js (< 2 КБ gzip), favicon.svg
deploy/         systemd, Caddyfile, nginx.conf, install-vps.sh
```

---

## Лицензия

**RZ-NONCOMMERCIAL-1.0** (см. [`LICENSE`](LICENSE)).

Код можно читать, изучать, запускать для себя и изменять для личного
некоммерческого использования. **Любое коммерческое использование и любое
извлечение выгоды — включая рекламу, платный доступ, консалтинг, размещение
на коммерческих площадках и перепродажу — запрещено.**
