use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub data_dir: PathBuf,
    pub web_dir: PathBuf,
    pub releases_dir: PathBuf,
    pub versions_dir: PathBuf,
    pub public_url: String,
    pub github_repo: String,
    pub managed_updates: bool,
    pub admin_username: String,
    pub admin_password: String,
    pub offline_seconds: u64,
    pub session_ttl_hours: i64,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let _ = dotenvy::dotenv();

        let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port = env::var("PORT")
            .or_else(|_| env::var("LIGHTMONITOR_PORT"))
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(8080);
        let data_dir = env::var("LIGHTMONITOR_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("data"));
        let web_dir = env::var("LIGHTMONITOR_WEB_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("web/dist"));
        let releases_dir = env::var("LIGHTMONITOR_RELEASES_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("releases"));
        let versions_dir = env::var("LIGHTMONITOR_VERSIONS_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| data_dir.join("versions"));
        // Optional override for agent callback URL. Empty / placeholder / loopback
        // means handlers auto-detect from the current HTTP request.
        let public_url = env::var("LIGHTMONITOR_PUBLIC_URL")
            .unwrap_or_default()
            .trim()
            .trim_end_matches('/')
            .to_string();
        let github_repo = env::var("LIGHTMONITOR_GITHUB_REPO")
            .unwrap_or_else(|_| "AsukaCC/LightMonitor".to_string());
        let managed_updates = env::var("LIGHTMONITOR_MANAGED_UPDATES")
            .ok()
            .is_some_and(|value| {
                matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes")
            });
        let admin_username =
            env::var("LIGHTMONITOR_ADMIN_USERNAME").unwrap_or_else(|_| "admin".to_string());
        let admin_password =
            env::var("LIGHTMONITOR_ADMIN_PASSWORD").unwrap_or_else(|_| "admin".to_string());
        let offline_seconds = env::var("LIGHTMONITOR_OFFLINE_SECONDS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);
        let session_ttl_hours = env::var("LIGHTMONITOR_SESSION_TTL_HOURS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(168);

        Ok(Self {
            host,
            port,
            data_dir,
            web_dir,
            releases_dir,
            versions_dir,
            public_url,
            github_repo,
            managed_updates,
            admin_username,
            admin_password,
            offline_seconds,
            session_ttl_hours,
        })
    }

    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("lightmonitor.db")
    }

    pub fn listen_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}
