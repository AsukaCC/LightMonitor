use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HostStatus {
    Pending,
    Installing,
    Online,
    Warning,
    Offline,
    Error,
}

impl HostStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Installing => "installing",
            Self::Online => "online",
            Self::Warning => "warning",
            Self::Offline => "offline",
            Self::Error => "error",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "installing" => Self::Installing,
            "online" => Self::Online,
            "warning" => Self::Warning,
            "offline" => Self::Offline,
            "error" => Self::Error,
            _ => Self::Pending,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskSample {
    pub name: String,
    pub mount_point: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemSample {
    pub hostname: String,
    pub os: String,
    pub kernel: String,
    pub uptime_seconds: u64,
    #[serde(default)]
    pub cpu_cores: u32,
    pub cpu_percent: f32,
    pub memory_total_bytes: u64,
    pub memory_used_bytes: u64,
    pub swap_total_bytes: u64,
    pub swap_used_bytes: u64,
    pub load_average: [f64; 3],
    pub network_rx_bytes: u64,
    pub network_tx_bytes: u64,
    #[serde(default)]
    pub network_rx_rate: Option<f64>,
    #[serde(default)]
    pub network_tx_rate: Option<f64>,
    pub disks: Vec<DiskSample>,
    pub collected_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MetricHistoryPoint {
    pub collected_at: DateTime<Utc>,
    pub cpu_percent: f32,
    pub memory_percent: f32,
    pub disk_percent: f32,
    pub load_one: f64,
    pub network_rx_bytes: u64,
    pub network_tx_bytes: u64,
    pub network_rx_rate: Option<f64>,
    pub network_tx_rate: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MetricHistoryResponse {
    pub range: String,
    pub points: Vec<MetricHistoryPoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallLog {
    pub at: DateTime<Utc>,
    pub ok: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostDomain {
    pub id: Uuid,
    pub domain: String,
    pub port: u16,
    pub resolved_ipv4: Vec<String>,
    pub resolved_ipv6: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssl_expires_at: Option<DateTime<Utc>>,
    pub ssl_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packet_loss_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_checked_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Host {
    pub id: Uuid,
    pub is_system: bool,
    pub name: String,
    pub address: String,
    pub region: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    pub resolved_ipv4: Vec<String>,
    pub resolved_ipv6: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packet_loss_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_probed_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub probe_error: Option<String>,
    pub domains: Vec<HostDomain>,
    pub ssh_user: String,
    pub ssh_port: u16,
    pub update_interval_seconds: u64,
    /// Whether an SSH password is stored (password itself is never returned).
    pub has_ssh_password: bool,
    /// Whether a reusable SSH identity file path is stored.
    pub has_ssh_identity: bool,
    pub tags: Vec<String>,
    pub status: HostStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest: Option<SystemSample>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<DateTime<Utc>>,
    pub install_logs: Vec<InstallLog>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicMetrics {
    pub cpu_cores: u32,
    pub cpu_percent: f32,
    pub memory_used_bytes: u64,
    pub memory_total_bytes: u64,
    pub memory_percent: f32,
    pub disk_used_bytes: u64,
    pub disk_total_bytes: u64,
    pub disk_percent: f32,
    pub load_average: [f64; 3],
    pub uptime_seconds: u64,
    pub network_rx_rate: f64,
    pub network_tx_rate: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicHost {
    pub id: Uuid,
    pub name: String,
    pub region: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    pub resolved_ipv4: Vec<String>,
    pub resolved_ipv6: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packet_loss_percent: Option<f64>,
    pub domains: Vec<HostDomain>,
    pub tags: Vec<String>,
    pub status: HostStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<PublicMetrics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<DateTime<Utc>>,
}

impl Host {
    pub fn to_public(&self) -> PublicHost {
        let metrics = self.latest.as_ref().map(|sample| {
            let memory_percent = if sample.memory_total_bytes > 0 {
                (sample.memory_used_bytes as f64 / sample.memory_total_bytes as f64 * 100.0) as f32
            } else {
                0.0
            };
            let disk_percent = sample
                .disks
                .first()
                .map(|disk| {
                    if disk.total_bytes > 0 {
                        ((disk.total_bytes - disk.available_bytes) as f64 / disk.total_bytes as f64
                            * 100.0) as f32
                    } else {
                        0.0
                    }
                })
                .unwrap_or(0.0);
            let (disk_used_bytes, disk_total_bytes) = sample
                .disks
                .first()
                .map(|disk| {
                    (
                        disk.total_bytes.saturating_sub(disk.available_bytes),
                        disk.total_bytes,
                    )
                })
                .unwrap_or((0, 0));
            PublicMetrics {
                cpu_cores: sample.cpu_cores,
                cpu_percent: sample.cpu_percent,
                memory_used_bytes: sample.memory_used_bytes,
                memory_total_bytes: sample.memory_total_bytes,
                memory_percent,
                disk_used_bytes,
                disk_total_bytes,
                disk_percent,
                load_average: sample.load_average,
                uptime_seconds: sample.uptime_seconds,
                network_rx_rate: sample.network_rx_rate.unwrap_or(0.0),
                network_tx_rate: sample.network_tx_rate.unwrap_or(0.0),
            }
        });

        PublicHost {
            id: self.id,
            name: self.name.clone(),
            region: self.region.clone(),
            expires_at: self.expires_at,
            resolved_ipv4: self.resolved_ipv4.clone(),
            resolved_ipv6: self.resolved_ipv6.clone(),
            latency_ms: self.latency_ms,
            packet_loss_percent: self.packet_loss_percent,
            domains: self.domains.clone(),
            tags: self.tags.clone(),
            status: self.status.clone(),
            metrics,
            last_seen: self.last_seen,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateHostRequest {
    pub name: String,
    pub address: String,
    #[serde(default)]
    pub region: String,
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
    pub ssh_user: String,
    pub ssh_port: u16,
    /// Optional SSH password for remote install (stored server-side only).
    #[serde(default)]
    pub ssh_password: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateHostRequest {
    pub name: String,
    pub address: String,
    #[serde(default)]
    pub region: String,
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
    pub ssh_user: String,
    pub ssh_port: u16,
    /// Empty = keep existing password; non-empty = replace; use clear_ssh_password to remove.
    #[serde(default)]
    pub ssh_password: String,
    #[serde(default)]
    pub clear_ssh_password: bool,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeleteHostsRequest {
    pub ids: Vec<Uuid>,
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateHostIntervalRequest {
    pub ids: Vec<Uuid>,
    pub interval_seconds: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateHostDomainRequest {
    pub domain: String,
    #[serde(default = "default_https_port")]
    pub port: u16,
}

fn default_https_port() -> u16 {
    443
}

#[derive(Debug, Clone, Deserialize)]
pub struct MetricHistoryQuery {
    pub range: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub username: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionResponse {
    pub username: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RegisterAgentRequest {
    pub token: String,
    pub hostname: String,
    pub address: Option<String>,
    #[allow(dead_code)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RegisterAgentResponse {
    pub agent_id: Uuid,
    pub interval_seconds: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentConfigResponse {
    pub interval_seconds: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MetricReport {
    pub agent_id: Uuid,
    pub token: String,
    pub sample: SystemSample,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InstallAgentRequest {
    /// Optional legacy private key path inside the server container/host.
    #[serde(default)]
    pub ssh_key_path: String,
    /// Managed SSH key uploaded to the server data volume.
    #[serde(default)]
    pub ssh_key_id: Option<Uuid>,
    /// Optional SSH password (password auth). Prefer key when both set.
    #[serde(default)]
    pub ssh_password: String,
    /// Reuse the identity file path saved after a previous successful install.
    #[serde(default)]
    pub use_saved_identity: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SshKey {
    pub id: Uuid,
    pub name: String,
    pub size_bytes: u64,
    pub updated_at: DateTime<Utc>,
    pub in_use: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AppRelease {
    pub version: String,
    pub name: String,
    pub published_at: Option<String>,
    pub html_url: String,
    pub prerelease: bool,
    pub installed: bool,
    pub active: bool,
    pub asset_name: Option<String>,
    pub asset_size: Option<u64>,
    pub can_delete: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReleaseCatalog {
    pub current_version: String,
    pub latest_version: Option<String>,
    pub github_repo: String,
    pub managed_updates: bool,
    pub platform_asset: Option<String>,
    pub releases: Vec<AppRelease>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApplyReleaseRequest {
    pub version: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApplyReleaseResponse {
    pub version: String,
    pub restarting: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    HostUpdated { host: Box<Host> },
    HostsDeleted { host_ids: Vec<Uuid> },
    InstallLog { host_id: Uuid, log: InstallLog },
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentTokenResponse {
    pub host_id: Uuid,
    pub agent_token: String,
    pub install_command: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}
