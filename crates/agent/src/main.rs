use anyhow::{Context, bail};
use chrono::{DateTime, Utc};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::env;
use std::path::PathBuf;
use std::time::Duration;
use sysinfo::{Disks, Networks, System};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
struct RegisterAgentRequest {
    token: String,
    hostname: String,
    address: Option<String>,
    version: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct RegisterAgentResponse {
    agent_id: Uuid,
    #[serde(default)]
    interval_seconds: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
struct AgentConfigResponse {
    interval_seconds: u64,
}

#[derive(Debug, Clone, Serialize)]
struct MetricReport {
    agent_id: Uuid,
    token: String,
    sample: SystemSample,
}

#[derive(Debug, Clone, Serialize)]
struct DiskSample {
    name: String,
    mount_point: String,
    total_bytes: u64,
    available_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
struct SystemSample {
    hostname: String,
    os: String,
    kernel: String,
    uptime_seconds: u64,
    cpu_cores: u32,
    cpu_percent: f32,
    memory_total_bytes: u64,
    memory_used_bytes: u64,
    swap_total_bytes: u64,
    swap_used_bytes: u64,
    load_average: [f64; 3],
    network_rx_bytes: u64,
    network_tx_bytes: u64,
    disks: Vec<DiskSample>,
    collected_at: DateTime<Utc>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::from_env()?;
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("failed to build http client")?;

    let registration = register(&client, &config).await?;
    let mut agent_id = registration.agent_id;
    let mut interval = registration
        .interval_seconds
        .and_then(server_interval)
        .unwrap_or(config.interval);
    save_agent_id(&config, agent_id).await.ok();

    let mut system = System::new_all();
    let mut disks = Disks::new_with_refreshed_list();
    let mut networks = Networks::new_with_refreshed_list();

    loop {
        system.refresh_all();
        disks.refresh(true);
        networks.refresh(true);

        let sample = collect_sample(&system, &disks, &networks);
        let report = MetricReport {
            agent_id,
            token: config.token.clone(),
            sample,
        };

        let response = client
            .post(config.endpoint("/api/agents/metrics"))
            .json(&report)
            .send()
            .await;

        match response {
            Ok(response) if response.status().is_success() => {
                if let Ok(settings) = response.json::<AgentConfigResponse>().await
                    && let Some(next) = server_interval(settings.interval_seconds)
                {
                    interval = next;
                }
            }
            Ok(response) if response.status() == StatusCode::UNAUTHORIZED => {
                eprintln!("metrics credentials rejected; attempting to register again");
                match register(&client, &config).await {
                    Ok(registration) => {
                        agent_id = registration.agent_id;
                        if let Some(next) = registration.interval_seconds.and_then(server_interval)
                        {
                            interval = next;
                        }
                        save_agent_id(&config, agent_id).await.ok();
                        eprintln!("agent registration refreshed");
                    }
                    Err(err) => eprintln!("agent re-registration failed: {err}"),
                }
            }
            Ok(response) => eprintln!("metrics rejected: {}", response.status()),
            Err(err) => eprintln!("metrics upload failed: {err}"),
        }

        tokio::time::sleep(interval).await;
    }
}

struct Config {
    server_url: String,
    token: String,
    interval: Duration,
    state_dir: PathBuf,
}

impl Config {
    fn from_env() -> anyhow::Result<Self> {
        let server_url = env::var("LIGHTMONITOR_SERVER_URL")
            .context("LIGHTMONITOR_SERVER_URL is required")?
            .trim_end_matches('/')
            .to_string();
        let token =
            env::var("LIGHTMONITOR_AGENT_TOKEN").context("LIGHTMONITOR_AGENT_TOKEN is required")?;
        if token.trim().is_empty() {
            bail!("LIGHTMONITOR_AGENT_TOKEN cannot be empty");
        }

        let interval = env::var("LIGHTMONITOR_INTERVAL_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(5));
        let state_dir = env::var("LIGHTMONITOR_STATE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."));

        Ok(Self {
            server_url,
            token,
            interval,
            state_dir,
        })
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}{}", self.server_url, path)
    }
}

async fn register(client: &Client, config: &Config) -> anyhow::Result<RegisterAgentResponse> {
    let hostname = System::host_name().unwrap_or_else(|| "unknown".to_string());
    let request = RegisterAgentRequest {
        token: config.token.clone(),
        hostname,
        address: None,
        version: Some(env!("CARGO_PKG_VERSION").to_string()),
    };

    let response = client
        .post(config.endpoint("/api/agents/register"))
        .json(&request)
        .send()
        .await
        .context("agent registration failed")?;

    if !response.status().is_success() {
        bail!("agent registration rejected: {}", response.status());
    }

    response
        .json::<RegisterAgentResponse>()
        .await
        .map_err(Into::into)
}

fn server_interval(seconds: u64) -> Option<Duration> {
    (1..=3600)
        .contains(&seconds)
        .then(|| Duration::from_secs(seconds))
}

fn collect_sample(system: &System, disks: &Disks, networks: &Networks) -> SystemSample {
    let load = System::load_average();
    let (network_rx_bytes, network_tx_bytes) =
        networks.iter().fold((0, 0), |(rx, tx), (_, network)| {
            (
                rx + network.total_received(),
                tx + network.total_transmitted(),
            )
        });

    SystemSample {
        hostname: System::host_name().unwrap_or_else(|| "unknown".to_string()),
        os: System::long_os_version()
            .or_else(System::name)
            .unwrap_or_else(|| "unknown".to_string()),
        kernel: System::kernel_version().unwrap_or_else(|| "unknown".to_string()),
        uptime_seconds: System::uptime(),
        cpu_cores: system.cpus().len() as u32,
        cpu_percent: system.global_cpu_usage(),
        memory_total_bytes: system.total_memory(),
        memory_used_bytes: system.used_memory(),
        swap_total_bytes: system.total_swap(),
        swap_used_bytes: system.used_swap(),
        load_average: [load.one, load.five, load.fifteen],
        network_rx_bytes,
        network_tx_bytes,
        disks: disks
            .iter()
            .map(|disk| DiskSample {
                name: disk.name().to_string_lossy().to_string(),
                mount_point: disk.mount_point().to_string_lossy().to_string(),
                total_bytes: disk.total_space(),
                available_bytes: disk.available_space(),
            })
            .collect(),
        collected_at: Utc::now(),
    }
}

async fn save_agent_id(config: &Config, agent_id: Uuid) -> anyhow::Result<()> {
    tokio::fs::create_dir_all(&config.state_dir).await?;
    tokio::fs::write(config.state_dir.join("agent-id"), agent_id.to_string()).await?;
    Ok(())
}
