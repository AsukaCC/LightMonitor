use crate::auth::{ApiError, AuthUser, random_token};
use crate::models::{
    AgentConfigResponse, AgentTokenResponse, ApplyReleaseRequest, ApplyReleaseResponse,
    CreateHostRequest, DeleteHostsRequest, Host, HostStatus, InstallAgentRequest, InstallLog,
    LoginRequest, LoginResponse, MetricHistoryQuery, MetricHistoryResponse, MetricReport,
    RegisterAgentRequest, RegisterAgentResponse, ReleaseCatalog, ServerEvent, SessionResponse,
    SshKey, UpdateHostIntervalRequest, UpdateHostRequest,
};
use crate::state::AppState;
use axum::Json;
use axum::extract::{Multipart, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use chrono::{Duration, Utc};
use std::convert::Infallible;
use std::fs;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use uuid::Uuid;

pub async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") }))
}

pub async fn list_app_releases(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<ReleaseCatalog>, ApiError> {
    crate::updater::release_catalog(&state)
        .await
        .map(Json)
        .map_err(|error| ApiError::bad_gateway(error.to_string()))
}

pub async fn apply_app_release(
    State(state): State<AppState>,
    _user: AuthUser,
    Json(body): Json<ApplyReleaseRequest>,
) -> Result<(StatusCode, Json<ApplyReleaseResponse>), ApiError> {
    if !state.config.managed_updates {
        return Err(ApiError::bad_request(
            "managed updates are disabled for this deployment",
        ));
    }

    let version = body.version.trim();
    if version.is_empty() {
        return Err(ApiError::bad_request("version is required"));
    }

    let guard = state
        .update_lock
        .clone()
        .try_lock_owned()
        .map_err(|_| ApiError::conflict("another version change is in progress"))?;
    let selected = crate::updater::install_and_activate(&state, version)
        .await
        .map_err(|error| ApiError::bad_gateway(error.to_string()))?;
    drop(guard);

    tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        std::process::exit(75);
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(ApplyReleaseResponse {
            version: selected,
            restarting: true,
        }),
    ))
}

pub async fn delete_app_release(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(version): Path<String>,
) -> Result<StatusCode, ApiError> {
    if !state.config.managed_updates {
        return Err(ApiError::bad_request(
            "managed updates are disabled for this deployment",
        ));
    }

    let guard = state
        .update_lock
        .clone()
        .try_lock_owned()
        .map_err(|_| ApiError::conflict("another version change is in progress"))?;
    let deleted = crate::updater::delete_downloaded_version(&state, &version).map_err(|error| {
        let message = error.to_string();
        if message.contains("invalid release version")
            || message.contains("active version")
            || message.contains("invalid downloaded version")
        {
            ApiError::bad_request(message)
        } else {
            ApiError::internal(message)
        }
    })?;
    drop(guard);

    if !deleted {
        return Err(ApiError::not_found(format!(
            "downloaded version {} was not found",
            version.trim()
        )));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Server-to-browser event stream. SSE keeps this channel one-way: the
/// browser receives updates but cannot send messages over the stream.
pub async fn events(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let receiver = state.events.subscribe();
    let stream = futures_util::stream::unfold(receiver, |mut receiver| async move {
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    let event = match Event::default().json_data(event) {
                        Ok(event) => event,
                        Err(_) => continue,
                    };
                    return Some((Ok(event), receiver));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    });

    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(std::time::Duration::from_secs(20))
            .text("keep-alive"),
    ))
}

pub async fn login(
    State(state): State<AppState>,
    Json(body): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, ApiError> {
    if body.username != state.config.admin_username || body.password != state.config.admin_password
    {
        return Err(ApiError::unauthorized("invalid username or password"));
    }
    let token = random_token();
    state
        .db
        .create_session(&token, &body.username, state.config.session_ttl_hours)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(LoginResponse {
        token,
        username: body.username,
    }))
}

pub async fn logout(State(state): State<AppState>, user: AuthUser) -> Result<StatusCode, ApiError> {
    state
        .db
        .delete_session(&user.token)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn session(user: AuthUser) -> Json<SessionResponse> {
    Json(SessionResponse {
        username: user.username,
    })
}

pub async fn list_ssh_keys(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<Vec<SshKey>>, ApiError> {
    state
        .db
        .list_ssh_keys()
        .map(Json)
        .map_err(|error| ApiError::internal(error.to_string()))
}

pub async fn upload_ssh_key(
    State(state): State<AppState>,
    _user: AuthUser,
    multipart: Multipart,
) -> Result<Json<SshKey>, ApiError> {
    let (name, fallback_name, contents) = read_ssh_key_upload(multipart).await?;
    let name = crate::ssh_keys::validate_name(
        name.as_deref()
            .or(fallback_name.as_deref())
            .unwrap_or("SSH key"),
    )
    .map_err(|error| ApiError::bad_request(error.to_string()))?;
    crate::ssh_keys::validate_contents(&contents)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;

    let id = Uuid::new_v4();
    let path = crate::ssh_keys::path_for(&state.config.ssh_keys_dir, id);
    crate::ssh_keys::write_private(&path, &contents)
        .map_err(|error| ApiError::internal(error.to_string()))?;
    if let Err(error) =
        state
            .db
            .create_ssh_key(id, &name, &path.to_string_lossy(), contents.len() as u64)
    {
        let _ = fs::remove_file(&path);
        return Err(ApiError::internal(error.to_string()));
    }

    let key = state
        .db
        .list_ssh_keys()
        .map_err(|error| ApiError::internal(error.to_string()))?
        .into_iter()
        .find(|key| key.id == id)
        .ok_or_else(|| ApiError::internal("uploaded SSH key disappeared"))?;
    Ok(Json(key))
}

pub async fn update_ssh_key(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<Uuid>,
    multipart: Multipart,
) -> Result<Json<SshKey>, ApiError> {
    let (name, _fallback_name, contents) = read_ssh_key_upload(multipart).await?;
    let (existing_name, storage_path) = state
        .db
        .get_ssh_key(id)
        .map_err(|error| ApiError::internal(error.to_string()))?
        .ok_or_else(|| ApiError::not_found("SSH key not found"))?;
    let name = match name.as_deref() {
        Some(name) => crate::ssh_keys::validate_name(name),
        None => Ok(existing_name),
    }
    .map_err(|error| ApiError::bad_request(error.to_string()))?;
    crate::ssh_keys::validate_contents(&contents)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let path = std::path::PathBuf::from(&storage_path);
    if !path.starts_with(&state.config.ssh_keys_dir) {
        return Err(ApiError::internal(
            "SSH key storage path is outside the key directory",
        ));
    }
    crate::ssh_keys::write_private(&path, &contents)
        .map_err(|error| ApiError::internal(error.to_string()))?;
    if !state
        .db
        .update_ssh_key(id, &name, contents.len() as u64)
        .map_err(|error| ApiError::internal(error.to_string()))?
    {
        return Err(ApiError::not_found("SSH key not found"));
    }
    let key = state
        .db
        .list_ssh_keys()
        .map_err(|error| ApiError::internal(error.to_string()))?
        .into_iter()
        .find(|key| key.id == id)
        .ok_or_else(|| ApiError::internal("updated SSH key disappeared"))?;
    Ok(Json(key))
}

pub async fn delete_ssh_key(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let path = state.db.delete_ssh_key(id).map_err(|error| {
        let message = error.to_string();
        if message.contains("used by") {
            ApiError::conflict(message)
        } else {
            ApiError::internal(message)
        }
    })?;
    let Some(path) = path else {
        return Err(ApiError::not_found("SSH key not found"));
    };
    let path = std::path::PathBuf::from(path);
    if !path.starts_with(&state.config.ssh_keys_dir) {
        return Err(ApiError::internal(
            "SSH key storage path is outside the key directory",
        ));
    }
    if path.exists() {
        fs::remove_file(path).map_err(|error| ApiError::internal(error.to_string()))?;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn read_ssh_key_upload(
    mut multipart: Multipart,
) -> Result<(Option<String>, Option<String>, Vec<u8>), ApiError> {
    let mut name = None;
    let mut fallback_name = None;
    let mut contents = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| ApiError::bad_request(format!("invalid SSH key upload: {error}")))?
    {
        match field.name() {
            Some("name") => {
                name = Some(field.text().await.map_err(|error| {
                    ApiError::bad_request(format!("invalid SSH key name: {error}"))
                })?);
            }
            Some("file") => {
                fallback_name = field.file_name().map(str::to_string);
                contents = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|error| {
                            ApiError::bad_request(format!("invalid SSH key file: {error}"))
                        })?
                        .to_vec(),
                );
            }
            _ => {}
        }
    }
    let contents = contents.ok_or_else(|| ApiError::bad_request("SSH key file is required"))?;
    Ok((name, fallback_name, contents))
}

pub async fn list_hosts(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<Vec<Host>>, ApiError> {
    let hosts = state
        .db
        .list_hosts()
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(hosts))
}

pub async fn list_public_hosts(
    State(state): State<AppState>,
) -> Result<Json<Vec<crate::models::PublicHost>>, ApiError> {
    let hosts = state
        .db
        .list_hosts()
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(hosts.iter().map(Host::to_public).collect()))
}

pub async fn create_host(
    State(state): State<AppState>,
    _user: AuthUser,
    Json(mut body): Json<CreateHostRequest>,
) -> Result<Json<Host>, ApiError> {
    if body.name.trim().is_empty() || body.address.trim().is_empty() {
        return Err(ApiError::bad_request("name and address are required"));
    }
    body.region = body.region.trim().to_string();
    if body.region.is_empty() {
        body.region = crate::geo::resolve_region(&body.address).await;
    }
    let agent_token = random_token();
    let host = state
        .db
        .create_host(body, agent_token)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    state.publish(ServerEvent::HostUpdated {
        host: Box::new(host.clone()),
    });
    Ok(Json(host))
}

pub async fn update_host(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<Uuid>,
    Json(mut body): Json<UpdateHostRequest>,
) -> Result<Json<Host>, ApiError> {
    body.region = body.region.trim().to_string();
    // Empty region on update: re-resolve from address (address may have changed)
    if body.region.is_empty() {
        body.region = crate::geo::resolve_region(&body.address).await;
    }
    let host = state
        .db
        .update_host(id, body)
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("host not found"))?;
    state.publish(ServerEvent::HostUpdated {
        host: Box::new(host.clone()),
    });
    Ok(Json(host))
}

pub async fn delete_hosts(
    State(state): State<AppState>,
    _user: AuthUser,
    Json(body): Json<DeleteHostsRequest>,
) -> Result<StatusCode, ApiError> {
    if state
        .db
        .contains_system_host(&body.ids)
        .map_err(|e| ApiError::internal(e.to_string()))?
    {
        return Err(ApiError::bad_request("宿主机为系统内置主机，无法删除"));
    }

    if !body.force {
        let mut targets = Vec::new();
        for id in &body.ids {
            let Some(host) = state
                .db
                .get_host(*id)
                .map_err(|e| ApiError::internal(e.to_string()))?
            else {
                continue;
            };
            let (password, saved_key_path) = state
                .db
                .ssh_credentials(*id)
                .map_err(|e| ApiError::internal(e.to_string()))?
                .unwrap_or_default();
            let key_path = if !saved_key_path.is_empty() {
                saved_key_path
            } else {
                "/root/.ssh/id_rsa".to_string()
            };
            if host.agent_id.is_some() {
                if host.ssh_user.trim().is_empty() {
                    return Err(ApiError::bad_request(format!(
                        "主机「{}」缺少 SSH 账号，无法自动卸载探针",
                        host.name
                    )));
                }
                if password.is_empty() && !std::path::Path::new(&key_path).is_file() {
                    return Err(ApiError::bad_request(format!(
                        "主机「{}」缺少可用的 SSH 密码或密钥，无法自动卸载探针",
                        host.name
                    )));
                }
            }
            targets.push((host, key_path, password));
        }

        for (host, key_path, password) in targets {
            if host.agent_id.is_some() {
                run_ssh_script(
                    &host.address,
                    host.ssh_port,
                    &host.ssh_user,
                    &key_path,
                    &password,
                    build_uninstall_script(),
                )
                .await
                .map_err(|err| {
                    ApiError::bad_request(format!("主机「{}」探针自动卸载失败：{err}", host.name))
                })?;
            }
        }
    }

    let deleted = state
        .db
        .delete_hosts(&body.ids)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    if !deleted.is_empty() {
        state.publish(ServerEvent::HostsDeleted { host_ids: deleted });
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn update_host_intervals(
    State(state): State<AppState>,
    _user: AuthUser,
    Json(body): Json<UpdateHostIntervalRequest>,
) -> Result<Json<Vec<Host>>, ApiError> {
    if body.ids.is_empty() {
        return Err(ApiError::bad_request("select at least one host"));
    }
    if !(1..=3600).contains(&body.interval_seconds) {
        return Err(ApiError::bad_request(
            "interval_seconds must be between 1 and 3600",
        ));
    }

    let hosts = state
        .db
        .update_host_intervals(&body.ids, body.interval_seconds)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    if hosts.is_empty() {
        return Err(ApiError::not_found("hosts not found"));
    }
    for host in &hosts {
        state.publish(ServerEvent::HostUpdated {
            host: Box::new(host.clone()),
        });
    }
    Ok(Json(hosts))
}

pub async fn get_metric_history(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<Uuid>,
    Query(query): Query<MetricHistoryQuery>,
) -> Result<Json<MetricHistoryResponse>, ApiError> {
    if state
        .db
        .get_host(id)
        .map_err(|e| ApiError::internal(e.to_string()))?
        .is_none()
    {
        return Err(ApiError::not_found("host not found"));
    }

    let range = query.range.as_deref().unwrap_or("1h");
    let hours = match range {
        "1h" => 1,
        "4h" => 4,
        "6h" => 6,
        "12h" => 12,
        "1d" => 24,
        _ => {
            return Err(ApiError::bad_request(
                "range must be one of 1h, 4h, 6h, 12h, 1d",
            ));
        }
    };
    let points = state
        .db
        .metric_history(id, Utc::now() - Duration::hours(hours), 360)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(MetricHistoryResponse {
        range: range.to_string(),
        points,
    }))
}

pub async fn get_agent_token(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<AgentTokenResponse>, ApiError> {
    let host = state
        .db
        .get_host(id)
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("host not found"))?;
    if host.is_system {
        return Err(ApiError::bad_request("宿主机使用内置采集，无需安装探针"));
    }
    let agent_token = state
        .db
        .agent_token(id)
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("host not found"))?;
    // Prefer request URL (browser Host/Origin); optional LIGHTMONITOR_PUBLIC_URL overrides.
    let server_url = resolve_server_url(&state.config.public_url, &headers)?;
    // Binary is fetched from GitHub Releases; --server-url is only for agent API reporting.
    let install_command = format!(
        "curl -fsSL https://raw.githubusercontent.com/AsukaCC/LightMonitor/main/scripts/install-agent.sh | sudo bash -s -- --server-url {server_url} --token {agent_token}"
    );
    Ok(Json(AgentTokenResponse {
        host_id: id,
        agent_token,
        install_command,
    }))
}

pub async fn register_agent(
    State(state): State<AppState>,
    Json(body): Json<RegisterAgentRequest>,
) -> Result<Json<RegisterAgentResponse>, ApiError> {
    let result = state
        .db
        .register_agent(&body.token, &body.hostname, body.address.as_deref())
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::unauthorized("invalid agent token"))?;

    let (agent_id, host) = result;
    let interval_seconds = host.update_interval_seconds;
    state.publish(ServerEvent::HostUpdated {
        host: Box::new(host),
    });
    Ok(Json(RegisterAgentResponse {
        agent_id,
        interval_seconds,
    }))
}

pub async fn report_metrics(
    State(state): State<AppState>,
    Json(body): Json<MetricReport>,
) -> Result<Json<AgentConfigResponse>, ApiError> {
    let host = state
        .db
        .apply_metrics(body.agent_id, &body.token, &body.sample)
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::unauthorized("invalid agent credentials"))?;
    let interval_seconds = host.update_interval_seconds;
    state.publish(ServerEvent::HostUpdated {
        host: Box::new(host),
    });
    Ok(Json(AgentConfigResponse { interval_seconds }))
}

pub async fn install_agent(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<InstallAgentRequest>,
) -> Result<Json<Host>, ApiError> {
    let mut key_path = body.ssh_key_path.trim().to_string();
    let mut password = body.ssh_password.trim().to_string();

    let existing = state
        .db
        .get_host(id)
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("host not found"))?;
    if existing.is_system {
        return Err(ApiError::bad_request("宿主机使用内置采集，无需安装探针"));
    }

    let (stored_password, stored_identity) = state
        .db
        .ssh_credentials(id)
        .map_err(|e| ApiError::internal(e.to_string()))?
        .unwrap_or_default();
    if let Some(key_id) = body.ssh_key_id {
        let (_, managed_path) = state
            .db
            .get_ssh_key(key_id)
            .map_err(|e| ApiError::internal(e.to_string()))?
            .ok_or_else(|| ApiError::bad_request("selected SSH key no longer exists"))?;
        key_path = managed_path;
        password.clear();
    } else if body.use_saved_identity {
        if stored_identity.is_empty() {
            return Err(ApiError::bad_request(
                "no saved SSH identity file is available for this host",
            ));
        }
        key_path = stored_identity;
        password.clear();
    } else if key_path.is_empty() && password.is_empty() {
        password = stored_password;
    }

    if existing.ssh_user.trim().is_empty() {
        return Err(ApiError::bad_request(
            "no SSH account configured; edit the host to set an SSH account, or use the manual install command",
        ));
    }
    if key_path.is_empty() && password.is_empty() {
        return Err(ApiError::bad_request(
            "no SSH password on host and none provided; edit host to set password, or use key path / install form password",
        ));
    }
    if !key_path.is_empty() && !std::path::Path::new(&key_path).is_file() && password.is_empty() {
        return Err(ApiError::bad_request(format!(
            "SSH key not found: {key_path}. Mount host keys (LIGHTMONITOR_SSH_DIR) or set host SSH password."
        )));
    }

    let agent_token = state
        .db
        .agent_token(id)
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("host not found"))?;

    let public_url = resolve_server_url(&state.config.public_url, &headers)?;
    if is_loopback_public_url(&public_url) {
        return Err(ApiError::bad_request(format!(
            "resolved server URL is {public_url} (localhost). \
Remote agents cannot reach loopback. Open the admin UI via a host IP/domain \
the target can reach, or set LIGHTMONITOR_PUBLIC_URL (e.g. http://你的公网IP:8080) \
and retry."
        )));
    }
    if is_placeholder_public_url(&public_url) {
        return Err(ApiError::bad_request(format!(
            "resolved server URL is still a placeholder ({public_url}). \
Open the admin UI via a real IP/domain, or set LIGHTMONITOR_PUBLIC_URL."
        )));
    }

    // Only enter Installing after all synchronous validation passes. Otherwise a
    // rejected request would leave the host stuck in an in-progress state.
    let host = state
        .db
        .set_status(id, HostStatus::Installing)
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("host not found"))?;
    state.publish(ServerEvent::HostUpdated {
        host: Box::new(host.clone()),
    });

    let script = build_install_script(&public_url, &agent_token);

    let result = run_ssh_script(
        &existing.address,
        existing.ssh_port,
        &existing.ssh_user,
        &key_path,
        &password,
        &script,
    )
    .await;

    match result {
        Ok(output) => {
            if !key_path.is_empty() {
                state
                    .db
                    .set_ssh_key_path(id, &key_path)
                    .map_err(|e| ApiError::internal(e.to_string()))?;
            }
            let summary = summarize_install_output(&output);
            push_log(&state, id, true, summary)?;
            let host = state
                .db
                .set_status(id, HostStatus::Pending)
                .map_err(|e| ApiError::internal(e.to_string()))?
                .ok_or_else(|| ApiError::not_found("host not found"))?;
            state.publish(ServerEvent::HostUpdated {
                host: Box::new(host.clone()),
            });
            Ok(Json(host))
        }
        Err(err) => {
            push_log(&state, id, false, format!("install failed: {err}"))?;
            let host = state
                .db
                .set_status(id, HostStatus::Error)
                .map_err(|e| ApiError::internal(e.to_string()))?
                .ok_or_else(|| ApiError::not_found("host not found"))?;
            state.publish(ServerEvent::HostUpdated {
                host: Box::new(host.clone()),
            });
            Err(ApiError::bad_request(err))
        }
    }
}

fn summarize_install_output(output: &str) -> String {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return "install finished".to_string();
    }
    // Keep last non-empty line to avoid dumping full systemd status into UI logs.
    trimmed
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.trim().to_string())
        .unwrap_or_else(|| "install finished".to_string())
}

fn push_log(state: &AppState, host_id: Uuid, ok: bool, message: String) -> Result<(), ApiError> {
    let log = InstallLog {
        at: Utc::now(),
        ok,
        message,
    };
    state
        .db
        .append_install_log(host_id, &log)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    state.publish(ServerEvent::InstallLog {
        host_id,
        log: log.clone(),
    });
    if let Ok(Some(host)) = state.db.get_host(host_id) {
        state.publish(ServerEvent::HostUpdated {
            host: Box::new(host),
        });
    }
    Ok(())
}

/// Resolve agent callback base URL.
/// 1) Explicit usable `LIGHTMONITOR_PUBLIC_URL` (not empty / placeholder / loopback)
/// 2) Else auto from current request (Origin / X-Forwarded-* / Host)
fn resolve_server_url(configured: &str, headers: &HeaderMap) -> Result<String, ApiError> {
    if let Some(url) = usable_configured_public_url(configured) {
        return Ok(url);
    }
    if let Some(url) = public_url_from_request(headers) {
        return Ok(url);
    }
    if let Some(url) = normalize_public_url(configured)
        && !is_placeholder_public_url(&url)
    {
        return Ok(url);
    }
    Err(ApiError::bad_request(
        "cannot determine server URL from request; \
open the admin UI via a reachable host, or set LIGHTMONITOR_PUBLIC_URL",
    ))
}

fn usable_configured_public_url(configured: &str) -> Option<String> {
    let url = normalize_public_url(configured)?;
    if is_placeholder_public_url(&url) || is_loopback_public_url(&url) {
        return None;
    }
    Some(url)
}

fn public_url_from_request(headers: &HeaderMap) -> Option<String> {
    if let Some(origin) = header_first(headers, "origin") {
        let origin = origin.trim();
        if origin != "null"
            && origin.contains("://")
            && let Some(url) = normalize_public_url(origin)
            && !is_placeholder_public_url(&url)
        {
            return Some(url);
        }
    }

    if let Some(referer) = header_first(headers, "referer")
        && let Some(url) = normalize_public_url(referer)
        && !is_placeholder_public_url(&url)
    {
        return Some(url);
    }

    let host =
        header_first(headers, "x-forwarded-host").or_else(|| header_first(headers, "host"))?;
    let host = host.split(',').next()?.trim();
    if host.is_empty() {
        return None;
    }
    // Reject garbage like "jiangcheng.site/:8080" in Host
    let host = fix_host_with_slash_port(host);
    let scheme = header_first(headers, "x-forwarded-proto")
        .map(|v| v.split(',').next().unwrap_or(v).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "http".to_string());
    normalize_public_url(&format!("{scheme}://{host}"))
        .filter(|url| !is_placeholder_public_url(url))
}

/// Normalize to `scheme://host[:port]` (no path/query).
/// Also fixes common typo `http://host/:8080` → `http://host:8080`.
fn normalize_public_url(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let fixed = fix_slash_before_port(raw);
    origin_like_from_url(&fixed)
}

fn fix_slash_before_port(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_string();
    };
    // host/:8080/...  →  host:8080/...
    if let Some(idx) = rest.find("/:") {
        let host = &rest[..idx];
        let after = &rest[idx + 2..];
        let port_len = after.chars().take_while(|c| c.is_ascii_digit()).count();
        if port_len > 0 && !host.contains(':') && !host.is_empty() {
            let port = &after[..port_len];
            let tail = &after[port_len..];
            return format!("{scheme}://{host}:{port}{tail}");
        }
    }
    url.to_string()
}

fn fix_host_with_slash_port(host: &str) -> String {
    if let Some(idx) = host.find("/:") {
        let name = &host[..idx];
        let after = &host[idx + 2..];
        let port_len = after.chars().take_while(|c| c.is_ascii_digit()).count();
        if port_len > 0 && !name.contains(':') && !name.is_empty() {
            return format!("{name}:{}", &after[..port_len]);
        }
    }
    // Strip accidental path in Host
    host.split('/').next().unwrap_or(host).to_string()
}

fn origin_like_from_url(url: &str) -> Option<String> {
    let url = url.trim();
    let (scheme, rest) = url.split_once("://")?;
    if scheme.is_empty() {
        return None;
    }
    let mut authority = rest.split(['/', '?', '#']).next()?.trim().to_string();
    if authority.is_empty() {
        return None;
    }
    // host/:8080 without scheme handling already done; still clean host
    authority = fix_host_with_slash_port(&authority);
    if authority.is_empty() {
        return None;
    }
    Some(format!("{scheme}://{authority}"))
}

fn header_first<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

fn is_placeholder_public_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.contains("your-server-ip")
        || lower.contains("your_server_ip")
        || lower.contains("changeme")
        || lower.contains("example.invalid")
}

fn is_loopback_public_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.contains("://127.")
        || lower.contains("://localhost")
        || lower.contains("://[::1]")
        || lower.contains("://0.0.0.0")
}

fn build_install_script(server_url: &str, agent_token: &str) -> String {
    // Prefer GitHub Releases so remote hosts do not need to reach the server's /releases.
    // Optional offline path: install-agent.sh --from-server downloads from PUBLIC_URL/releases.
    let github_repo = std::env::var("LIGHTMONITOR_GITHUB_REPO")
        .unwrap_or_else(|_| "AsukaCC/LightMonitor".to_string());
    let agent_version =
        std::env::var("LIGHTMONITOR_AGENT_VERSION").unwrap_or_else(|_| "latest".to_string());
    format!(
        r#"set -eu
export LIGHTMONITOR_GITHUB_REPO='{github_repo}'
run_installer() {{
  if [ "$(id -u)" -eq 0 ]; then
    bash -s -- "$@"
  else
    sudo -n bash -s -- "$@" || sudo bash -s -- "$@"
  fi
}}
if command -v curl >/dev/null 2>&1; then
  curl -fsSL "https://raw.githubusercontent.com/{github_repo}/main/scripts/install-agent.sh" | \
    run_installer --server-url '{server_url}' --token '{agent_token}' --version '{agent_version}' --repo '{github_repo}'
elif command -v wget >/dev/null 2>&1; then
  wget -qO- "https://raw.githubusercontent.com/{github_repo}/main/scripts/install-agent.sh" | \
    run_installer --server-url '{server_url}' --token '{agent_token}' --version '{agent_version}' --repo '{github_repo}'
else
  echo "curl or wget required" >&2
  exit 1
fi
"#
    )
}

fn build_uninstall_script() -> &'static str {
    r#"set -eu
run_root() {
  if [ "$(id -u)" -eq 0 ]; then
    "$@"
  else
    sudo -n "$@" || sudo "$@"
  fi
}
run_root systemctl disable --now lightmonitor-agent >/dev/null 2>&1 || true
run_root rm -f /etc/systemd/system/lightmonitor-agent.service
run_root systemctl daemon-reload
run_root rm -rf /opt/lightmonitor
echo "LightMonitor agent uninstalled"
"#
}

async fn run_ssh_script(
    address: &str,
    port: u16,
    user: &str,
    key_path: &str,
    password: &str,
    script: &str,
) -> Result<String, String> {
    let key_ok = !key_path.is_empty() && std::path::Path::new(key_path).is_file();
    let use_password = !key_ok && !password.is_empty();

    // Avoid writing known_hosts into a read-only mounted ~/.ssh
    let ssh_common = [
        "-p".to_string(),
        port.to_string(),
        "-o".to_string(),
        "StrictHostKeyChecking=no".to_string(),
        "-o".to_string(),
        "UserKnownHostsFile=/dev/null".to_string(),
        "-o".to_string(),
        "GlobalKnownHostsFile=/dev/null".to_string(),
        "-o".to_string(),
        "ConnectTimeout=15".to_string(),
        "-o".to_string(),
        "LogLevel=ERROR".to_string(),
    ];

    let mut cmd = if use_password {
        // sshpass feeds password non-interactively
        let mut c = Command::new("sshpass");
        c.arg("-p")
            .arg(password)
            .arg("ssh")
            .args(&ssh_common)
            .arg("-o")
            .arg("PreferredAuthentications=password")
            .arg("-o")
            .arg("PubkeyAuthentication=no")
            .arg("-o")
            .arg("NumberOfPasswordPrompts=1");
        c
    } else if key_ok {
        let mut c = Command::new("ssh");
        c.args(&ssh_common)
            .arg("-i")
            .arg(key_path)
            .arg("-o")
            .arg("IdentitiesOnly=yes")
            .arg("-o")
            .arg("BatchMode=yes")
            .arg("-o")
            .arg("PasswordAuthentication=no");
        c
    } else {
        return Err(
            "no usable SSH key or password; mount LIGHTMONITOR_SSH_DIR or fill password".into(),
        );
    };

    let mut child = cmd
        .arg(format!("{user}@{address}"))
        .arg("sh")
        .arg("-s")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if use_password {
                format!(
                    "failed to spawn sshpass/ssh: {e}. Install sshpass on the server host/image."
                )
            } else {
                format!("failed to spawn ssh: {e}")
            }
        })?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(script.as_bytes())
            .await
            .map_err(|e| format!("failed to write remote script: {e}"))?;
    }

    let output = child
        .wait_with_output()
        .await
        .map_err(|e| format!("ssh failed: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if output.status.success() {
        Ok(format!("{stdout}{stderr}"))
    } else {
        Err(format!(
            "ssh exit {}: {}{}",
            output.status.code().unwrap_or(-1),
            stdout,
            stderr
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn uninstall_script_removes_service_and_files() {
        let script = build_uninstall_script();
        assert!(script.contains("systemctl disable --now lightmonitor-agent"));
        assert!(script.contains("/etc/systemd/system/lightmonitor-agent.service"));
        assert!(script.contains("rm -rf /opt/lightmonitor"));
    }

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (k, v) in pairs {
            map.insert(
                axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                HeaderValue::from_str(v).unwrap(),
            );
        }
        map
    }

    #[test]
    fn prefers_explicit_public_url() {
        let h = headers(&[("host", "192.168.1.10:8080")]);
        let url = resolve_server_url("https://monitor.example.com", &h).unwrap();
        assert_eq!(url, "https://monitor.example.com");
    }

    #[test]
    fn auto_from_host_when_public_url_empty() {
        let h = headers(&[("host", "203.0.113.10:8080")]);
        let url = resolve_server_url("", &h).unwrap();
        assert_eq!(url, "http://203.0.113.10:8080");
    }

    #[test]
    fn ignores_placeholder_and_uses_request() {
        let h = headers(&[
            ("x-forwarded-proto", "https"),
            ("x-forwarded-host", "monitor.example.com"),
            ("host", "127.0.0.1:8080"),
        ]);
        let url = resolve_server_url("http://your-server-ip:8080", &h).unwrap();
        assert_eq!(url, "https://monitor.example.com");
    }

    #[test]
    fn ignores_loopback_config_and_uses_origin() {
        let h = headers(&[
            ("origin", "http://10.0.0.5:8080"),
            ("host", "127.0.0.1:8080"),
        ]);
        let url = resolve_server_url("http://127.0.0.1:8080", &h).unwrap();
        assert_eq!(url, "http://10.0.0.5:8080");
    }

    #[test]
    fn detects_placeholder() {
        assert!(is_placeholder_public_url("http://your-server-ip:8080"));
        assert!(!is_placeholder_public_url("http://203.0.113.10:8080"));
    }

    #[test]
    fn fixes_slash_before_port_typo() {
        assert_eq!(
            normalize_public_url("http://jiangcheng.site/:8080").unwrap(),
            "http://jiangcheng.site:8080"
        );
        assert_eq!(
            normalize_public_url("https://monitor.example.com/:443/admin").unwrap(),
            "https://monitor.example.com:443"
        );
        let h = headers(&[("host", "jiangcheng.site/:8080")]);
        let url = resolve_server_url("", &h).unwrap();
        assert_eq!(url, "http://jiangcheng.site:8080");
    }
}
