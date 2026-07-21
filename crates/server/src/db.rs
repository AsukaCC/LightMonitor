use crate::credential::CredentialCipher;
use crate::models::{
    CreateHostDomainRequest, CreateHostRequest, Host, HostDomain, HostStatus, InstallLog,
    MetricHistoryPoint, SshAuthType, SshKey, SystemSample, UpdateHostRequest,
};
use crate::probe::{DomainProbeResult, HostProbeResult};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{Connection, OptionalExtension, Row, params};
use std::collections::HashSet;
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
                expires_at TEXT,
                resolved_ipv4_json TEXT NOT NULL DEFAULT '[]',
                resolved_ipv6_json TEXT NOT NULL DEFAULT '[]',
                latency_ms REAL,
                packet_loss_percent REAL,
                last_probed_at TEXT,
                probe_error TEXT NOT NULL DEFAULT '',
                ssh_user TEXT NOT NULL,
                ssh_port INTEGER NOT NULL,
                update_interval_seconds INTEGER NOT NULL DEFAULT 5,
                ssh_password TEXT NOT NULL DEFAULT '',
                ssh_key_path TEXT NOT NULL DEFAULT '',
                ssh_auth_type TEXT NOT NULL DEFAULT 'password',
                ssh_key_id TEXT,
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

            CREATE TABLE IF NOT EXISTS ssh_keys (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                storage_path TEXT NOT NULL UNIQUE,
                size_bytes INTEGER NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS host_domains (
                id TEXT PRIMARY KEY NOT NULL,
                host_id TEXT NOT NULL,
                domain TEXT NOT NULL,
                port INTEGER NOT NULL DEFAULT 443,
                resolved_ipv4_json TEXT NOT NULL DEFAULT '[]',
                resolved_ipv6_json TEXT NOT NULL DEFAULT '[]',
                ssl_expires_at TEXT,
                ssl_status TEXT NOT NULL DEFAULT 'pending',
                latency_ms REAL,
                packet_loss_percent REAL,
                last_checked_at TEXT,
                last_error TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL,
                FOREIGN KEY(host_id) REFERENCES hosts(id) ON DELETE CASCADE,
                UNIQUE(host_id, domain, port)
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
                network_rx_rate REAL,
                network_tx_rate REAL,
                FOREIGN KEY(host_id) REFERENCES hosts(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_hosts_agent_token ON hosts(agent_token);
            CREATE INDEX IF NOT EXISTS idx_hosts_status ON hosts(status);
            CREATE INDEX IF NOT EXISTS idx_install_logs_host ON install_logs(host_id);
            CREATE INDEX IF NOT EXISTS idx_host_domains_host ON host_domains(host_id);
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
        let _ = conn.execute(
            "ALTER TABLE hosts ADD COLUMN ssh_auth_type TEXT NOT NULL DEFAULT 'password'",
            [],
        );
        let _ = conn.execute("ALTER TABLE hosts ADD COLUMN ssh_key_id TEXT", []);
        let _ = conn.execute(
            "UPDATE hosts SET ssh_key_id = (
                SELECT id FROM ssh_keys WHERE storage_path = hosts.ssh_key_path
             ) WHERE ssh_key_path <> '' AND ssh_key_id IS NULL",
            [],
        );
        let _ = conn.execute(
            "UPDATE hosts SET ssh_auth_type = 'key' WHERE ssh_key_path <> ''",
            [],
        );
        let _ = conn.execute("ALTER TABLE hosts ADD COLUMN expires_at TEXT", []);
        let _ = conn.execute(
            "ALTER TABLE hosts ADD COLUMN resolved_ipv4_json TEXT NOT NULL DEFAULT '[]'",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE hosts ADD COLUMN resolved_ipv6_json TEXT NOT NULL DEFAULT '[]'",
            [],
        );
        let _ = conn.execute("ALTER TABLE hosts ADD COLUMN latency_ms REAL", []);
        let _ = conn.execute("ALTER TABLE hosts ADD COLUMN packet_loss_percent REAL", []);
        let _ = conn.execute("ALTER TABLE hosts ADD COLUMN last_probed_at TEXT", []);
        let _ = conn.execute(
            "ALTER TABLE hosts ADD COLUMN probe_error TEXT NOT NULL DEFAULT ''",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE metric_history ADD COLUMN network_rx_rate REAL",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE metric_history ADD COLUMN network_tx_rate REAL",
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

    pub fn list_ssh_keys(&self) -> Result<Vec<SshKey>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, storage_path, size_bytes, updated_at
                 FROM ssh_keys ORDER BY updated_at DESC, name ASC",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })?;
            let mut keys = Vec::new();
            for row in rows {
                let (id, name, storage_path, size_bytes, updated_at) = row?;
                let id = Uuid::parse_str(&id)?;
                let mut host_statement = conn.prepare(
                    "SELECT id, name FROM hosts
                     WHERE ssh_key_id = ?1 OR (ssh_key_id IS NULL AND ssh_key_path = ?2)
                     ORDER BY name ASC",
                )?;
                let bindings = host_statement
                    .query_map(params![id.to_string(), storage_path], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                let host_ids = bindings
                    .iter()
                    .filter_map(|(host_id, _)| Uuid::parse_str(host_id).ok())
                    .collect::<Vec<_>>();
                let host_names = bindings
                    .into_iter()
                    .map(|(_, host_name)| host_name)
                    .collect::<Vec<_>>();
                keys.push(SshKey {
                    id,
                    name,
                    size_bytes: size_bytes.max(0) as u64,
                    updated_at: parse_dt(&updated_at),
                    in_use: !host_ids.is_empty(),
                    host_ids,
                    host_names,
                });
            }
            Ok(keys)
        })
    }

    pub fn get_ssh_key(&self, id: Uuid) -> Result<Option<(String, String)>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT name, storage_path FROM ssh_keys WHERE id = ?1",
                params![id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(Into::into)
        })
    }

    pub fn create_ssh_key(
        &self,
        id: Uuid,
        name: &str,
        storage_path: &str,
        size_bytes: u64,
    ) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO ssh_keys (id, name, storage_path, size_bytes, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    id.to_string(),
                    name,
                    storage_path,
                    size_bytes as i64,
                    Utc::now().to_rfc3339()
                ],
            )?;
            Ok(())
        })
    }

    pub fn update_ssh_key(&self, id: Uuid, name: &str, size_bytes: u64) -> Result<bool> {
        self.with_conn(|conn| {
            let changed = conn.execute(
                "UPDATE ssh_keys SET name = ?1, size_bytes = ?2, updated_at = ?3 WHERE id = ?4",
                params![
                    name,
                    size_bytes as i64,
                    Utc::now().to_rfc3339(),
                    id.to_string()
                ],
            )?;
            Ok(changed > 0)
        })
    }

    pub fn delete_ssh_key(&self, id: Uuid) -> Result<Option<String>> {
        self.with_conn(|conn| {
            let Some(storage_path) = conn
                .query_row(
                    "SELECT storage_path FROM ssh_keys WHERE id = ?1",
                    params![id.to_string()],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
            else {
                return Ok(None);
            };
            let in_use = conn.query_row(
                "SELECT COUNT(*) FROM hosts WHERE ssh_key_id = ?1 OR ssh_key_path = ?2",
                params![id.to_string(), storage_path],
                |row| row.get::<_, i64>(0),
            )?;
            if in_use > 0 {
                bail!("SSH key is used by {in_use} host(s)");
            }
            conn.execute(
                "DELETE FROM ssh_keys WHERE id = ?1",
                params![id.to_string()],
            )?;
            Ok(Some(storage_path))
        })
    }

    pub fn assign_ssh_key_hosts(&self, id: Uuid, host_ids: &[Uuid]) -> Result<Vec<Host>> {
        self.with_conn(|conn| {
            let storage_path = conn
                .query_row(
                    "SELECT storage_path FROM ssh_keys WHERE id = ?1",
                    params![id.to_string()],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
                .context("SSH key not found")?;
            let requested = host_ids.iter().copied().collect::<HashSet<_>>();
            if requested.len() != host_ids.len() {
                bail!("duplicate host assignment");
            }

            for host_id in &requested {
                let assignable = conn
                    .query_row(
                        "SELECT is_system = 0 FROM hosts WHERE id = ?1",
                        params![host_id.to_string()],
                        |row| row.get::<_, bool>(0),
                    )
                    .optional()?
                    .unwrap_or(false);
                if !assignable {
                    bail!("host {host_id} does not exist or cannot use an SSH key");
                }
            }

            let mut statement =
                conn.prepare("SELECT id FROM hosts WHERE ssh_key_id = ?1 OR ssh_key_path = ?2")?;
            let existing = statement
                .query_map(params![id.to_string(), storage_path], |row| {
                    row.get::<_, String>(0)
                })?
                .filter_map(|row| row.ok())
                .filter_map(|host_id| Uuid::parse_str(&host_id).ok())
                .collect::<HashSet<_>>();

            for host_id in existing.difference(&requested) {
                conn.execute(
                    "UPDATE hosts SET ssh_auth_type = 'password', ssh_key_id = NULL,
                     ssh_key_path = '', updated_at = ?1 WHERE id = ?2",
                    params![Utc::now().to_rfc3339(), host_id.to_string()],
                )?;
            }
            for host_id in &requested {
                conn.execute(
                    "UPDATE hosts SET ssh_auth_type = 'key', ssh_key_id = ?1,
                     ssh_key_path = ?2, ssh_password = '', updated_at = ?3 WHERE id = ?4",
                    params![
                        id.to_string(),
                        storage_path,
                        Utc::now().to_rfc3339(),
                        host_id.to_string(),
                    ],
                )?;
            }

            let mut hosts = Vec::new();
            for host_id in existing.union(&requested) {
                if let Some(host) = get_host_conn(conn, *host_id)? {
                    hosts.push(host);
                }
            }
            Ok(hosts)
        })
    }

    pub fn list_hosts(&self) -> Result<Vec<Host>> {
        self.with_conn(|conn| {
            let query = format!("{HOST_SELECT} ORDER BY is_system DESC, created_at DESC");
            let mut stmt = conn.prepare(&query)?;
            let rows = stmt.query_map([], read_host_record)?;

            let mut hosts = Vec::new();
            for row in rows {
                hosts.push(host_from_record(conn, row?)?);
            }
            Ok(hosts)
        })
    }

    pub fn get_host(&self, id: Uuid) -> Result<Option<Host>> {
        self.with_conn(|conn| get_host_conn(conn, id))
    }

    pub fn ensure_system_host_with_details(&self, address: &str, region: &str) -> Result<Host> {
        let id = system_host_id();
        let name = "本机";
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
                    name,
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
                 name = ?1, address = ?2,
                 region = CASE WHEN TRIM(?3) <> '' THEN ?3 ELSE region END,
                 agent_id = ?4, updated_at = ?5 WHERE id = ?6",
                params![name, address, region, id.to_string(), now, id.to_string()],
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
        let encrypted_password = if req.ssh_auth_type == SshAuthType::Password {
            self.credentials.encrypt(&req.ssh_password)?
        } else {
            String::new()
        };
        self.with_conn(|conn| {
            let (ssh_key_id, ssh_key_path) =
                resolve_ssh_key(conn, req.ssh_auth_type, req.ssh_key_id)?;
            conn.execute(
                "INSERT INTO hosts (
                    id, name, address, region, expires_at, ssh_user, ssh_port, ssh_password,
                    ssh_auth_type, ssh_key_id, ssh_key_path, tags_json, status, agent_id,
                    agent_token, latest_json, last_seen, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                          NULL, ?14, NULL, NULL, ?15, ?16)",
                params![
                    id.to_string(),
                    req.name,
                    req.address,
                    req.region,
                    req.expires_at.map(|value| value.to_rfc3339()),
                    req.ssh_user,
                    req.ssh_port as i64,
                    encrypted_password,
                    req.ssh_auth_type.as_str(),
                    ssh_key_id.map(|value| value.to_string()),
                    ssh_key_path,
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
            let (ssh_key_id, ssh_key_path) =
                resolve_ssh_key(conn, req.ssh_auth_type, req.ssh_key_id)?;
            let n = if req.ssh_auth_type == SshAuthType::Key {
                conn.execute(
                    "UPDATE hosts SET name = ?1, address = ?2, region = ?3, ssh_user = ?4, ssh_port = ?5,
                     ssh_auth_type = 'key', ssh_key_id = ?6, ssh_key_path = ?7, ssh_password = '',
                     tags_json = ?8, expires_at = ?9, updated_at = ?10 WHERE id = ?11",
                    params![
                        req.name,
                        req.address,
                        req.region,
                        req.ssh_user,
                        req.ssh_port as i64,
                        ssh_key_id.map(|value| value.to_string()),
                        ssh_key_path,
                        tags_json,
                        req.expires_at.map(|value| value.to_rfc3339()),
                        now,
                        id.to_string()
                    ],
                )?
            } else if req.clear_ssh_password {
                conn.execute(
                    "UPDATE hosts SET name = ?1, address = ?2, region = ?3, ssh_user = ?4, ssh_port = ?5,
                     ssh_auth_type = 'password', ssh_key_id = NULL, ssh_key_path = '', ssh_password = '',
                     tags_json = ?6, expires_at = ?7, updated_at = ?8 WHERE id = ?9",
                    params![
                        req.name,
                        req.address,
                        req.region,
                        req.ssh_user,
                        req.ssh_port as i64,
                        tags_json,
                        req.expires_at.map(|value| value.to_rfc3339()),
                        now,
                        id.to_string()
                    ],
                )?
            } else if !req.ssh_password.is_empty() {
                conn.execute(
                    "UPDATE hosts SET name = ?1, address = ?2, region = ?3, ssh_user = ?4, ssh_port = ?5,
                     ssh_auth_type = 'password', ssh_key_id = NULL, ssh_key_path = '', ssh_password = ?6,
                     tags_json = ?7, expires_at = ?8, updated_at = ?9 WHERE id = ?10",
                    params![
                        req.name,
                        req.address,
                        req.region,
                        req.ssh_user,
                        req.ssh_port as i64,
                        encrypted_password,
                        tags_json,
                        req.expires_at.map(|value| value.to_rfc3339()),
                        now,
                        id.to_string()
                    ],
                )?
            } else {
                conn.execute(
                    "UPDATE hosts SET name = ?1, address = ?2, region = ?3, ssh_user = ?4, ssh_port = ?5,
                     ssh_auth_type = 'password', ssh_key_id = NULL, ssh_key_path = '',
                     tags_json = ?6, expires_at = ?7, updated_at = ?8 WHERE id = ?9",
                    params![
                        req.name,
                        req.address,
                        req.region,
                        req.ssh_user,
                        req.ssh_port as i64,
                        tags_json,
                        req.expires_at.map(|value| value.to_rfc3339()),
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

    pub fn add_host_domain(
        &self,
        host_id: Uuid,
        request: CreateHostDomainRequest,
    ) -> Result<Option<Host>> {
        self.with_conn(|conn| {
            let host_exists = conn
                .query_row(
                    "SELECT 1 FROM hosts WHERE id = ?1",
                    params![host_id.to_string()],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if !host_exists {
                return Ok(None);
            }
            conn.execute(
                "INSERT OR IGNORE INTO host_domains (
                    id, host_id, domain, port, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    Uuid::new_v4().to_string(),
                    host_id.to_string(),
                    request.domain,
                    request.port as i64,
                    Utc::now().to_rfc3339(),
                ],
            )?;
            get_host_conn(conn, host_id)
        })
    }

    pub fn delete_host_domain(&self, host_id: Uuid, domain_id: Uuid) -> Result<Option<Host>> {
        self.with_conn(|conn| {
            let changed = conn.execute(
                "DELETE FROM host_domains WHERE id = ?1 AND host_id = ?2",
                params![domain_id.to_string(), host_id.to_string()],
            )?;
            if changed == 0 {
                return Ok(None);
            }
            get_host_conn(conn, host_id)
        })
    }

    pub fn apply_probe_results(
        &self,
        host_id: Uuid,
        host_probe: &HostProbeResult,
        domain_probes: &[DomainProbeResult],
    ) -> Result<Option<Host>> {
        self.with_conn(|conn| {
            let changed = conn.execute(
                "UPDATE hosts SET resolved_ipv4_json = ?1, resolved_ipv6_json = ?2,
                 latency_ms = ?3, packet_loss_percent = ?4, last_probed_at = ?5,
                 probe_error = ?6,
                 region = CASE WHEN ?7 <> '' THEN ?7 ELSE region END,
                 updated_at = ?5 WHERE id = ?8",
                params![
                    serde_json::to_string(&host_probe.resolved_ipv4)?,
                    serde_json::to_string(&host_probe.resolved_ipv6)?,
                    host_probe.latency_ms,
                    host_probe.packet_loss_percent,
                    host_probe.checked_at.to_rfc3339(),
                    host_probe.error.as_deref().unwrap_or_default(),
                    host_probe.region,
                    host_id.to_string(),
                ],
            )?;
            if changed == 0 {
                return Ok(None);
            }

            for probe in domain_probes {
                conn.execute(
                    "UPDATE host_domains SET resolved_ipv4_json = ?1, resolved_ipv6_json = ?2,
                     ssl_expires_at = ?3, ssl_status = ?4, latency_ms = ?5,
                     packet_loss_percent = ?6, last_checked_at = ?7, last_error = ?8
                     WHERE id = ?9 AND host_id = ?10",
                    params![
                        serde_json::to_string(&probe.resolved_ipv4)?,
                        serde_json::to_string(&probe.resolved_ipv6)?,
                        probe.ssl_expires_at.map(|value| value.to_rfc3339()),
                        probe.ssl_status,
                        probe.latency_ms,
                        probe.packet_loss_percent,
                        probe.checked_at.to_rfc3339(),
                        probe.error.as_deref().unwrap_or_default(),
                        probe.id.to_string(),
                        host_id.to_string(),
                    ],
                )?;
            }
            get_host_conn(conn, host_id)
        })
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
                        network_rx_bytes, network_tx_bytes, network_rx_rate, network_tx_rate
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
                        network_rx_rate: row.get(7)?,
                        network_tx_rate: row.get(8)?,
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
    let sample = normalize_network_rates(conn, host_id, sample)?;
    let latest_json = serde_json::to_string(&sample)?;
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
            network_rx_bytes, network_tx_bytes, network_rx_rate, network_tx_rate
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            host_id.to_string(),
            sample.collected_at.to_rfc3339(),
            sample.cpu_percent as f64,
            memory_percent,
            disk_percent,
            sample.load_average[0],
            sample.network_rx_bytes as i64,
            sample.network_tx_bytes as i64,
            sample.network_rx_rate,
            sample.network_tx_rate,
        ],
    )?;
    let retention = (now - Duration::days(7)).to_rfc3339();
    conn.execute(
        "DELETE FROM metric_history WHERE collected_at < ?1",
        params![retention],
    )?;
    get_host_conn(conn, host_id)
}

fn normalize_network_rates(
    conn: &Connection,
    host_id: Uuid,
    sample: &SystemSample,
) -> Result<SystemSample> {
    let mut normalized = sample.clone();
    normalized.network_rx_rate = valid_network_rate(normalized.network_rx_rate);
    normalized.network_tx_rate = valid_network_rate(normalized.network_tx_rate);

    if normalized.network_rx_rate.is_some() && normalized.network_tx_rate.is_some() {
        return Ok(normalized);
    }

    let previous = conn
        .query_row(
            "SELECT latest_json FROM hosts WHERE id = ?1",
            params![host_id.to_string()],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten()
        .and_then(|json| serde_json::from_str::<SystemSample>(&json).ok());

    if let Some(previous) = previous {
        let elapsed =
            (normalized.collected_at - previous.collected_at).num_milliseconds() as f64 / 1000.0;
        if elapsed > 0.0 {
            normalized.network_rx_rate.get_or_insert_with(|| {
                normalized
                    .network_rx_bytes
                    .saturating_sub(previous.network_rx_bytes) as f64
                    / elapsed
            });
            normalized.network_tx_rate.get_or_insert_with(|| {
                normalized
                    .network_tx_bytes
                    .saturating_sub(previous.network_tx_bytes) as f64
                    / elapsed
            });
        }
    }

    normalized.network_rx_rate.get_or_insert(0.0);
    normalized.network_tx_rate.get_or_insert(0.0);
    Ok(normalized)
}

fn valid_network_rate(rate: Option<f64>) -> Option<f64> {
    rate.filter(|rate| rate.is_finite() && *rate >= 0.0)
}

const HOST_SELECT: &str =
    "SELECT id, name, address, region, ssh_user, ssh_port, ssh_password, ssh_key_path,
            ssh_auth_type, ssh_key_id,
            (SELECT name FROM ssh_keys WHERE id = hosts.ssh_key_id),
            tags_json, status, agent_id, latest_json, last_seen, update_interval_seconds,
            created_at, is_system, expires_at, resolved_ipv4_json, resolved_ipv6_json,
            latency_ms, packet_loss_percent, last_probed_at, probe_error
     FROM hosts";

struct HostRecord {
    id: String,
    name: String,
    address: String,
    region: String,
    ssh_user: String,
    ssh_port: i64,
    ssh_password: String,
    ssh_key_path: String,
    ssh_auth_type: String,
    ssh_key_id: Option<String>,
    ssh_key_name: Option<String>,
    tags_json: String,
    status: String,
    agent_id: Option<String>,
    latest_json: Option<String>,
    last_seen: Option<String>,
    update_interval_seconds: i64,
    created_at: String,
    is_system: i64,
    expires_at: Option<String>,
    resolved_ipv4_json: String,
    resolved_ipv6_json: String,
    latency_ms: Option<f64>,
    packet_loss_percent: Option<f64>,
    last_probed_at: Option<String>,
    probe_error: String,
}

fn read_host_record(row: &Row<'_>) -> rusqlite::Result<HostRecord> {
    Ok(HostRecord {
        id: row.get(0)?,
        name: row.get(1)?,
        address: row.get(2)?,
        region: row.get(3)?,
        ssh_user: row.get(4)?,
        ssh_port: row.get(5)?,
        ssh_password: row.get(6)?,
        ssh_key_path: row.get(7)?,
        ssh_auth_type: row.get(8)?,
        ssh_key_id: row.get(9)?,
        ssh_key_name: row.get(10)?,
        tags_json: row.get(11)?,
        status: row.get(12)?,
        agent_id: row.get(13)?,
        latest_json: row.get(14)?,
        last_seen: row.get(15)?,
        update_interval_seconds: row.get(16)?,
        created_at: row.get(17)?,
        is_system: row.get(18)?,
        expires_at: row.get(19)?,
        resolved_ipv4_json: row.get(20)?,
        resolved_ipv6_json: row.get(21)?,
        latency_ms: row.get(22)?,
        packet_loss_percent: row.get(23)?,
        last_probed_at: row.get(24)?,
        probe_error: row.get(25)?,
    })
}

fn get_host_conn(conn: &Connection, id: Uuid) -> Result<Option<Host>> {
    let query = format!("{HOST_SELECT} WHERE id = ?1");
    let row = conn
        .query_row(&query, params![id.to_string()], read_host_record)
        .optional()?;

    let Some(record) = row else {
        return Ok(None);
    };

    host_from_record(conn, record).map(Some)
}

fn host_from_record(conn: &Connection, record: HostRecord) -> Result<Host> {
    let host_id = Uuid::parse_str(&record.id)?;
    Ok(Host {
        id: host_id,
        is_system: record.is_system != 0,
        name: record.name,
        address: record.address,
        region: record.region,
        expires_at: record.expires_at.as_deref().map(parse_dt),
        resolved_ipv4: serde_json::from_str(&record.resolved_ipv4_json).unwrap_or_default(),
        resolved_ipv6: serde_json::from_str(&record.resolved_ipv6_json).unwrap_or_default(),
        latency_ms: record.latency_ms,
        packet_loss_percent: record.packet_loss_percent,
        last_probed_at: record.last_probed_at.as_deref().map(parse_dt),
        probe_error: (!record.probe_error.is_empty()).then_some(record.probe_error),
        domains: load_host_domains(conn, &record.id)?,
        ssh_user: record.ssh_user,
        ssh_port: record.ssh_port as u16,
        ssh_auth_type: SshAuthType::parse(&record.ssh_auth_type),
        ssh_key_id: record
            .ssh_key_id
            .as_deref()
            .and_then(|value| Uuid::parse_str(value).ok()),
        ssh_key_name: record.ssh_key_name,
        update_interval_seconds: record.update_interval_seconds.max(1) as u64,
        has_ssh_password: !record.ssh_password.is_empty(),
        has_ssh_identity: !record.ssh_key_path.is_empty(),
        tags: serde_json::from_str(&record.tags_json).unwrap_or_default(),
        status: HostStatus::parse(&record.status),
        agent_id: record
            .agent_id
            .and_then(|value| Uuid::parse_str(&value).ok()),
        latest: record
            .latest_json
            .and_then(|value| serde_json::from_str(&value).ok()),
        last_seen: record.last_seen.as_deref().map(parse_dt),
        install_logs: load_install_logs(conn, &record.id)?,
        created_at: parse_dt(&record.created_at),
    })
}

fn resolve_ssh_key(
    conn: &Connection,
    auth_type: SshAuthType,
    key_id: Option<Uuid>,
) -> Result<(Option<Uuid>, String)> {
    if auth_type == SshAuthType::Password {
        return Ok((None, String::new()));
    }
    let key_id = key_id.context("SSH key is required for key authentication")?;
    let storage_path = conn
        .query_row(
            "SELECT storage_path FROM ssh_keys WHERE id = ?1",
            params![key_id.to_string()],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .context("selected SSH key no longer exists")?;
    Ok((Some(key_id), storage_path))
}

fn load_host_domains(conn: &Connection, host_id: &str) -> Result<Vec<HostDomain>> {
    let mut statement = conn.prepare(
        "SELECT id, domain, port, resolved_ipv4_json, resolved_ipv6_json, ssl_expires_at,
                ssl_status, latency_ms, packet_loss_percent, last_checked_at, last_error, created_at
         FROM host_domains WHERE host_id = ?1 ORDER BY domain ASC, port ASC",
    )?;
    statement
        .query_map(params![host_id], |row| {
            let ssl_expires_at: Option<String> = row.get(5)?;
            let last_checked_at: Option<String> = row.get(9)?;
            let last_error: String = row.get(10)?;
            Ok(HostDomain {
                id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_else(|_| Uuid::nil()),
                domain: row.get(1)?,
                port: row.get::<_, i64>(2)?.clamp(1, u16::MAX as i64) as u16,
                resolved_ipv4: serde_json::from_str(&row.get::<_, String>(3)?).unwrap_or_default(),
                resolved_ipv6: serde_json::from_str(&row.get::<_, String>(4)?).unwrap_or_default(),
                ssl_expires_at: ssl_expires_at.as_deref().map(parse_dt),
                ssl_status: row.get(6)?,
                latency_ms: row.get(7)?,
                packet_loss_percent: row.get(8)?,
                last_checked_at: last_checked_at.as_deref().map(parse_dt),
                last_error: (!last_error.is_empty()).then_some(last_error),
                created_at: parse_dt(&row.get::<_, String>(11)?),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
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
                network_rx_rate: average_optional_rate(bucket, |point| point.network_rx_rate),
                network_tx_rate: average_optional_rate(bucket, |point| point.network_tx_rate),
            }
        })
        .collect()
}

fn average_optional_rate(
    points: &[MetricHistoryPoint],
    rate: impl Fn(&MetricHistoryPoint) -> Option<f64>,
) -> Option<f64> {
    let (sum, count) = points
        .iter()
        .filter_map(rate)
        .fold((0.0, 0usize), |(sum, count), value| {
            (sum + value, count + 1)
        });
    (count > 0).then(|| sum / count as f64)
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
            .ensure_system_host_with_details("127.0.0.1", "本机")
            .unwrap();

        assert!(system_host.is_system);
        assert_eq!(system_host.name, "本机");
        assert_eq!(db.list_hosts().unwrap()[0].id, system_host.id);
        assert!(db.contains_system_host(&[system_host.id]).unwrap());
        assert!(db.delete_hosts(&[system_host.id]).unwrap().is_empty());
        assert!(db.get_host(system_host.id).unwrap().is_some());

        let refreshed = db
            .ensure_system_host_with_details("192.168.1.8", "中国 · 广东 · 深圳")
            .unwrap();
        assert_eq!(refreshed.name, "本机");
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
            network_rx_rate: Some(2048.0),
            network_tx_rate: Some(1024.0),
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
        let latest = monitored.latest.unwrap();
        assert_eq!(latest.cpu_percent, 12.5);
        assert_eq!(latest.network_rx_rate, Some(2048.0));
        assert_eq!(latest.network_tx_rate, Some(1024.0));

        let same_host = db
            .ensure_system_host_with_details("127.0.0.1", "本机")
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
                    expires_at: None,
                    ssh_user: String::new(),
                    ssh_port: 22,
                    ssh_auth_type: SshAuthType::Password,
                    ssh_key_id: None,
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
                network_rx_rate: None,
                network_tx_rate: None,
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
        let latest_rx_rate = history.last().unwrap().network_rx_rate.unwrap();
        let latest_tx_rate = history.last().unwrap().network_tx_rate.unwrap();
        assert!((latest_rx_rate - 1024.0).abs() < 5.0);
        assert!((latest_tx_rate - 512.0).abs() < 5.0);
        assert!(history.last().unwrap().memory_percent > 0.0);

        drop(db);
        cleanup(&path);
    }

    #[test]
    fn stores_host_domains_expiry_and_probe_results() {
        let path = std::env::temp_dir().join(format!("lightmonitor-domains-{}.db", Uuid::new_v4()));
        let db = Db::open(&path).unwrap();
        let expires_at = Utc::now() + Duration::days(90);
        let host = db
            .create_host(
                CreateHostRequest {
                    name: "domain-test".to_string(),
                    address: "example.com".to_string(),
                    region: "🇺🇸 美国".to_string(),
                    expires_at: Some(expires_at),
                    ssh_user: String::new(),
                    ssh_port: 22,
                    ssh_auth_type: SshAuthType::Password,
                    ssh_key_id: None,
                    ssh_password: String::new(),
                    tags: Vec::new(),
                },
                "domain-token".to_string(),
            )
            .unwrap();
        let host = db
            .add_host_domain(
                host.id,
                CreateHostDomainRequest {
                    domain: "example.com".to_string(),
                    port: 443,
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(host.domains.len(), 1);
        assert_eq!(host.expires_at.unwrap().timestamp(), expires_at.timestamp());

        let checked_at = Utc::now();
        let ssl_expires_at = checked_at + Duration::days(60);
        let domain_id = host.domains[0].id;
        let host = db
            .apply_probe_results(
                host.id,
                &HostProbeResult {
                    resolved_ipv4: vec!["192.0.2.10".to_string()],
                    resolved_ipv6: vec!["2001:db8::10".to_string()],
                    latency_ms: Some(12.5),
                    packet_loss_percent: Some(25.0),
                    checked_at,
                    error: None,
                    region: "🇺🇸 美国 · 加利福尼亚".to_string(),
                },
                &[DomainProbeResult {
                    id: domain_id,
                    resolved_ipv4: vec!["192.0.2.20".to_string()],
                    resolved_ipv6: Vec::new(),
                    ssl_expires_at: Some(ssl_expires_at),
                    ssl_status: "valid".to_string(),
                    latency_ms: Some(18.0),
                    packet_loss_percent: Some(0.0),
                    checked_at,
                    error: None,
                }],
            )
            .unwrap()
            .unwrap();
        assert_eq!(host.resolved_ipv4, vec!["192.0.2.10"]);
        assert_eq!(host.resolved_ipv6, vec!["2001:db8::10"]);
        assert_eq!(host.latency_ms, Some(12.5));
        assert_eq!(host.packet_loss_percent, Some(25.0));
        assert!(host.region.starts_with("🇺🇸"));
        assert_eq!(host.domains[0].ssl_status, "valid");
        assert_eq!(host.domains[0].latency_ms, Some(18.0));
        assert_eq!(
            host.domains[0].ssl_expires_at.unwrap().timestamp(),
            ssl_expires_at.timestamp()
        );

        let host = db.delete_host_domain(host.id, domain_id).unwrap().unwrap();
        assert!(host.domains.is_empty());
        drop(db);
        cleanup(&path);
    }

    #[test]
    fn migrates_probe_columns_for_existing_databases() {
        let path = std::env::temp_dir().join(format!(
            "lightmonitor-probe-migration-{}.db",
            Uuid::new_v4()
        ));
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE hosts (
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
            CREATE TABLE ssh_keys (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                storage_path TEXT NOT NULL UNIQUE,
                size_bytes INTEGER NOT NULL,
                updated_at TEXT NOT NULL
            );",
        )
        .unwrap();
        let legacy_host_id = Uuid::new_v4();
        let legacy_key_id = Uuid::new_v4();
        let legacy_key_path = std::env::temp_dir()
            .join(format!("lightmonitor-legacy-key-{}", Uuid::new_v4()))
            .to_string_lossy()
            .to_string();
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO ssh_keys (id, name, storage_path, size_bytes, updated_at)
             VALUES (?1, 'legacy-key', ?2, 42, ?3)",
            params![legacy_key_id.to_string(), legacy_key_path, now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO hosts (
                id, name, address, ssh_user, ssh_port, ssh_key_path, tags_json, status,
                agent_token, created_at, updated_at
             ) VALUES (?1, 'legacy-host', '192.0.2.40', 'root', 22, ?2, '[]',
                       'pending', 'legacy-token', ?3, ?3)",
            params![legacy_host_id.to_string(), legacy_key_path, now],
        )
        .unwrap();
        drop(conn);

        let db = Db::open(&path).unwrap();
        db.with_conn(|conn| {
            let mut statement = conn.prepare("PRAGMA table_info(hosts)")?;
            let columns = statement
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            assert!(columns.contains(&"expires_at".to_string()));
            assert!(columns.contains(&"resolved_ipv4_json".to_string()));
            assert!(columns.contains(&"resolved_ipv6_json".to_string()));
            assert!(columns.contains(&"latency_ms".to_string()));
            assert!(columns.contains(&"packet_loss_percent".to_string()));
            assert!(columns.contains(&"ssh_auth_type".to_string()));
            assert!(columns.contains(&"ssh_key_id".to_string()));
            let domain_table: i64 = conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'host_domains'",
                [],
                |row| row.get(0),
            )?;
            assert_eq!(domain_table, 1);
            Ok(())
        })
        .unwrap();
        let legacy_host = db.get_host(legacy_host_id).unwrap().unwrap();
        assert_eq!(legacy_host.ssh_auth_type, SshAuthType::Key);
        assert_eq!(legacy_host.ssh_key_id, Some(legacy_key_id));
        assert_eq!(legacy_host.ssh_key_name.as_deref(), Some("legacy-key"));
        drop(db);
        cleanup(&path);
    }

    #[test]
    fn manages_ssh_keys_and_prevents_deleting_keys_in_use() {
        let path =
            std::env::temp_dir().join(format!("lightmonitor-ssh-keys-{}.db", Uuid::new_v4()));
        let db = Db::open(&path).unwrap();
        let key_id = Uuid::new_v4();
        let storage_path = std::env::temp_dir()
            .join(format!("lightmonitor-ssh-key-{}", Uuid::new_v4()))
            .to_string_lossy()
            .to_string();
        db.create_ssh_key(key_id, "production", &storage_path, 42)
            .unwrap();
        assert_eq!(db.list_ssh_keys().unwrap()[0].name, "production");

        let host = db
            .create_host(
                CreateHostRequest {
                    name: "key-host".to_string(),
                    address: "192.0.2.20".to_string(),
                    region: String::new(),
                    expires_at: None,
                    ssh_user: "root".to_string(),
                    ssh_port: 22,
                    ssh_auth_type: SshAuthType::Password,
                    ssh_key_id: None,
                    ssh_password: String::new(),
                    tags: Vec::new(),
                },
                "key-host-token".to_string(),
            )
            .unwrap();
        let assigned = db.assign_ssh_key_hosts(key_id, &[host.id]).unwrap();
        assert_eq!(assigned[0].ssh_auth_type, SshAuthType::Key);
        assert_eq!(assigned[0].ssh_key_id, Some(key_id));
        let key = &db.list_ssh_keys().unwrap()[0];
        assert!(key.in_use);
        assert_eq!(key.host_ids, vec![host.id]);
        assert!(db.delete_ssh_key(key_id).is_err());

        db.assign_ssh_key_hosts(key_id, &[]).unwrap();
        let host = db.get_host(host.id).unwrap().unwrap();
        assert_eq!(host.ssh_auth_type, SshAuthType::Password);
        assert_eq!(host.ssh_key_id, None);
        assert_eq!(db.delete_ssh_key(key_id).unwrap(), Some(storage_path));
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
                    expires_at: None,
                    ssh_user: "root".to_string(),
                    ssh_port: 22,
                    ssh_auth_type: SshAuthType::Password,
                    ssh_key_id: None,
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

    #[test]
    fn switches_host_between_password_and_key_authentication() {
        let path =
            std::env::temp_dir().join(format!("lightmonitor-auth-switch-{}.db", Uuid::new_v4()));
        let db = Db::open(&path).unwrap();
        let key_id = Uuid::new_v4();
        let key_path = std::env::temp_dir()
            .join(format!("lightmonitor-auth-key-{}", Uuid::new_v4()))
            .to_string_lossy()
            .to_string();
        db.create_ssh_key(key_id, "switch-key", &key_path, 42)
            .unwrap();

        let host = db
            .create_host(
                CreateHostRequest {
                    name: "auth-host".to_string(),
                    address: "192.0.2.50".to_string(),
                    region: String::new(),
                    expires_at: None,
                    ssh_user: "root".to_string(),
                    ssh_port: 22,
                    ssh_auth_type: SshAuthType::Key,
                    ssh_key_id: Some(key_id),
                    ssh_password: "must-not-be-stored".to_string(),
                    tags: Vec::new(),
                },
                "auth-switch-token".to_string(),
            )
            .unwrap();
        assert_eq!(host.ssh_auth_type, SshAuthType::Key);
        assert_eq!(host.ssh_key_id, Some(key_id));
        assert!(!host.has_ssh_password);
        assert!(host.has_ssh_identity);

        let host = db
            .update_host(
                host.id,
                UpdateHostRequest {
                    name: host.name.clone(),
                    address: host.address.clone(),
                    region: host.region.clone(),
                    expires_at: None,
                    ssh_user: host.ssh_user.clone(),
                    ssh_port: host.ssh_port,
                    ssh_auth_type: SshAuthType::Password,
                    ssh_key_id: None,
                    ssh_password: "new-secret".to_string(),
                    clear_ssh_password: false,
                    tags: Vec::new(),
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(host.ssh_auth_type, SshAuthType::Password);
        assert_eq!(host.ssh_key_id, None);
        assert!(host.has_ssh_password);
        assert!(!host.has_ssh_identity);
        assert_eq!(
            db.ssh_credentials(host.id).unwrap().unwrap().0,
            "new-secret"
        );

        let host = db
            .update_host(
                host.id,
                UpdateHostRequest {
                    name: host.name.clone(),
                    address: host.address.clone(),
                    region: host.region.clone(),
                    expires_at: None,
                    ssh_user: host.ssh_user.clone(),
                    ssh_port: host.ssh_port,
                    ssh_auth_type: SshAuthType::Key,
                    ssh_key_id: Some(key_id),
                    ssh_password: String::new(),
                    clear_ssh_password: false,
                    tags: Vec::new(),
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(host.ssh_auth_type, SshAuthType::Key);
        assert_eq!(host.ssh_key_id, Some(key_id));
        assert!(!host.has_ssh_password);
        assert_eq!(db.ssh_credentials(host.id).unwrap().unwrap().0, "");

        drop(db);
        cleanup(&path);
    }
}
