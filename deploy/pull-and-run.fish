#!/usr/bin/env fish
#
# pull-and-run.fish — скачать сайт с ветки и запустить его локально на маке.
#
# Скрипт самодостаточный: его НЕ нужно держать в репозитории. Положи файл
# куда угодно (например ~/bin/pull-and-run.fish) и запускай откуда угодно —
# он сам склонирует репозиторий или обновит уже склонированный.
#
#   curl -fsSL https://raw.githubusercontent.com/RUdolf1517/rudolfzinovev.me/arena/b9b8d4dd-rudolfzinovev-me/deploy/pull-and-run.fish -o ~/bin/pull-and-run.fish
#   chmod +x ~/bin/pull-and-run.fish
#   ~/bin/pull-and-run.fish
#
# Запуск:
#   pull-and-run.fish                 ветка по умолчанию, порт 8080, ~/rudolfzinovev.me
#   pull-and-run.fish -b main         другая ветка
#   pull-and-run.fish -p 9090         другой порт
#   pull-and-run.fish -t ~/src/rz     другой каталог для клона
#   pull-and-run.fish -n              без сборки, только перезапуск
#   pull-and-run.fish -s              + сид демо-данных
#   pull-and-run.fish -o ~/rzdata     свой каталог данных
#   pull-and-run.fish -d              в фоне, без ожидания Ctrl-C
#   pull-and-run.fish -k              упасть, если порт уже занят
#
# Что делает: клонирует репозиторий (если его нет) или обновляет рабочую копию
# на нужной ветке, собирает релиз, освобождает порт, запускает сервер,
# ждёт /healthz и открывает браузер. Слушает только 127.0.0.1.

# ------------------------------------------------------------------ константы

set -g repo_url "https://github.com/RUdolf1517/rudolfzinovev.me.git"
# Ветка по умолчанию — та, где лежит самая свежая работа. Когда её вльют в
# main, скрипт это поймёт сам: пропадёт из origin — возьмёт main.
set -g default_branch "arena/b9b8d4dd-rudolfzinovev-me"
set -g fallback_branch "main"

# Скрипт одинаково работает и файлом, и через `curl … | fish`: `status filename`
# в конвейере пуст, поэтому справка напечатана встроенным текстом.
# ------------------------------------------------------------------- fetch-url
# Печатает этот скрипт из ветки через api.github.com — когда
# raw.githubusercontent.com не резолвится. Использование:
#   pull-and-run.fish fetch-url > ~/pull-and-run.fish
if set -q argv[1]; and test "$argv[1]" = fetch-url
    set -l ref arena/b9b8d4dd-rudolfzinovev-me
    if set -q argv[2]
        set ref $argv[2]
    end
    set -l url "https://api.github.com/repos/RUdolf1517/rudolfzinovev.me/contents/deploy/pull-and-run.fish?ref=$ref"
    command -q curl; or begin
        echo "нужен curl" >&2
        exit 1
    end
    curl -fsSL -H 'Accept: application/vnd.github.raw' $url
    exit $status
end

# ВАЖНО: в fish спецификация — это только `короткий/длинный=`, где `=` значит
# «принимает значение». Писать `p/port=8080` нельзя: fish отвечает
# «Invalid option spec ... at char '8'». Значения по умолчанию заданы ниже.
argparse --name=pull-and-run.fish 'h/help' \
    'b/branch=' 'p/port=' 't/target=' 'o/data=' 'w/workers=' \
    'n/no-build' 'k/keep-old' 's/seed' 'd/detach' -- $argv
or begin
    printf 'непонятный флаг, см. --help\n' >&2
    exit 2
end

if set -q _flag_h
    printf '%s\n' \
        'pull-and-run.fish — скачать сайт с ветки и запустить локально' \
        '' \
        'Запуск:' \
        '  pull-and-run.fish                 ветка по умолчанию, порт 8080' \
        '  pull-and-run.fish -b main         другая ветка' \
        '  pull-and-run.fish -p 9090         другой порт' \
        '  pull-and-run.fish -t ~/src/rz     другой каталог для клона' \
        '  pull-and-run.fish -n              без сборки, только перезапуск' \
        '  pull-and-run.fish -s              + сид демо-данных' \
        '  pull-and-run.fish -o ~/rzdata     свой каталог данных' \
        '  pull-and-run.fish -d              в фоне, без ожидания Ctrl-C' \
        '  pull-and-run.fish -k              упасть, если порт уже занят' \
        '' \
        'Если raw.githubusercontent.com не открывается (бывает в России),' \
        'скачай сам через API и запусти:' \
        '  pull-and-run.fish fetch-url > ~/pull-and-run.fish' \
        '  chmod +x ~/pull-and-run.fish && ~/pull-and-run.fish' \
        '' \
        'Клонирует репозиторий (если его нет) или обновляет рабочую копию на' \
        'нужной ветке, собирает релиз, запускает сервер и открывает браузер.' \
        'Слушает только 127.0.0.1.'
    exit 0
end

# Ключи argparse не должны просочиться в cargo или в сервер.
set -e argv

# ------------------------------------------------------------------ функции

function step --argument-names text
    set_color brblack; printf '▸ '; set_color normal; printf '%s\n' $text
end

function ok --argument-names text
    set_color green; printf '✓ '; set_color normal; printf '%s\n' $text
end

function warn --argument-names text
    set_color yellow; printf '! '; set_color normal; printf '%s\n' $text
end

function die --argument-names text
    set_color red; printf '✗ '; set_color normal; printf '%s\n' $text >&2
    exit 1
end

function tail_log --argument-names file
    if test -f $file
        test -s $file; and tail -n 20 $file
    end
end

# Открывает ссылку в браузере на маке; тихо ничего не делает, если нельзя.
function open_url --argument-names url
    if command -q open
        open $url 2>/dev/null
    else if command -q xdg-open
        xdg-open $url 2>/dev/null
    end
end

# ------------------------------------------------------------------ настройки

set -l branch $default_branch
set -g branch_given 0
if set -q _flag_b; and test -n "$_flag_b"
    set branch $_flag_b
    set branch_given 1
end

set -l port 8080
if set -q _flag_p; and test -n "$_flag_p"
    set port $_flag_p
end

set -l workers 4
if set -q _flag_w; and test -n "$_flag_w"
    set workers $_flag_w
end

# Куда клонировать. По умолчанию ~/rudolfzinovev.me — рядом с домашним, а не
# в текущем каталоге: скрипт можно запускать откуда угодно.
set -g root "$HOME/rudolfzinovev.me"
if set -q _flag_t; and test -n "$_flag_t"
    set root $_flag_t
end
string match -q '/*' -- $root; or set root "$PWD/$root"
# Хвостовой слэш ломает сравнения путей — убираем.
set root (string replace -r '/+$' '' -- "$root")
test -n "$root"; or set root /

set -g data "$root/data"
if set -q _flag_o; and test -n "$_flag_o"
    set data $_flag_o
    string match -q '/*' -- $data; or set data "$PWD/$data"
    mkdir -p -- $data; or die "не смог создать $data"
end

set -l bind "127.0.0.1:$port"
# global, а не local: функции в fish не видят локальные переменные вызывающего,
# а обработчик Ctrl-C должен суметь убить сервер и убрать pid-файл.
set -g log "$data/server.log"
set -g pidfile "$data/server.pid"
set -g bin "$root/target/release/rudolfzinovev"

step "rudolfzinovev · локальный запуск"
printf '  репозиторий: %s\n' $root

command -q git; or die "нет git — поставь Xcode Command Line Tools: xcode-select --install"
command -q curl; or die "нет curl — на macOS он входит в систему; проверь PATH"
# cargo нужен только когда действительно будем собирать: -n означает «не собирай».
if not set -q _flag_n
    command -q cargo; or die "нет cargo — поставь Rust: https://rustup.rs (потом перезайди в терминал)"
end

# Проверяем, что ветка вообще есть в origin: иначе клон упадёт с невнятной
# ошибкой, а после слияния ветки в main ссылка на неё станет битой. Таймаут,
# чтобы плохая сеть давала сообщение, а не бесконечное ожидание.
set -gx GIT_TERMINAL_PROMPT 0
set -gx GIT_HTTP_LOW_SPEED_LIMIT 1000
set -gx GIT_HTTP_LOW_SPEED_TIME 30
step "проверяю доступ к origin"
if git ls-remote --exit-code --heads $repo_url $branch >/dev/null 2>&1
    ok "ветка $branch есть в origin"
else
    if test $branch_given -eq 1
        die "ветки $branch нет в $repo_url"
    end
    if not git ls-remote --exit-code --heads $repo_url $fallback_branch >/dev/null 2>&1
        die "не могу достучаться до GitHub и не вижу ветку $branch. Проверь сеть: git ls-remote $repo_url"
    end
    warn "ветки $branch больше нет в origin — беру $fallback_branch"
    set branch $fallback_branch
    git ls-remote --exit-code --heads $repo_url $branch >/dev/null 2>&1
    or die "не вижу в origin ни $default_branch, ни $fallback_branch"
end
printf '  ветка:       %s\n' $branch

# ------------------------------------------------------- клонирование и pull

# Три случая: каталога нет, каталог есть но это не репозиторий, репозиторий есть.
if not test -d "$root"
    step "клонирую $repo_url"
    set -l parent (dirname $root)
    mkdir -p -- $parent; or die "не смог создать $parent"
    git clone --branch $branch --depth 1 $repo_url $root
    or die "git clone не прошёл"
    ok "склонировано в $root"
else if not test -d "$root/.git"
    # Каталог занят чем-то другим — не трогаем его.
    die "$root уже существует и это не git-репозиторий; укажи другой каталог через -t"
else
    cd $root; or die "нет доступа к $root"

    # Клон мог быть сделан с --depth 1 (тогда видна только одна ветка),
    # поэтому сначала расширяем список веток remote'а, потом делаем fetch.
    git remote set-branches origin '*' 2>/dev/null
    set -l remote_ok 1
    git fetch --prune origin; or set remote_ok 0
    if test $remote_ok -eq 0
        warn "git fetch не прошёл — собираю то, что уже лежит локально"
    else
        set -l dirty ""
        if not git diff --quiet; or not git diff --cached --quiet
            set dirty 1
        end

        set -l current (git rev-parse --abbrev-ref HEAD 2>/dev/null)
        if test "$current" != "$branch"
            if test -n "$dirty"
                # Переключение ветки поверх правок — так теряют работу. Но и
                # падать не обязательно: собираем то, что уже есть.
                warn "в $root есть незакоммиченные правки; остаюсь на ветке $current"
                set branch $current
            else
                step "переключаюсь на ветку $branch"
                git switch --track -c $branch origin/$branch 2>/dev/null
                or git switch $branch
                or die "не смог переключиться на ветку $branch"
            end
        end

        if test -n "$dirty"
            warn "рабочая копия изменена — git pull пропущен"
        else
            step "забираю изменения"
            if git pull --ff-only --quiet
                ok "код обновлён до "(git rev-parse --short HEAD)
            else
                warn "git pull не прошёл (ветки разошлись?) — собираю локальное состояние"
            end
        end
    end
end

cd $root; or die "нет доступа к $root"
test -f Cargo.toml; or die "в $root нет Cargo.toml — это не тот репозиторий"

# --------------------------------------------------- системные библиотеки
# Бинарник линкуется с системными libsqlite3, libcrypto и libz: на macOS они
# ставятся через Homebrew, и линкеру нужно показать, где они лежат.

if test (uname -s) = Darwin
    if not command -q brew
        die "нет Homebrew — поставь его с https://brew.sh"
    end
    set -l brew_prefix (brew --prefix)
    for formula in sqlite openssl@3
        if not test -d "$brew_prefix/opt/$formula"
            step "brew install $formula"
            brew install $formula; or die "не удалось поставить $formula"
        end
    end
    # Отдельные переменные, а не RUSTFLAGS: правка RUSTFLAGS меняет хэш сборки
    # и ломает кэш между запусками.
    # В fish необъявленная переменная в списке — это не пустота, а ошибка
    # "variables cannot be combined", поэтому подставляем её только если она есть.
    set -l lib_paths "$brew_prefix/opt/sqlite/lib" "$brew_prefix/opt/openssl@3/lib"
    set -q LIBRARY_PATH; and set lib_paths $lib_paths $LIBRARY_PATH
    set -gx LIBRARY_PATH $lib_paths
    set -l pkg_paths "$brew_prefix/opt/sqlite/lib/pkgconfig" "$brew_prefix/opt/openssl@3/lib/pkgconfig"
    set -q PKG_CONFIG_PATH; and set pkg_paths $pkg_paths $PKG_CONFIG_PATH
    set -gx PKG_CONFIG_PATH $pkg_paths
end

# -------------------------------------------------------------------- сборка

if set -q _flag_n
    test -x $bin; or die "бинарника нет, а сборка отключена (-n)"
    warn "сборка пропущена (-n)"
else
    # Первая сборка на axum качает две зависимости с crates.io (axum + tokio и
    # их дерево). Дальше они лежат в ~/.cargo и сеть не нужна.
    step "cargo build --release"
    if not test -f Cargo.lock
        warn "Cargo.lock нет — cargo создаст его сам при первой сборке"
    end
    set -l started (date +%s)
    # Вывод не глушим: если сборка падает, сообщение об ошибке должно быть видно.
    if not cargo build --release
        die "сборка упала"
    end
    set -l size (du -h $bin | string replace -r '\s.*$' '' | string trim)
    test -n "$size"; or set size "?"
    ok "собрано за "(math (date +%s) - $started)"s · $size"
end

# ------------------------------------------------------------ порт и старый pid

mkdir -p $data

if test -f $pidfile
    set -l old (cat $pidfile 2>/dev/null)
    # pid мог достаться чужому процессу после перезагрузки — проверяем имя,
    # чтобы не убить что-то постороннее.
    if test -n "$old"; and kill -0 $old 2>/dev/null
        set -l old_name (ps -p $old -o comm= 2>/dev/null)
        if not string match -q '*rudolfzinovev*' -- $old_name
            warn "в $pidfile чужой pid $old ($old_name) — игнорирую"
            set old ""
        end
    end
    if test -n "$old"; and kill -0 $old 2>/dev/null
        if set -q _flag_k
            die "на порту $port уже работает pid $old (флаг -k)"
        end
        step "останавливаю прошлый сервер (pid $old)"
        kill $old 2>/dev/null
        set -l waited 0
        while test $waited -lt 30
            kill -0 $old 2>/dev/null; or break
            sleep 0.1
            set waited (math $waited + 1)
        end
        kill -9 $old 2>/dev/null
    end
    rm -f $pidfile
end

# Порт мог занять кто-то другой — например, забытый `cargo run`.
if command -q lsof
    set -l squatter (lsof -ti tcp:$port 2>/dev/null)
    if test -n "$squatter"
        warn "порт $port занят процессом $squatter — освобождаю"
        kill $squatter 2>/dev/null
        sleep 0.5
        set squatter (lsof -ti tcp:$port 2>/dev/null)
        test -n "$squatter"; and kill -9 $squatter 2>/dev/null
        sleep 0.3
    end
end

# ---------------------------------------------------------------- пароль

set -l owner owner
if set -q RZ_OWNER; and test -n "$RZ_OWNER"
    set owner $RZ_OWNER
end

set -l password ""
if set -q RZ_PASSWORD; and test -n "$RZ_PASSWORD"
    set password $RZ_PASSWORD
else
    # .env, если он есть, важнее автогенерации.
    if test -f "$root/.env"
        set -l line (grep -m1 '^RZ_PASSWORD=' "$root/.env" 2>/dev/null | string replace -r '^RZ_PASSWORD=' '')
        if test -n "$line"
            set password (string trim -c '"' -c "'" -- $line)
        end
    end
end

if test -z "$password"
    # Пароль переживает перезапуски: иначе после каждого запуска пришлось бы
    # логиниться заново и заново менять пароль владельца.
    if test -f "$data/dev-password"
        set password (cat "$data/dev-password")
        warn "пароль владельца — из $data/dev-password"
    else
        set password (openssl rand -hex 12 2>/dev/null)
        test -n "$password"; or set password "dev-password-"(random 1000 9999)
        printf '%s' $password >"$data/dev-password"
        chmod 600 "$data/dev-password" 2>/dev/null
        ok "новый пароль владельца записан в $data/dev-password"
    end
end

# ------------------------------------------------------------------- запуск

set -l seed_flags
if set -q _flag_s
    set seed_flags --seed
end

# Пароль выше имеет смысл только при первой инициализации базы: если база уже
# есть, владелец остаётся с тем паролем, который был хэширован раньше.
set -g fresh_db 0
if not test -f "$data/site.sqlite"
    set fresh_db 1
end

step "запускаю на http://$bind"
# Пароль передаём только через окружение: в argv его видно в `ps` всем юзерам.
env RZ_BIND=$bind RZ_DATA=$data RZ_OWNER=$owner RZ_PASSWORD=$password \
    $bin serve --bind $bind --data $data --owner $owner \
    --workers $workers $seed_flags >$log 2>&1 &
set -g pid $last_pid
printf '%s' $pid >$pidfile

# Ctrl-C должен гасить и сервер, а не только скрипт: иначе он останется
# висеть на порту, и следующий запуск будет его вычищать.
function __rz_cleanup --on-signal INT --on-signal TERM
    step "останавливаю сервер"
    kill $pid 2>/dev/null
    rm -f $pidfile
    exit 0
end

# Ждём именно ответа /healthz: процесс может быть жив, но ещё открывать базу.
set -l ready 0
set -l attempt 0
while test $attempt -lt 100
    set attempt (math $attempt + 1)
    if not kill -0 $pid 2>/dev/null
        printf '\n'
        warn "сервер умер при старте, хвост лога:"
        tail_log $log
        exit 1
    end
    if command -q curl
        if curl -fsS -m 1 "http://$bind/healthz" >/dev/null 2>&1
            set ready 1
            break
        end
    else
        if test $attempt -ge 15
            set ready 1
            break
        end
    end
    sleep 0.1
end
printf '\n'

test $ready -eq 1; or die "сервер не ответил на /healthz. Смотри $log"
ok "готово: http://$bind · pid $pid"
printf '  админка: http://%s/login\n' $bind
if test $fresh_db -eq 1
    printf '  вход:    %s / %s\n' $owner $password
else
    printf '  вход:    %s / пароль из %s/dev-password\n' $owner $data
end
printf '  лог:     tail -f %s\n' $log

# Браузер открываем в обоих режимах: и когда ждём Ctrl-C, и когда уходим в фон.
open_url "http://$bind/"

if set -q _flag_d
    printf '  стоп:    kill %s\n' $pid
    exit 0
end

while kill -0 $pid 2>/dev/null
    sleep 1
end
warn "сервер завершился сам — хвост лога:"
tail_log $log
