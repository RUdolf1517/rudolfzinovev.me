//! rudolfzinovev.me — a personal website and owner cabinet.
//!
//! Pure Rust, zero external crates. Links against the system `libsqlite3`,
//! `libcrypto` (OpenSSL 3) and `libz`. See ARCHITECTURE.md.

mod app;
mod crypto;
mod db;
mod http;
mod i18n;
mod markdown;
mod models;
mod sqlite;
mod templates;

use std::path::PathBuf;
use std::sync::Arc;

use crate::app::App;
use crate::http::{Server, ServerConfig};

fn env_or(key: &str, fallback: &str) -> String {
    std::env::var(key)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

fn usage() -> ! {
    eprintln!(
        "rudolfzinovev.me {}\n\n\
         USAGE:\n  rudolfzinovev [serve] [--bind ADDR] [--data DIR] [--owner NAME] [--password PW]\n\
                        [--workers N] [--no-gzip] [--seed]\n  \
         rudolfzinovev hash-password <password>\n  \
         rudolfzinovev seed\n\n\
         ENV:\n  RZ_BIND (0.0.0.0:8080)   RZ_DATA (./data)   RZ_OWNER (owner)\n  \
         RZ_PASSWORD               RZ_WORKERS (CPU count)  RZ_GZIP (1)  RZ_SEED (1)\n",
        env!("CARGO_PKG_VERSION")
    );
    std::process::exit(2)
}

fn main() {
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
            "serve" => i += 1,
            "hash-password" => {
                let pw = args.get(i + 1).cloned().unwrap_or_else(|| {
                    eprintln!("hash-password requires a value");
                    std::process::exit(2)
                });
                println!("{}", crypto::hash_password(&pw));
                return;
            }
            "seed" => {
                mode = "seed".into();
                i += 1;
            }
            "--bind" => {
                bind = args.get(i + 1).cloned().unwrap_or_else(|| usage());
                i += 2;
            }
            "--data" => {
                data_dir = PathBuf::from(args.get(i + 1).cloned().unwrap_or_else(|| usage()));
                i += 2;
            }
            "--owner" => {
                owner = args.get(i + 1).cloned().unwrap_or_else(|| usage());
                i += 2;
            }
            "--password" => {
                password = args.get(i + 1).cloned();
                i += 2;
            }
            "--workers" => {
                workers = args
                    .get(i + 1)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or_else(|| usage());
                i += 2;
            }
            "--no-gzip" => {
                gzip = false;
                i += 1;
            }
            "--seed" => {
                seed = true;
                i += 1;
            }
            "--help" | "-h" => usage(),
            "--version" | "-V" => {
                println!("{}", env!("CARGO_PKG_VERSION"));
                return;
            }
            other => {
                eprintln!("unknown argument: {other}");
                usage();
            }
        }
    }

    std::fs::create_dir_all(&data_dir).unwrap_or_else(|e| {
        eprintln!("cannot create data dir {}: {e}", data_dir.display());
        std::process::exit(1)
    });

    let app = match App::bootstrap(&data_dir, &owner, password.as_deref(), seed) {
        Ok(app) => Arc::new(app),
        Err(err) => {
            eprintln!("bootstrap failed: {err}");
            std::process::exit(1);
        }
    };

    if let Some(generated) = app.generated_password() {
        println!("──────────────────────────────────────────────");
        println!(" OWNER BOOTSTRAP — save this password now");
        println!("   user:     {owner}");
        println!("   password: {generated}");
        println!("──────────────────────────────────────────────");
    }

    if mode == "seed" {
        println!("articles: {}", app.store.article_count().unwrap_or(0));
        return;
    }

    let handler = Arc::new(move |req: &http::Request| app.handle(req));

    let server = match Server::bind(
        &bind,
        handler,
        ServerConfig {
            workers,
            gzip_enabled: gzip,
        },
    ) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    };

    println!(
        "rudolfzinovev.me {} · listening on {} · workers {} · gzip {} · db {}",
        env!("CARGO_PKG_VERSION"),
        server.local_addr(),
        workers,
        if gzip { "on" } else { "off" },
        data_dir.join("site.sqlite").display()
    );

    if let Err(err) = server.run() {
        eprintln!("server stopped: {err}");
        std::process::exit(1);
    }
}
