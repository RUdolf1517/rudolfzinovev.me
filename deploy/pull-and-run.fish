#!/usr/bin/env fish
#
# pull-and-run.fish — забрать сайт с ветки и запустить его локально на маке.
#
# Что делает: pull нужной ветки → собрать релиз → освободить порт → запустить
# сервер → дождаться /healthz → открыть браузер.
#
# Запуск:
#   ./deploy/pull-and-run.fish              текущая ветка, порт 8080
#   ./deploy/pull-and-run.fish -b main      другая ветка (дерево должно быть чистым)
#   ./deploy/pull-and-run.fish -p 9090      другой порт
#   ./deploy/pull-and-run.fish -n           без сборки, только перезапуск
#   ./deploy/pull-and-run.fish -s           + сид демо-данных
#   ./deploy/pull-and-run.fish -o ~/rz      свой каталог данных
#   ./deploy/pull-and-run.fish -d           в фоне, без ожидания Ctrl-C
#
# Ветку по умолчанию скрипт НЕ переключает: берёт ту, на которой ты стоишь,
# чтобы не утащить незакоммиченные правки на чужую ветку.
#
# Слушает только 127.0.0.1 — снаружи машины сервер не виден.

argparse --name=(status current-command) 'h/help' \
    'b/branch=' 'p/port=8080' 'o/data=' 'w/workers=4' 'n/no-build' 'k/keep-old' 's/seed' 'd/detach' -- $argv
or begin
    printf 'см. %s --help\n' (status filename)
    exit 2
end

if set -q _flag_h
    # Печатаем шапку файла без решёток — до первой строки, не начинающейся с «#».
    awk 'NR > 1 { if ($0 !~ /^#/) exit; sub(/^# ?/, ""); print }' (status filename)
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

# ---------------------------------------------------------------- настройки

# realpath на маке либо отсутствует, либо BSD-шный (без -m), поэтому корень
# считаем через cd + pwd -P: это работает везде и разворачивает симлинки.
set -l here (dirname (status filename))
# cd здесь выполняется в том же процессе, но нам это и нужно: ниже скрипт всё
# равно переходит в корень репозитория.
set -l root (cd "$here/.."; and pwd -P)
test -n "$root"; or die "не понял, где лежит репозиторий"
set -l port 8080
if set -q _flag_p
    and test -n "$_flag_p"
    set port $_flag_p
end
set -l workers 4
if set -q _flag_w
    and test -n "$_flag_w"
    set workers $_flag_w
end
set -l data "$root/data"
if set -q _flag_o
    and test -n "$_flag_o"
    # realpath -m на маке нет, а cd внутри подстановки в fish меняет каталог
    # самого скрипта — поэтому просто делаем путь абсолютным вручную.
    mkdir -p -- $_flag_o; or die "не смог создать $_flag_o"
    set data $_flag_o
    string match -q '/*' -- $data; or set data (pwd)/$data
end
set -l bind "127.0.0.1:$port"
# global, а не local: функции в fish не видят локальные переменные вызывающего,
# а обработчик Ctrl-C должен суметь убить сервер и убрать pid-файл.
set -g log "$data/server.log"
set -g pidfile "$data/server.pid"
set -g bin "$root/target/release/rudolfzinovev"

step "rudolfzinovev · локальный запуск"
printf '  репозиторий: %s\n' $root

cd $root; or die "нет доступа к $root"

# -------------------------------------------------------------------- ветка

set -l current (git rev-parse --abbrev-ref HEAD)
if set -q _flag_b; and test -n "$_flag_b"; and test "$_flag_b" != "$current"
    # Переключение ветки поверх незакоммиченных правок — так теряют работу.
    if not git diff --quiet; or not git diff --cached --quiet
        die "в дереве есть незакоммиченные правки; сначала git stash или commit"
    end
    step "переключаюсь на ветку $_flag_b"
    git switch $_flag_b; or die "не смог переключиться на $_flag_b"
    set current (git rev-parse --abbrev-ref HEAD)
end
ok "ветка: $current"

# --------------------------------------------------------------------- pull

if git rev-parse --verify --quiet '@{upstream}' >/dev/null
    step "забираю изменения"
    if git pull --ff-only --quiet
        ok "код обновлён до "(git rev-parse --short HEAD)
    else
        warn "git pull не прошёл (ветки разошлись?) — собираю локальное состояние"
    end
else
    warn "у ветки нет upstream в origin — pull пропущен"
end

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

command -q cargo; or die "нет cargo — поставь Rust: https://rustup.rs"

# -------------------------------------------------------------------- сборка

if set -q _flag_n
    test -x $bin; or die "бинарника нет, а сборка отключена (-n)"
    warn "сборка пропущена (-n)"
else
    step "cargo build --release"
    set -l started (date +%s)
    # Вывод не глушим: если сборка падает, сообщение об ошибке должно быть видно.
    if not cargo build --release
        die "сборка упала"
    end
    set -l size (du -h $bin | string replace -r '\s.*$' '' | string trim)
    test -n "$size"; or set size "?"
    ok "собрано за "(math (date +%s) - $started)"s · $size
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
if set -q RZ_OWNER
    and test -n "$RZ_OWNER"
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
        step "сервер умер при старте, хвост лога:"
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
    printf '  вход:    %s / %s (если база была создана раньше с другим паролем — он не перезаписан)\n' $owner $password
end
printf '  лог:     tail -f %s\n' $log

# Браузер открываем в обоих режимах: и когда ждём Ctrl-C, и когда уходим в фон.
command -q open; and open "http://$bind/" 2>/dev/null

if set -q _flag_d
    printf '  стоп:    kill %s\n' $pid
    exit 0
end

while kill -0 $pid 2>/dev/null
    sleep 1
end
warn "сервер завершился сам — хвост лога:"
tail_log $log
