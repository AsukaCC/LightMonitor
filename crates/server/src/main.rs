mod auth;
mod config;
mod credential;
mod db;
mod geo;
mod local_monitor;
mod models;
mod probe;
mod routes;
mod ssh_keys;
mod state;
mod updater;

use crate::config::Config;
use crate::db::Db;
use crate::state::AppState;
use axum::Router;
use axum::routing::{delete, get, post, put};
use futures_util::stream::{self, StreamExt};
use std::net::SocketAddr;
use std::time::Duration;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()))
        .init();

    let config = Config::from_env()?;
    std::fs::create_dir_all(&config.data_dir)?;
    std::fs::create_dir_all(&config.releases_dir)?;
    std::fs::create_dir_all(&config.versions_dir)?;
    std::fs::create_dir_all(&config.ssh_keys_dir)?;

    let db = Db::open(&config.db_path())?;
    let state = AppState::new(db, config.clone());

    let local_address = local_monitor::local_ip().unwrap_or_else(|| "127.0.0.1".to_string());
    let local_region = {
        let region = geo::resolve_local_region(&local_address).await;
        if region.is_empty() {
            geo::resolve_region(&local_address).await
        } else {
            region
        }
    };
    state
        .db
        .ensure_system_host_with_details(&local_address, &local_region)?;
    spawn_local_monitor(state.clone());
    spawn_offline_watcher(state.clone());
    spawn_probe_watcher(state.clone());

    let api = Router::new()
        .route("/health", get(routes::health))
        .route("/auth/login", post(routes::login))
        .route("/auth/logout", post(routes::logout))
        .route("/auth/session", get(routes::session))
        .route(
            "/ssh-keys",
            get(routes::list_ssh_keys).post(routes::upload_ssh_key),
        )
        .route(
            "/ssh-keys/{id}",
            put(routes::update_ssh_key).delete(routes::delete_ssh_key),
        )
        .route("/public/hosts", get(routes::list_public_hosts))
        .route("/system/releases", get(routes::list_app_releases))
        .route("/system/releases/apply", post(routes::apply_app_release))
        .route(
            "/system/releases/{version}",
            delete(routes::delete_app_release),
        )
        .route(
            "/hosts",
            get(routes::list_hosts)
                .post(routes::create_host)
                .delete(routes::delete_hosts),
        )
        .route("/hosts/update-interval", put(routes::update_host_intervals))
        .route(
            "/hosts/{id}/metrics-history",
            get(routes::get_metric_history),
        )
        .route("/hosts/{id}", put(routes::update_host))
        .route("/hosts/{id}/domains", post(routes::add_host_domain))
        .route(
            "/hosts/{id}/domains/{domain_id}",
            delete(routes::delete_host_domain),
        )
        .route("/hosts/{id}/probe", post(routes::probe_host_now))
        .route("/hosts/{id}/agent-token", get(routes::get_agent_token))
        .route("/hosts/{id}/install-agent", post(routes::install_agent))
        .route("/agents/register", post(routes::register_agent))
        .route("/agents/metrics", post(routes::report_metrics));

    let mut app = Router::new()
        .nest("/api", api)
        .route("/events", get(routes::events))
        .nest_service("/releases", ServeDir::new(&config.releases_dir))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .with_state(state);

    if config.web_dir.exists() {
        let index = config.web_dir.join("index.html");
        let static_files = ServeDir::new(&config.web_dir).not_found_service(ServeFile::new(index));
        app = app.fallback_service(static_files);
    } else {
        tracing::warn!(
            "web dir {} not found; API-only mode",
            config.web_dir.display()
        );
    }

    let addr: SocketAddr = config.listen_addr().parse()?;
    if config.public_url.is_empty() {
        tracing::warn!("LightMonitor listening on http://{addr} (public_url=auto from request)");
    } else {
        tracing::warn!(
            "LightMonitor listening on http://{addr} (public_url={})",
            config.public_url
        );
    }

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

fn spawn_local_monitor(state: AppState) {
    tokio::spawn(async move {
        let mut collector = local_monitor::LocalCollector::new();
        loop {
            let sample = collector.collect();
            let interval_seconds = match state.db.apply_system_metrics(&sample) {
                Ok(host) => {
                    let interval_seconds = host.update_interval_seconds;
                    state.publish(models::ServerEvent::HostUpdated {
                        host: Box::new(host),
                    });
                    interval_seconds
                }
                Err(err) => {
                    tracing::warn!("local host metrics collection failed: {err}");
                    5
                }
            };
            tokio::time::sleep(Duration::from_secs(interval_seconds.clamp(1, 3600))).await;
        }
    });
}

fn spawn_offline_watcher(state: AppState) {
    let seconds = state.config.offline_seconds;
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        loop {
            interval.tick().await;
            match state.db.mark_offline(seconds) {
                Ok(hosts) => {
                    for host in hosts {
                        state.publish(models::ServerEvent::HostUpdated {
                            host: Box::new(host),
                        });
                    }
                }
                Err(err) => tracing::warn!("offline scan failed: {err}"),
            }
        }
    });
}

fn spawn_probe_watcher(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            let hosts = match state.db.list_hosts() {
                Ok(hosts) => hosts,
                Err(error) => {
                    tracing::warn!("host probe list failed: {error}");
                    continue;
                }
            };
            stream::iter(hosts)
                .for_each_concurrent(8, |host| {
                    let state = state.clone();
                    async move {
                        match probe::refresh_host(&state, &host).await {
                            Ok(host) => state.publish(models::ServerEvent::HostUpdated {
                                host: Box::new(host),
                            }),
                            Err(error) => tracing::warn!("host probe failed: {error}"),
                        }
                    }
                })
                .await;
        }
    });
}
