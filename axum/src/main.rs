//! Entry point for the axum build.
//!
//! Same CLI surface as the zero-crate binary: `serve`, `seed`,
//! `hash-password`, the same environment variables, the same data directory.
//! The only difference is that hyper/tokio own the socket and the request loop.

use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use rudolfzinovev_axum::state::{ActiveGuard, AppState};
use rudolfzinovev_axum::{crypto, db::Store, handlers};

fn env_or(key: &str, fallback: &str) -> String {
    std::env::var(key)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

fn usage() -> ! {
    eprintln!(
        "rudolfzinovev-axum {}\n\n\
         USAGE:\n  rudolfzinovev-axum [serve] [--bind ADDR] [--data DIR] [--owner NAME] [--password PW]\n\
         \x20                      [--workers N] [--no-gzip] [--seed]\n  \
         rudolfzinovev-axum hash-password <password>\n  \
         rudolfzinovev-axum seed\n\n\
         ENV:\n  RZ_BIND (0.0.0.0:8080)   RZ_DATA (./data)   RZ_OWNER (owner)\n  \
         RZ_PASSWORD               RZ_WORKERS (CPU count)  RZ_GZIP (1)  RZ_SEED (1)\n",
        env!("CARGO_PKG_VERSION")
    );
    std::process::exit(2)
}

/// Client address for the limiters. Behind Caddy, `X-Forwarded-For` carries the
/// real peer; the first hop is taken because Caddy appends, it does not rewrite.
fn client_ip(headers: &HeaderMap) -> String {
    if let Some(fwd) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        if let Some(first) = fwd.split(',').next() {
            let ip = first.trim();
            if !ip.is_empty() {
                return ip.to_string();
            }
        }
    }
    "unknown".to_string()
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut bind = env_or("RZ_BIND", "0.0.0.0:8080");
    let mut data_dir = PathBuf::from(env_or("RZ_DATA", "./data"));
    let mut owner = env_or("RZ_OWNER", "owner");
    let mut password = std::env::var("RZ_PASSWORD").ok().filter(|p| !p.is_empty());
    let mut workers = env_or("RZ_WORKERS", "")
        .parse::<usize>()
        .ok()
        .or_else(|| std::thread::available_parallelism().ok().map(|n| n.get()))
        .unwrap_or(2);
    let mut gzip = env_or("RZ_GZIP", "1") != "0";
    let mut seed = env_or("RZ_SEED", "1") != "0";
    let mut mode = "serve".to_string();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "serve" => mode = "serve".into(),
            "seed" => mode = "seed".into(),
            "hash-password" => {
                let pw = args.get(i + 1).cloned().unwrap_or_else(|| usage());
                println!("{}", crypto::hash_password(&pw));
                return;
            }
            "--version" | "-V" => {
                println!("rudolfzinovev-axum {}", env!("CARGO_PKG_VERSION"));
                return;
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
                workers = args.get(i).and_then(|v| v.parse().ok()).unwrap_or(workers);
            }
            "--no-gzip" => gzip = false,
            "--seed" => seed = true,
            "--help" | "-h" => usage(),
            other => {
                eprintln!("unknown argument: {other}");
                usage();
            }
        }
        i += 1;
    }

    if mode == "seed" {
        let store = match Store::open(&data_dir.join("site.sqlite")) {
            Ok(s) => s,
            Err(err) => {
                eprintln!("open: {err}");
                std::process::exit(1);
            }
        };
        match store.seed_if_empty() {
            Ok(true) => println!("seeded: {} articles", store.article_count().unwrap_or(0)),
            Ok(false) => println!("already seeded"),
            Err(err) => {
                eprintln!("seed failed: {err}");
                std::process::exit(1);
            }
        }
        return;
    }

    let (state, generated) = match AppState::boot(&data_dir, &owner, password.as_deref(), seed) {
        Ok(pair) => pair,
        Err(err) => {
            eprintln!("bootstrap failed: {err}");
            std::process::exit(1);
        }
    };
    if let Some(pw) = generated {
        eprintln!(
            "\n  OWNER BOOTSTRAP — a password was generated, store it now:\n\n    {pw}\n\n  \
             It is printed once. Change it in /cabinet/security.\n"
        );
    }
    let shared = Arc::new(state);

    // axum spawns one task per connection on the tokio runtime; `worker_threads`
    // is the pool size, so --workers keeps meaning what it meant before.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers.max(1))
        .enable_all()
        .build()
        .expect("tokio runtime");
    let _guard = runtime.enter();

    let app = handlers::router(shared.clone(), gzip);
    let app = with_health(app, shared.clone());

    let listener = match tokio::net::TcpListener::bind(&bind).await {
        Ok(l) => l,
        Err(err) => {
            eprintln!("bind {bind}: {err}");
            std::process::exit(1);
        }
    };
    let local = listener.local_addr().map(|a| a.to_string()).unwrap_or_else(|_| bind.clone());
    println!(
        "rudolfzinovev.me (axum) {} · listening on {local} · workers {workers} · gzip {} · db {}",
        env!("CARGO_PKG_VERSION"),
        if gzip { "on" } else { "off" },
        data_dir.join("site.sqlite").display()
    );

    if let Err(err) = axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
    {
        eprintln!("server error: {err}");
    }
}

/// `/healthz` is registered here rather than in `handlers` because it reports
/// runtime facts that only `main` owns (the pool and the process).
fn with_health(app: Router, state: Arc<AppState>) -> Router {
    app.route(
        "/healthz",
        get(move |headers: HeaderMap, uri: Uri| {
            let state = state.clone();
            async move {
                let _active = ActiveGuard::new(&state.active);
                let body = format!(
                    "ok {} requests={} active={} uptime={}s\n",
                    rudolfzinovev_axum::sqlite::Connection::sqlite_version(),
                    state.requests.load(std::sync::atomic::Ordering::Relaxed),
                    state.active.load(std::sync::atomic::Ordering::Relaxed),
                    state.uptime_secs()
                );
                let _ = (headers, uri);
                Response::builder()
                    .status(200)
                    .header("content-type", "text/plain; charset=utf-8")
                    .header("cache-control", "no-store")
                    .body(axum::body::Body::from(body))
                    .unwrap_or_else(|_| "ok\n".into_response())
                    .into_response()
            }
        }),
    )
}
