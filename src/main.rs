//! Entry point for the axum build.
//!
//! Same CLI surface the site always had: `serve`, `seed`,
//! `hash-password`, `github <owner/name>`, the same environment variables, the
//! same data directory and the same SQLite file. The difference is that
//! hyper/tokio own the socket and the request loop.
//!
//! `main` is a **plain** function that builds the runtime by hand: the worker
//! count is a documented flag (`--workers`), and `#[tokio::main]` would have
//! already created a runtime before the flag could be read.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use axum::Router;
use axum::extract::{ConnectInfo, Request};
use axum::http::{HeaderMap, HeaderValue, StatusCode};

/// Trait for `HeaderValue::from_str`; without it the call needs a fully
/// qualified path and the error reads like a missing method.
use std::str::FromStr;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use rudolfzinovev_me::state::{ActiveGuard, AppState};
use rudolfzinovev_me::{crypto, db::Store, github, handlers, i18n, sqlite};

fn env_or(key: &str, fallback: &str) -> String {
    std::env::var(key)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

fn usage() -> ! {
    eprintln!(
        "rudolfzinovev {}\n\n\
         USAGE:\n  rudolfzinovev [serve] [--bind ADDR] [--data DIR] [--owner NAME] [--password PW]\n\
         \x20                      [--workers N] [--no-gzip] [--seed]\n  \
         rudolfzinovev hash-password <password>\n  \
         rudolfzinovev seed\n  \
         rudolfzinovev github <owner/name>\n\n\
         ENV:\n  RZ_BIND (0.0.0.0:8080)   RZ_DATA (./data)   RZ_OWNER (owner)\n  \
         RZ_PASSWORD               RZ_WORKERS (CPU count)  RZ_GZIP (1)  RZ_SEED (1)\n",
        env!("CARGO_PKG_VERSION")
    );
    std::process::exit(2)
}

/// The real client address, for the rate limiter and the unique-visitor count.
///
/// Behind Caddy the socket peer is always 127.0.0.1, so `X-Forwarded-For` wins;
/// directly exposed, the peer address from `ConnectInfo` is used. The value is
/// put into a private header so every handler picks it up from one place.
async fn peer_ip(mut req: Request, next: Next) -> Response {
    let forwarded = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|fwd| fwd.split(',').next())
        .map(|ip| ip.trim().to_string())
        .filter(|ip| !ip.is_empty());
    let ip = forwarded
        .or_else(|| {
            req.extensions()
                .get::<ConnectInfo<SocketAddr>>()
                .map(|c| c.0.ip().to_string())
        })
        .unwrap_or_else(|| "unknown".to_string());
    if let Ok(value) = HeaderValue::from_str(&ip) {
        req.headers_mut().insert(handlers::PEER_IP_HEADER, value);
    }
    next.run(req).await
}

fn main() {
    // An explicit runtime keeps the CLI parsing outside of async: `seed`,
    // `hash-password` and `github` must work without starting a single worker.
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("tokio runtime: {err}");
            std::process::exit(1);
        }
    };
    match run(&runtime) {
        Ok(()) => {}
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    }
}

fn run(runtime: &tokio::runtime::Runtime) -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cpu = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2);

    let mut mode = "serve".to_string();
    let mut bind = env_or("RZ_BIND", "0.0.0.0:8080");
    let mut data_dir = PathBuf::from(env_or("RZ_DATA", "./data"));
    let mut owner = env_or("RZ_OWNER", "owner");
    let mut password = std::env::var("RZ_PASSWORD").ok().filter(|p| !p.is_empty());
    let mut workers = env_or("RZ_WORKERS", "")
        .parse::<usize>()
        .ok()
        .unwrap_or(cpu)
        .max(1);
    let mut gzip = env_or("RZ_GZIP", "1") != "0";
    let mut seed = env_or("RZ_SEED", "1") != "0";

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "serve" => mode = "serve".into(),
            "seed" => mode = "seed".into(),
            "hash-password" => {
                let pw = args
                    .get(i + 1)
                    .cloned()
                    .unwrap_or_else(|| usage());
                println!("{}", crypto::hash_password(&pw));
                return Ok(());
            }
            "--version" | "-V" => {
                println!("rudolfzinovev {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--bind" => {
                i += 1;
                bind = args.get(i).cloned().unwrap_or_else(|| usage());
            }
            "--data" => {
                i += 1;
                data_dir = PathBuf::from(args.get(i).cloned().unwrap_or_else(|| usage()));
            }
            "--owner" => {
                i += 1;
                owner = args.get(i).cloned().unwrap_or_else(|| usage());
            }
            "--password" => {
                i += 1;
                password = args.get(i).cloned();
            }
            "--workers" => {
                i += 1;
                workers = args
                    .get(i)
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(workers)
                    .max(1);
            }
            "--no-gzip" => gzip = false,
            "--seed" => seed = true,
            // Diagnostics: one API call, printed. Used to verify the GitHub
            // binding on a VPS where the app sits behind nginx.
            "github" => {
                let Some(spec) = args.get(i + 1) else {
                    eprintln!("usage: rudolfzinovev github <owner/name>");
                    std::process::exit(2);
                };
                let Some(repo) = github::normalise_repo(spec) else {
                    eprintln!("not an owner/name reference: {spec}");
                    std::process::exit(2);
                };
                match github::fetch(&repo) {
                    Ok(fetched) => {
                        let r = &fetched.repo;
                        println!(
                            "{repo} · stars {} · forks {} · {} · {}",
                            r.stars, r.forks, r.language, r.spdx
                        );
                        if !r.description.is_empty() {
                            println!("  {}", r.description);
                        }
                        if !r.release_tag.is_empty() {
                            println!("  release {} ({})", r.release_tag, r.release_name);
                        }
                        println!("  events: {}", fetched.events.len());
                        for e in fetched.events.iter().take(6) {
                            println!("  · {} {} — {}", e.kind, i18n::format_date_utc(e.at), e.title);
                        }
                    }
                    Err(err) => {
                        eprintln!("github: {} ({:?})", err.code(), err);
                        std::process::exit(1);
                    }
                }
                return Ok(());
            }
            "--help" | "-h" => usage(),
            other => {
                eprintln!("unknown argument: {other}");
                usage();
            }
        }
        i += 1;
    }

    if mode == "seed" {
        let store = Store::open(&data_dir.join("site.sqlite"))?;
        match store.seed_if_empty() {
            Ok(true) => println!("seeded: {} articles", store.article_count().unwrap_or(0)),
            Ok(false) => println!("already seeded"),
            Err(err) => return Err(format!("seed failed: {err}")),
        }
        return Ok(());
    }

    let (state, generated) = AppState::boot(&data_dir, &owner, password.as_deref(), seed)
        .map_err(|err| format!("bootstrap failed: {err}"))?;
    if let Some(pw) = generated {
        eprintln!(
            "\n  OWNER BOOTSTRAP — a password was generated, store it now:\n\n    {pw}\n\n  \
             It is printed once. Change it in /cabinet/security.\n"
        );
    }

    runtime.block_on(serve(Arc::new(state), &bind, workers, gzip, &data_dir))
}

async fn serve(
    shared: Arc<AppState>,
    bind: &str,
    workers: usize,
    gzip: bool,
    data_dir: &std::path::Path,
) -> Result<(), String> {
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(|err| format!("bind {bind}: {err}"))?;
    let local = listener
        .local_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| bind.to_string());

    let app = handlers::router(shared.clone(), gzip);
    let app = with_health(app, shared).layer(axum::middleware::from_fn(peer_ip));

    println!(
        "rudolfzinovev.me (axum) {} · listening on {local} · workers {workers} · gzip {} · db {}",
        env!("CARGO_PKG_VERSION"),
        if gzip { "on" } else { "off" },
        data_dir.join("site.sqlite").display()
    );

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .map_err(|err| format!("server error: {err}"))
}

/// `/healthz` is registered here rather than in `handlers` because it reports
/// runtime facts that only `main` owns: the process, the pool, SQLite itself.
fn with_health(app: Router, state: Arc<AppState>) -> Router {
    let handler = get(move |headers: HeaderMap| {
        let state = state.clone();
        async move {
            let _active = ActiveGuard::new(&state.active);
            let body = format!(
                "ok sqlite={} requests={} active={} uptime={}s\n",
                sqlite::Connection::sqlite_version(),
                state.requests.load(Ordering::Relaxed),
                state.active.load(Ordering::Relaxed),
                state.uptime_secs()
            );
            let _ = headers;
            (
                StatusCode::OK,
                [
                    ("content-type", "text/plain; charset=utf-8"),
                    ("cache-control", "no-store"),
                ],
                body,
            )
        }
    });
    app.route("/healthz", handler)
}
