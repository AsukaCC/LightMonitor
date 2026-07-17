use crate::credential::CredentialCipher;
use crate::models::{
    CreateHostRequest, Host, HostStatus, InstallLog, MetricHistoryPoint, SystemSample,
    UpdateHostRequest,
};
use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
    credentials: Arc<CredentialCipher>,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create data dir {}", parent.display()))?;
        }
        let credentials = CredentialCipher::load_or_create(&path.with_extension("key"))?;
        let conn =
            Connection::open(path).with_context(|| format!("open sqlite {}", path.display()))?;
        conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS hosts (
                id TEXT PRIMARY KEY NOT NULL,
                is_system INTEGER NOT NULL DEFAULT 0,
                name TEXT NOT NULL,
                address TEXT NOT NULL,
                region TEXT NOT NULL DEFAULT '',
                ssh_user TEXT NOT NULL,
                ssh_port INTEGER NOT NULL,
                update_interval_seconds INTEGER NOT NULL DEFAULT 5,
                ssh_password TEXT NOT NULL DEFAULT '',
                ssh_key_path TEXT NOT NULL DEFAULT '',
                tags_json TEXT NOT NULL DEFAULT '[]',
                status TEXT NOT NULL,
                agent_id TEXT,
                agent_token TEXT NOT NULL UNIQUE,
                latest_json TEXT,
                last_seen TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS install_logs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                host_id TEXT NOT NULL,
                at TEXT NOT NULL,
                ok INTEGER NOT NULL,
                message TEXT NOT NULL,
                FOREIGN KEY(host_id) REFERENCES hosts(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS admin_sessions (
                token TEXT PRIMARY KEY NOT NULL,
                username TEXT NOT NULL,
                created_at TEXT NOT NULL,
                expires_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS metric_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                host_id TEXT NOT NULL,
                collected_at TEXT NOT NULL,
                cpu_percent REAL NOT NULL,
                memory_percent REAL NOT NULL,
                disk_percent REAL NOT NULL,
                load_one REAL NOT NULL,
                network_rx_bytes INTEGER NOT NULL,
                network_tx_bytes INTEGER NOT NULL,
                FOREIGN KEY(host_id) REFERENCES hosts(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_hosts_agent_token ON hosts(agent_token);
            CREATE INDEX IF NOT EXISTS idx_hosts_status ON hosts(status);
            CREATE INDEX IF NOT EXISTS idx_install_logs_host ON install_logs(host_id);
            CREATE INDEX IF NOT EXISTS idx_metric_history_host_time ON metric_history(host_id, collected_at);
            CREATE INDEX IF NOT EXISTS idx_metric_history_time ON metric_history(collected_at);
            ",
        )?;
        // Best-effort migration for existing databases.
        let _ = conn.execute(
            "ALTER TABLE hosts ADD COLUMN region TEXT NOT NULL DEFAULT ''",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE hosts ADD COLUMN ssh_password TEXT NOT NULL DEFAULT ''",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE hosts ADD COLUMN update_interval_seconds INTEGER NOT NULL DEFAULT 5",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE hosts ADD COLUMN is_system INTEGER NOT NULL DEFAULT 0",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE hosts ADD COLUMN ssh_key_path TEXT NOT NULL DEFAULT ''",
            [],
        );
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
            credentials: Arc::new(credentials),
        };
        db.encrypt_legacy_ssh_passwords()?;
        Ok(db)
    }

    fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let conn = self.conn.lock().expect("db lock poisoned");
        f(&conn)
    }

    fn encrypt_legacy_ssh_passwords(&self) -> Result<()> {
        let plaintext = self.with_conn(|conn| {
            let mut statement =
                conn.prepare("SELECT id, ssh_password FROM hosts WHERE ssh_password <> ''")?;
            let rows = statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(Into::into)
        })?;

        for (id, password) in plaintext {
            if CredentialCipher::is_encrypted(&password) {
                continue;
            }
            let encrypted = self.credentials.encrypt(&password)?;
            self.with_conn(|conn| {
                conn.execute(
                    "UPDATE hosts SET ssh_password = ?1 WHERE id = ?2",
                    params![encrypted, id],
                )?;
                Ok(())
            })?;
        }
        Ok(())
    }

    pub fn create_session(&self, token: &str, username: &str, ttl_hours: i64) -> Result<()> {
        let now = Utc::now();
        let expires = now + Duration::hours(ttl_hours);
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO admin_sessions (token, username, created_at, expires_at) VALUES (?1, ?2, ?3, ?4)",
                params![token, username, now.to_rfc3339(), expires.to_rfc3339()],
            )?;
            Ok(())
        })
    }

    pub fn delete_session(&self, token: &str) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "DELETE FROM admin_sessions WHERE token = ?1",
                params![token],
            )?;
            Ok(())
        })
    }

    pub fn session_username(&self, token: &str) -> Result<Option<String>> {
        self.with_conn(|conn| {
            let now = Utc::now().to_rfc3339();
            conn.query_row(
                "SELECT username FROM admin_sessions WHERE token = ?1 AND expires_at > ?2",
                params![token, now],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
        })
    }

    pub fn list_hosts(&self) -> Result<Vec<Host>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, address, region, ssh_user, ssh_port, ssh_password, ssh_key_path, tags_json, status, agent_id,
                        latest_json, last_seen, update_interval_seconds, created_at, is_system
                 FROM hosts ORDER BY is_system DESC, created_at DESC",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, Option<String>>(11)?,
                    row.get::<_, Option<String>>(12)?,
                    row.get::<_, i64>(13)?,
                    row.get::<_, String>(14)?,
                    row.get::<_, i64>(15)?,
                ))
            })?;

            let mut hosts = Vec::new();
            for row in rows {
                let (
                    id,
                    name,
                    address,
                    region,
                    ssh_user,
                    ssh_port,
                    ssh_password,
                    ssh_key_path,
                    tags_json,
                    status,
                    agent_id,
                    latest_json,
                    last_seen,
                    update_interval_seconds,
                    created_at,
                    is_system,
                ) = row?;
                let host_id = Uuid::parse_str(&id)?;
                let logs = load_install_logs(conn, &id)?;
                hosts.push(Host {
                    id: host_id,
                    is_system: is_system != 0,
                    name,
                    address,
                    region,
                    ssh_user,
                    ssh_port: ssh_port as u16,
                    update_interval_seconds: update_interval_seconds.max(1) as u64,
                    has_ssh_password: !ssh_password.is_empty(),
                    has_ssh_identity: !ssh_key_path.is_empty(),
                    tags: serde_json::from_str(&tags_json).unwrap_or_default(),
                    status: HostStatus::parse(&status),
                    agent_id: agent_id.and_then(|v| Uuid::parse_str(&v).ok()),
                    latest: latest_json.and_then(|v| serde_json::from_str(&v).ok()),
                    last_seen: last_seen.and_then(|v| DateTime::parse_from_rfc3339(&v).ok().map(|d| d.with_timezone(&Utc))),
                    install_logs: logs,
                    created_at: parse_dt(&created_at),
                });
            }
            Ok(hosts)
        })
    }

    pub fn get_host(&self, id: Uuid) -> Result<Option<Host>> {
        self.with_conn(|conn| get_host_conn(conn, id))
    }

    pub fn ensure_system_host_with_details(
        &self,
        hostname: &str,
        address: &str,
        region: &str,
    ) -> Result<Host> {
        let id = system_host_id();
        let now = Utc::now().to_rfc3339();
        let token = format!("system-{}", Uuid::new_v4());
        let tags_json = serde_json::to_string(&vec!["宿主机"])?;
        let address = if address.trim().is_empty() {
            "127.0.0.1"
        } else {
            address.trim()
        };
        self.with_conn(|conn| {
            conn.execute(
                "INSERT OR IGNORE INTO hosts (
                    id, is_system, name, address, region, ssh_user, ssh_port, ssh_password,
                    tags_json, status, agent_id, agent_token, latest_json, last_seen,
                    created_at, updated_at
                 ) VALUES (?1, 1, ?2, ?3, ?4, '', 22, '', ?5, ?6, ?1, ?7,
                           NULL, NULL, ?8, ?8)",
                params![
                    id.to_string(),
                    hostname,
                    address,
                    region,
                    tags_json,
                    HostStatus::Pending.as_str(),
                    token,
                    now,
                ],
            )?;
            conn.execute(
                "UPDATE hosts SET is_system = 1,
                 name = CASE WHEN TRIM(name) = '' THEN ?1 ELSE name END, address = ?2,
                 region = CASE WHEN TRIM(?3) <> '' THEN ?3 ELSE region END,
                 agent_id = ?4, updated_at = ?5 WHERE id = ?6",
                params![
                    hostname,
                    address,
                    region,
                    id.to_string(),
                    now,
                    id.to_string()
                ],
            )?;
            Ok(())
        })?;
        self.get_host(id)?
            .context("system host missing after initialization")
    }

    pub fn create_host(&self, req: CreateHostRequest, agent_token: String) -> Result<Host> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let tags_json = serde_json::to_string(&req.tags)?;
        let encrypted_password = self.credentials.encrypt(&req.ssh_password)?;
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO hosts (
                    id, name, address, region, ssh_user, ssh_port, ssh_password, tags_json, status,
                    agent_id, agent_token, latest_json, last_seen, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10, NULL, NULL, ?11, ?12)",
                params![
                    id.to_string(),
                    req.name,
                    req.address,
                    req.region,
                    req.ssh_user,
                    req.ssh_port as i64,
                    encrypted_password,
                    tags_json,
                    HostStatus::Pending.as_str(),
                    agent_token,
                    now.to_rfc3339(),
                    now.to_rfc3339(),
                ],
            )?;
            Ok(())
        })?;
        self.get_host(id)?
            .context("created host missing after insert")
    }

    pub fn update_host(&self, id: Uuid, req: UpdateHostRequest) -> Result<Option<Host>> {
        let now = Utc::now().to_rfc3339();
        let tags_json = serde_json::to_string(&req.tags)?;
        let encrypted_password = if req.ssh_password.is_empty() {
            String::new()
        } else {
            self.credentials.encrypt(&req.ssh_password)?
        };
        let changed = self.with_conn(|conn| {
            let n = if req.clear_ssh_password {
                conn.execute(
                    "UPDATE hosts SET name = ?1, address = ?2, region = ?3, ssh_user = ?4, ssh_port = ?5,
                     ssh_password = '', tags_json = ?6, updated_at = ?7 WHERE id = ?8",
                    params![
                        req.name,
                        req.address,
                        req.region,
                        req.ssh_user,
                        req.ssh_port as i64,
                        tags_json,
                        now,
                        id.to_string()
                    ],
                )?
            } else if !req.ssh_password.is_empty() {
                conn.execute(
                    "UPDATE hosts SET name = ?1, address = ?2, region = ?3, ssh_user = ?4, ssh_port = ?5,
                     ssh_password = ?6, tags_json = ?7, updated_at = ?8 WHERE id = ?9",
                    params![
                        req.name,
                        req.address,
                        req.region,
                        req.ssh_user,
                        req.ssh_port as i64,
                        encrypted_password,
                        tags_json,
                        now,
                        id.to_string()
                    ],
                )?
            } else {
                conn.execute(
                    "UPDATE hosts SET name = ?1, address = ?2, region = ?3, ssh_user = ?4, ssh_port = ?5,
                     tags_json = ?6, updated_at = ?7 WHERE id = ?8",
                    params![
                        req.name,
                        req.address,
                        req.region,
                        req.ssh_user,
                        req.ssh_port as i64,
                        tags_json,
                        now,
                        id.to_string()
                    ],
                )?
            };
            Ok(n)
        })?;
        if changed == 0 {
            return Ok(None);
        }
        self.get_host(id)
    }

    pub fn ssh_credentials(&self, id: Uuid) -> Result<Option<(String, String)>> {
        let stored = self.with_conn(|conn| {
            conn.query_row(
                "SELECT ssh_password, ssh_key_path FROM hosts WHERE id = ?1",
                params![id.to_string()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(Into::into)
        })?;
        stored
            .map(|(password, key_path)| {
                self.credentials
                    .decrypt(&password)
                    .map(|password| (password, key_path))
            })
            .transpose()
    }

    pub fn set_ssh_key_path(&self, id: Uuid, key_path: &str) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE hosts SET ssh_key_path = ?1, updated_at = ?2 WHERE id = ?3",
                params![key_path, Utc::now().to_rfc3339(), id.to_string()],
            )?;
            Ok(())
        })
    }

    pub fn delete_hosts(&self, ids: &[Uuid]) -> Result<Vec<Uuid>> {
        self.with_conn(|conn| {
            let mut deleted = Vec::new();
            for id in ids {
                let n = conn.execute(
                    "DELETE FROM hosts WHERE id = ?1 AND is_system = 0",
                    params![id.to_string()],
                )?;
                if n > 0 {
                    deleted.push(*id);
                }
            }
            Ok(deleted)
        })
    }

    pub fn contains_system_host(&self, ids: &[Uuid]) -> Result<bool> {
        self.with_conn(|conn| {
            for id in ids {
                let is_system = conn
                    .query_row(
                        "SELECT is_system FROM hosts WHERE id = ?1",
                        params![id.to_string()],
                        |row| row.get::<_, i64>(0),
                    )
                    .optional()?
                    .unwrap_or(0);
                if is_system != 0 {
                    return Ok(true);
                }
            }
            Ok(false)
        })
    }

    pub fn update_host_intervals(&self, ids: &[Uuid], interval_seconds: u64) -> Result<Vec<Host>> {
        self.with_conn(|conn| {
            let now = Utc::now().to_rfc3339();
            let mut updated = Vec::new();
            for id in ids {
                let changed = conn.execute(
                    "UPDATE hosts SET update_interval_seconds = ?1, updated_at = ?2 WHERE id = ?3",
                    params![interval_seconds as i64, now, id.to_string()],
                )?;
                if changed > 0
                    && let Some(host) = get_host_conn(conn, *id)?
                {
                    updated.push(host);
                }
            }
            Ok(updated)
        })
    }

    pub fn agent_token(&self, id: Uuid) -> Result<Option<String>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT agent_token FROM hosts WHERE id = ?1",
                params![id.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
        })
    }

    pub fn set_status(&self, id: Uuid, status: HostStatus) -> Result<Option<Host>> {
        let now = Utc::now().to_rfc3339();
        let changed = self.with_conn(|conn| {
            let n = conn.execute(
                "UPDATE hosts SET status = ?1, updated_at = ?2 WHERE id = ?3",
                params![status.as_str(), now, id.to_string()],
            )?;
            Ok(n)
        })?;
        if changed == 0 {
            return Ok(None);
        }
        self.get_host(id)
    }

    pub fn append_install_log(&self, id: Uuid, log: &InstallLog) -> Result<Option<Host>> {
        self.with_conn(|conn| {
            let n = conn.execute(
                "INSERT INTO install_logs (host_id, at, ok, message) VALUES (?1, ?2, ?3, ?4)",
                params![
                    id.to_string(),
                    log.at.to_rfc3339(),
                    if log.ok { 1 } else { 0 },
                    log.message
                ],
            )?;
            if n == 0 {
                return Ok(());
            }
            Ok(())
        })?;
        self.get_host(id)
    }

    pub fn register_agent(
        &self,
        token: &str,
        hostname: &str,
        address: Option<&str>,
    ) -> Result<Option<(Uuid, Host)>> {
        self.with_conn(|conn| {
            let row = conn
                .query_row(
                    "SELECT id, agent_id, name FROM hosts WHERE agent_token = ?1",
                    params![token],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    },
                )
                .optional()?;

            let Some((host_id_str, existing_agent, name)) = row else {
                return Ok(None);
            };

            let host_id = Uuid::parse_str(&host_id_str)?;
            let agent_id = existing_agent
                .and_then(|v| Uuid::parse_str(&v).ok())
                .unwrap_or_else(Uuid::new_v4);
            let now = Utc::now();

            let new_name = if name.trim().is_empty() {
                let fallback = hostname.trim();
                if fallback.is_empty() {
                    "未命名主机".to_string()
                } else {
                    fallback.to_string()
                }
            } else {
                name
            };
            if let Some(addr) = address {
                conn.execute(
                    "UPDATE hosts SET agent_id = ?1, address = CASE WHEN address = '' OR address IS NULL THEN ?2 ELSE address END,
                     name = ?3, status = ?4, updated_at = ?5 WHERE id = ?6",
                    params![
                        agent_id.to_string(),
                        addr,
                        new_name,
                        HostStatus::Online.as_str(),
                        now.to_rfc3339(),
                        host_id_str
                    ],
                )?;
            } else {
                conn.execute(
                    "UPDATE hosts SET agent_id = ?1, name = ?2, status = ?3, updated_at = ?4 WHERE id = ?5",
                    params![
                        agent_id.to_string(),
                        new_name,
                        HostStatus::Online.as_str(),
                        now.to_rfc3339(),
                        host_id_str
                    ],
                )?;
            }

            let host = get_host_conn(conn, host_id)?.context("host missing after register")?;
            Ok(Some((agent_id, host)))
        })
    }

    pub fn apply_metrics(
        &self,
        agent_id: Uuid,
        token: &str,
        sample: &SystemSample,
    ) -> Result<Option<Host>> {
        self.with_conn(|conn| {
            let host_id = conn
                .query_row(
                    "SELECT id FROM hosts WHERE agent_id = ?1 AND agent_token = ?2 AND is_system = 0",
                    params![agent_id.to_string(), token],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            let Some(host_id) = host_id else {
                return Ok(None);
            };
            store_metric_sample(conn, Uuid::parse_str(&host_id)?, sample)
        })
    }

    pub fn apply_system_metrics(&self, sample: &SystemSample) -> Result<Host> {
        self.with_conn(|conn| {
            store_metric_sample(conn, system_host_id(), sample)?
                .context("system host missing while storing metrics")
        })
    }

    pub fn metric_history(
        &self,
        host_id: Uuid,
        since: DateTime<Utc>,
        max_points: usize,
    ) -> Result<Vec<MetricHistoryPoint>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT collected_at, cpu_percent, memory_percent, disk_percent, load_one,
                        network_rx_bytes, network_tx_bytes
                 FROM metric_history
                 WHERE host_id = ?1 AND collected_at >= ?2
                 ORDER BY collected_at ASC",
            )?;
            let points = stmt
                .query_map(params![host_id.to_string(), since.to_rfc3339()], |row| {
                    let collected_at: String = row.get(0)?;
                    Ok(MetricHistoryPoint {
                        collected_at: parse_dt(&collected_at),
                        cpu_percent: row.get::<_, f64>(1)? as f32,
                        memory_percent: row.get::<_, f64>(2)? as f32,
                        disk_percent: row.get::<_, f64>(3)? as f32,
                        load_one: row.get(4)?,
                        network_rx_bytes: row.get::<_, i64>(5)?.max(0) as u64,
                        network_tx_bytes: row.get::<_, i64>(6)?.max(0) as u64,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(downsample_history(points, max_points))
        })
    }

    pub fn mark_offline(&self, offline_seconds: u64) -> Result<Vec<Host>> {
        let current_time = Utc::now();
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, last_seen, update_interval_seconds FROM hosts
                 WHERE status IN ('online', 'warning', 'installing')
                   AND last_seen IS NOT NULL",
            )?;
            let ids: Vec<String> = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                })?
                .filter_map(|row| row.ok())
                .filter_map(|(id, last_seen, interval)| {
                    let last_seen = DateTime::parse_from_rfc3339(&last_seen)
                        .ok()?
                        .with_timezone(&Utc);
                    let interval_grace = (interval.max(1) as u64).saturating_mul(3);
                    let threshold = offline_seconds.max(interval_grace);
                    ((current_time - last_seen) > Duration::seconds(threshold as i64)).then_some(id)
                })
                .collect();

            let mut updated = Vec::new();
            let now = Utc::now().to_rfc3339();
            for id in ids {
                conn.execute(
                    "UPDATE hosts SET status = ?1, updated_at = ?2 WHERE id = ?3",
                    params![HostStatus::Offline.as_str(), now, id],
                )?;
                if let Some(host) = get_host_conn(conn, Uuid::parse_str(&id)?)? {
                    updated.push(host);
                }
            }
            Ok(updated)
        })
    }
}

fn system_host_id() -> Uuid {
    Uuid::from_u128(1)
}

fn store_metric_sample(
    conn: &Connection,
    host_id: Uuid,
    sample: &SystemSample,
) -> Result<Option<Host>> {
    let latest_json = serde_json::to_string(sample)?;
    let now = Utc::now();
    let changed = conn.execute(
        "UPDATE hosts SET latest_json = ?1, last_seen = ?2, status = ?3, updated_at = ?4
         WHERE id = ?5",
        params![
            latest_json,
            sample.collected_at.to_rfc3339(),
            HostStatus::Online.as_str(),
            now.to_rfc3339(),
            host_id.to_string(),
        ],
    )?;
    if changed == 0 {
        return Ok(None);
    }

    let memory_percent = if sample.memory_total_bytes > 0 {
        sample.memory_used_bytes as f64 / sample.memory_total_bytes as f64 * 100.0
    } else {
        0.0
    };
    let disk_percent = sample
        .disks
        .first()
        .filter(|disk| disk.total_bytes > 0)
        .map(|disk| {
            (disk.total_bytes - disk.available_bytes) as f64 / disk.total_bytes as f64 * 100.0
        })
        .unwrap_or(0.0);
    conn.execute(
        "INSERT INTO metric_history (
            host_id, collected_at, cpu_percent, memory_percent, disk_percent, load_one,
            network_rx_bytes, network_tx_bytes
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            host_id.to_string(),
            sample.collected_at.to_rfc3339(),
            sample.cpu_percent as f64,
            memory_percent,
            disk_percent,
            sample.load_average[0],
            sample.network_rx_bytes as i64,
            sample.network_tx_bytes as i64,
        ],
    )?;
    let retention = (now - Duration::days(7)).to_rfc3339();
    conn.execute(
        "DELETE FROM metric_history WHERE collected_at < ?1",
        params![retention],
    )?;
    get_host_conn(conn, host_id)
}

fn get_host_conn(conn: &Connection, id: Uuid) -> Result<Option<Host>> {
    let row = conn
        .query_row(
            "SELECT id, name, address, region, ssh_user, ssh_port, ssh_password, ssh_key_path, tags_json, status, agent_id,
                    latest_json, last_seen, update_interval_seconds, created_at, is_system
             FROM hosts WHERE id = ?1",
            params![id.to_string()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, Option<String>>(11)?,
                    row.get::<_, Option<String>>(12)?,
                    row.get::<_, i64>(13)?,
                    row.get::<_, String>(14)?,
                    row.get::<_, i64>(15)?,
                ))
            },
        )
        .optional()?;

    let Some((
        id_str,
        name,
        address,
        region,
        ssh_user,
        ssh_port,
        ssh_password,
        ssh_key_path,
        tags_json,
        status,
        agent_id,
        latest_json,
        last_seen,
        update_interval_seconds,
        created_at,
        is_system,
    )) = row
    else {
        return Ok(None);
    };

    let logs = load_install_logs(conn, &id_str)?;
    Ok(Some(Host {
        id: Uuid::parse_str(&id_str)?,
        is_system: is_system != 0,
        name,
        address,
        region,
        ssh_user,
        ssh_port: ssh_port as u16,
        update_interval_seconds: update_interval_seconds.max(1) as u64,
        has_ssh_password: !ssh_password.is_empty(),
        has_ssh_identity: !ssh_key_path.is_empty(),
        tags: serde_json::from_str(&tags_json).unwrap_or_default(),
        status: HostStatus::parse(&status),
        agent_id: agent_id.and_then(|v| Uuid::parse_str(&v).ok()),
        latest: latest_json.and_then(|v| serde_json::from_str(&v).ok()),
        last_seen: last_seen.and_then(|v| {
            DateTime::parse_from_rfc3339(&v)
                .ok()
                .map(|d| d.with_timezone(&Utc))
        }),
        install_logs: logs,
        created_at: parse_dt(&created_at),
    }))
}

fn load_install_logs(conn: &Connection, host_id: &str) -> Result<Vec<InstallLog>> {
    let mut stmt = conn.prepare(
        "SELECT at, ok, message FROM install_logs WHERE host_id = ?1 ORDER BY id DESC LIMIT 50",
    )?;
    let rows = stmt.query_map(params![host_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut logs = Vec::new();
    for row in rows {
        let (at, ok, message) = row?;
        logs.push(InstallLog {
            at: parse_dt(&at),
            ok: ok != 0,
            message,
        });
    }
    logs.reverse();
    Ok(logs)
}

fn parse_dt(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

fn downsample_history(
    points: Vec<MetricHistoryPoint>,
    max_points: usize,
) -> Vec<MetricHistoryPoint> {
    if max_points == 0 || points.len() <= max_points {
        return points;
    }

    let bucket_size = points.len().div_ceil(max_points);
    points
        .chunks(bucket_size)
        .map(|bucket| {
            let count = bucket.len() as f64;
            let last = bucket.last().expect("history bucket cannot be empty");
            MetricHistoryPoint {
                collected_at: last.collected_at,
                cpu_percent: (bucket
                    .iter()
                    .map(|point| point.cpu_percent as f64)
                    .sum::<f64>()
                    / count) as f32,
                memory_percent: (bucket
                    .iter()
                    .map(|point| point.memory_percent as f64)
                    .sum::<f64>()
                    / count) as f32,
                disk_percent: (bucket
                    .iter()
                    .map(|point| point.disk_percent as f64)
                    .sum::<f64>()
                    / count) as f32,
                load_one: bucket.iter().map(|point| point.load_one).sum::<f64>() / count,
                network_rx_bytes: last.network_rx_bytes,
                network_tx_bytes: last.network_tx_bytes,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::DiskSample;

    fn cleanup(path: &Path) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(path.with_extension("key"));
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[test]
    fn system_host_is_monitored_and_cannot_be_deleted() {
        let path = std::env::temp_dir().join(format!("lightmonitor-system-{}.db", Uuid::new_v4()));
        let db = Db::open(&path).unwrap();
        let system_host = db
            .ensure_system_host_with_details("monitor-node", "127.0.0.1", "本机")
            .unwrap();

        assert!(system_host.is_system);
        assert_eq!(system_host.name, "monitor-node");
        assert_eq!(db.list_hosts().unwrap()[0].id, system_host.id);
        assert!(db.contains_system_host(&[system_host.id]).unwrap());
        assert!(db.delete_hosts(&[system_host.id]).unwrap().is_empty());
        assert!(db.get_host(system_host.id).unwrap().is_some());

        let refreshed = db
            .ensure_system_host_with_details("monitor-node", "192.168.1.8", "中国 · 广东 · 深圳")
            .unwrap();
        assert_eq!(refreshed.name, "monitor-node");
        assert_eq!(refreshed.address, "192.168.1.8");
        assert_eq!(refreshed.region, "中国 · 广东 · 深圳");

        let sample = SystemSample {
            hostname: "monitor-node".to_string(),
            os: "test-os".to_string(),
            kernel: "test-kernel".to_string(),
            uptime_seconds: 60,
            cpu_cores: 8,
            cpu_percent: 12.5,
            memory_total_bytes: 100,
            memory_used_bytes: 40,
            swap_total_bytes: 20,
            swap_used_bytes: 5,
            load_average: [0.5, 0.25, 0.1],
            network_rx_bytes: 1024,
            network_tx_bytes: 512,
            disks: vec![DiskSample {
                name: "test-disk".to_string(),
                mount_point: "/".to_string(),
                total_bytes: 100,
                available_bytes: 70,
            }],
            collected_at: Utc::now(),
        };
        let monitored = db.apply_system_metrics(&sample).unwrap();
        assert_eq!(monitored.status, HostStatus::Online);
        assert_eq!(monitored.latest.unwrap().cpu_percent, 12.5);

        let same_host = db
            .ensure_system_host_with_details("renamed-node", "127.0.0.1", "本机")
            .unwrap();
        assert_eq!(same_host.id, system_host.id);
        assert_eq!(
            db.list_hosts()
                .unwrap()
                .iter()
                .filter(|host| host.is_system)
                .count(),
            1
        );

        drop(db);
        cleanup(&path);
    }

    #[test]
    fn stores_interval_and_uses_it_for_offline_grace() {
        let path =
            std::env::temp_dir().join(format!("lightmonitor-interval-{}.db", Uuid::new_v4()));
        let db = Db::open(&path).unwrap();
        let host = db
            .create_host(
                CreateHostRequest {
                    name: "自定义服务器".to_string(),
                    address: "192.0.2.10".to_string(),
                    region: String::new(),
                    ssh_user: String::new(),
                    ssh_port: 22,
                    ssh_password: String::new(),
                    tags: Vec::new(),
                },
                "test-token".to_string(),
            )
            .unwrap();
        assert_eq!(host.update_interval_seconds, 5);

        let updated = db.update_host_intervals(&[host.id], 60).unwrap();
        assert_eq!(updated[0].update_interval_seconds, 60);

        let recent = (Utc::now() - Duration::seconds(40)).to_rfc3339();
        db.with_conn(|conn| {
            conn.execute(
                "UPDATE hosts SET status = 'online', last_seen = ?1 WHERE id = ?2",
                params![recent, host.id.to_string()],
            )?;
            Ok(())
        })
        .unwrap();
        assert!(db.mark_offline(30).unwrap().is_empty());

        let stale = (Utc::now() - Duration::seconds(181)).to_rfc3339();
        db.with_conn(|conn| {
            conn.execute(
                "UPDATE hosts SET last_seen = ?1 WHERE id = ?2",
                params![stale, host.id.to_string()],
            )?;
            Ok(())
        })
        .unwrap();
        assert_eq!(db.mark_offline(30).unwrap().len(), 1);

        let (agent_id, _) = db
            .register_agent("test-token", "interval-test", None)
            .unwrap()
            .unwrap();
        assert_eq!(db.get_host(host.id).unwrap().unwrap().name, "自定义服务器");
        db.with_conn(|conn| {
            conn.execute(
                "UPDATE hosts SET name = '' WHERE id = ?1",
                params![host.id.to_string()],
            )?;
            Ok(())
        })
        .unwrap();
        let (_, fallback_host) = db
            .register_agent("test-token", "interval-test", None)
            .unwrap()
            .unwrap();
        assert_eq!(fallback_host.name, "interval-test");
        for index in 0..10 {
            let sample = SystemSample {
                hostname: "interval-test".to_string(),
                os: "test-os".to_string(),
                kernel: "test-kernel".to_string(),
                uptime_seconds: index,
                cpu_cores: 4,
                cpu_percent: index as f32,
                memory_total_bytes: 100,
                memory_used_bytes: index,
                swap_total_bytes: 0,
                swap_used_bytes: 0,
                load_average: [index as f64, 0.0, 0.0],
                network_rx_bytes: index * 1024,
                network_tx_bytes: index * 512,
                disks: vec![DiskSample {
                    name: "test-disk".to_string(),
                    mount_point: "/".to_string(),
                    total_bytes: 100,
                    available_bytes: 100 - index,
                }],
                collected_at: Utc::now() - Duration::seconds(10 - index as i64),
            };
            db.apply_metrics(agent_id, "test-token", &sample)
                .unwrap()
                .unwrap();
        }
        let history = db
            .metric_history(host.id, Utc::now() - Duration::hours(1), 4)
            .unwrap();
        assert!(history.len() <= 4);
        assert_eq!(history.last().unwrap().network_rx_bytes, 9 * 1024);
        assert!(history.last().unwrap().memory_percent > 0.0);

        drop(db);
        cleanup(&path);
    }

    #[test]
    fn stores_ssh_password_as_encrypted_ciphertext() {
        let path =
            std::env::temp_dir().join(format!("lightmonitor-credential-{}.db", Uuid::new_v4()));
        let db = Db::open(&path).unwrap();
        let host = db
            .create_host(
                CreateHostRequest {
                    name: "encrypted-host".to_string(),
                    address: "192.0.2.20".to_string(),
                    region: String::new(),
                    ssh_user: "root".to_string(),
                    ssh_port: 22,
                    ssh_password: "test-secret".to_string(),
                    tags: Vec::new(),
                },
                "credential-test-token".to_string(),
            )
            .unwrap();

        let stored = db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT ssh_password FROM hosts WHERE id = ?1",
                    params![host.id.to_string()],
                    |row| row.get::<_, String>(0),
                )
                .map_err(Into::into)
            })
            .unwrap();
        assert!(stored.starts_with("enc:v1:"));
        assert!(!stored.contains("test-secret"));
        assert_eq!(
            db.ssh_credentials(host.id).unwrap().unwrap().0,
            "test-secret"
        );

        drop(db);
        cleanup(&path);
    }
}
