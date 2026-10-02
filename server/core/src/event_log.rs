use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::jobs::{JobSnapshot, JobState, StoredJob};

const EVENT_LIMIT_MAX: usize = 500;
const DEFAULT_RETENTION_DAYS: u64 = 30;
const DEFAULT_MAX_ROWS: u64 = 100_000;
const CLEANUP_INTERVAL: u64 = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventSeverity {
    Info,
    Warning,
    Error,
}

impl EventSeverity {
    fn as_str(self) -> &'static str {
        match self { Self::Info => "info", Self::Warning => "warning", Self::Error => "error" }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "info" => Ok(Self::Info),
            "warning" => Ok(Self::Warning),
            "error" => Ok(Self::Error),
            _ => anyhow::bail!("invalid event severity '{value}'"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeEvent {
    pub id: u64,
    pub timestamp_unix_ms: u64,
    pub severity: EventSeverity,
    pub category: String,
    pub kind: String,
    pub message: String,
    pub metadata: Option<Value>,
    pub plugin_id: Option<String>,
    pub lease_id: Option<String>,
    pub job_id: Option<String>,
    pub request_id: Option<String>,
    pub source: Option<String>,
    pub actor: Option<String>,
    pub client_ip: Option<String>,
    pub peer_ip: Option<String>,
    pub http_method: Option<String>,
    pub http_path: Option<String>,
    pub http_status: Option<u16>,
    pub duration_us: Option<u64>,
    pub request_bytes: Option<u64>,
    pub response_bytes: Option<u64>,
}

#[derive(Debug, Default, Clone)]
pub struct AuditFields {
    pub actor: Option<String>,
    pub client_ip: Option<String>,
    pub peer_ip: Option<String>,
    pub http_method: Option<String>,
    pub http_path: Option<String>,
    pub http_status: Option<u16>,
    pub duration_us: Option<u64>,
    pub request_bytes: Option<u64>,
    pub response_bytes: Option<u64>,
}

#[derive(Debug, Default, Clone)]
pub struct EventQuery {
    pub cursor: Option<u64>,
    pub after: Option<u64>,
    pub severity: Option<EventSeverity>,
    pub category: Option<String>,
    pub kind: Option<String>,
    pub plugin_id: Option<String>,
    pub lease_id: Option<String>,
    pub job_id: Option<String>,
    pub request_id: Option<String>,
    pub actor: Option<String>,
    pub client_ip: Option<String>,
    pub http_method: Option<String>,
    pub http_status: Option<u16>,
    pub source: Option<String>,
    pub exclude_source: Option<String>,
    pub q: Option<String>,
    pub outcome: Option<String>,
    pub path_prefix: Option<String>,
    pub from_unix_ms: Option<u64>,
    pub to_unix_ms: Option<u64>,
    pub limit: usize,
}

#[derive(Debug, Serialize)]
pub struct EventPage {
    pub items: Vec<RuntimeEvent>,
    pub next_cursor: Option<u64>,
}

#[derive(Clone)]
pub struct EventStore {
    database_path: Arc<PathBuf>,
    retention: Arc<EventRetention>,
    writes_since_cleanup: Arc<AtomicU64>,
}

#[derive(Debug)]
struct EventRetention {
    days: u64,
    max_rows: u64,
}

impl EventStore {
    pub async fn open(state_root: PathBuf) -> Result<Self> {
        let retention = Arc::new(EventRetention::from_env()?);
        let database_path = state_root.join("rkserve.sqlite3");
        let migration_root = state_root;
        let opened_path = database_path.clone();
        let startup_retention = retention.clone();
        tokio::task::spawn_blocking(move || {
            fs::create_dir_all(&migration_root)
                .with_context(|| format!("create state directory {}", migration_root.display()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&migration_root, fs::Permissions::from_mode(0o700))
                    .with_context(|| format!("secure state directory {}", migration_root.display()))?;
            }
            let connection = open_connection(&opened_path)?;
            initialize_schema(&connection)?;
            migrate_event_schema(&connection)?;
            migrate_event_journals(&connection, &migration_root)?;
            prune_events(&connection, &startup_retention)?;
            Ok::<_, anyhow::Error>(())
        })
        .await
        .context("join SQLite initialization")??;
        Ok(Self {
            database_path: Arc::new(database_path),
            retention,
            writes_since_cleanup: Arc::new(AtomicU64::new(0)),
        })
    }

    pub async fn record(
        &self,
        severity: EventSeverity,
        category: impl Into<String>,
        kind: impl Into<String>,
        message: impl Into<String>,
        metadata: Option<Value>,
        plugin_id: Option<String>,
        lease_id: Option<String>,
        job_id: Option<String>,
        request_id: Option<String>,
        source: Option<String>,
    ) -> Result<RuntimeEvent> {
        self.record_with_audit(
            severity,
            category.into(),
            kind.into(),
            message.into(),
            metadata,
            plugin_id,
            lease_id,
            job_id,
            request_id,
            source,
            AuditFields::default(),
        )
        .await
    }

    async fn record_with_audit(
        &self,
        severity: EventSeverity,
        category: String,
        kind: String,
        message: String,
        metadata: Option<Value>,
        plugin_id: Option<String>,
        lease_id: Option<String>,
        job_id: Option<String>,
        request_id: Option<String>,
        source: Option<String>,
        audit: AuditFields,
    ) -> Result<RuntimeEvent> {
        // minimal-debt: writes open short-lived SQLite connections; introduce
        // a pool only if concurrent event volume makes this measurable.
        let path = self.database_path.as_ref().clone();
        let retention = self.retention.clone();
        let should_cleanup =
            self.writes_since_cleanup.fetch_add(1, Ordering::Relaxed) % CLEANUP_INTERVAL == 0;
        tokio::task::spawn_blocking(move || {
            let connection = open_connection(&path)?;
            let timestamp_unix_ms = now_ms();
            let metadata_json = metadata.as_ref().map(serde_json::to_string).transpose().context("serialize event metadata")?;
            connection.execute(
                "INSERT INTO events (timestamp_unix_ms, severity, category, kind, message, metadata, plugin_id, lease_id, job_id, request_id, source, actor, client_ip, peer_ip, http_method, http_path, http_status, duration_us, request_bytes, response_bytes) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
                params![timestamp_unix_ms as i64, severity.as_str(), category, kind, message, metadata_json, plugin_id, lease_id, job_id, request_id, source, audit.actor, audit.client_ip, audit.peer_ip, audit.http_method, audit.http_path, audit.http_status.map(i64::from), audit.duration_us.map(|value| value as i64), audit.request_bytes.map(|value| value as i64), audit.response_bytes.map(|value| value as i64)],
            ).context("insert runtime event")?;
            let event = RuntimeEvent {
                id: connection.last_insert_rowid() as u64,
                timestamp_unix_ms,
                severity,
                category,
                kind,
                message,
                metadata,
                plugin_id,
                lease_id,
                job_id,
                request_id,
                source,
                actor: audit.actor,
                client_ip: audit.client_ip,
                peer_ip: audit.peer_ip,
                http_method: audit.http_method,
                http_path: audit.http_path,
                http_status: audit.http_status,
                duration_us: audit.duration_us,
                request_bytes: audit.request_bytes,
                response_bytes: audit.response_bytes,
            };
            if should_cleanup {
                prune_events(&connection, &retention)?;
            }
            Ok::<_, anyhow::Error>(event)
        }).await.context("join event insert")?
    }

    pub async fn record_audit_best_effort(
        &self,
        severity: EventSeverity,
        category: impl Into<String>,
        kind: impl Into<String>,
        message: impl Into<String>,
        metadata: Option<Value>,
        plugin_id: Option<String>,
        lease_id: Option<String>,
        job_id: Option<String>,
        request_id: Option<String>,
        source: Option<String>,
        audit: AuditFields,
    ) {
        if let Err(error) = self
            .record_with_audit(
                severity,
                category.into(),
                kind.into(),
                message.into(),
                metadata,
                plugin_id,
                lease_id,
                job_id,
                request_id,
                source,
                audit,
            )
            .await
        {
            tracing::warn!(%error, "failed to persist API audit event");
        }
    }

    /// Persists an observability event without changing the outcome of the
    /// control-plane operation that produced it. Persistence failures remain
    /// visible in the process log instead of being silently discarded.
    pub async fn record_best_effort(
        &self,
        severity: EventSeverity,
        category: impl Into<String>,
        kind: impl Into<String>,
        message: impl Into<String>,
        metadata: Option<Value>,
        plugin_id: Option<String>,
        lease_id: Option<String>,
        job_id: Option<String>,
        request_id: Option<String>,
        source: Option<String>,
    ) {
        if let Err(error) = self
            .record(
                severity,
                category,
                kind,
                message,
                metadata,
                plugin_id,
                lease_id,
                job_id,
                request_id,
                source,
            )
            .await
        {
            tracing::warn!(%error, "failed to persist runtime event");
        }
    }

    pub async fn query(&self, query: EventQuery) -> Result<EventPage> {
        let path = self.database_path.as_ref().clone();
        tokio::task::spawn_blocking(move || query_events(&path, query)).await.context("join event query")?
    }

    pub(crate) async fn upsert_job(&self, job: StoredJob) -> Result<()> {
        let path = self.database_path.as_ref().clone();
        tokio::task::spawn_blocking(move || {
            let connection = open_connection(&path)?;
            let snapshot_json = serde_json::to_string(&job.snapshot).context("serialize job snapshot")?;
            connection.execute(
                "INSERT INTO jobs (id, plugin_id, capability_id, state, created_at_unix_ms, finished_at_unix_ms, snapshot_json, result_path) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) ON CONFLICT(id) DO UPDATE SET plugin_id=excluded.plugin_id, capability_id=excluded.capability_id, state=excluded.state, created_at_unix_ms=excluded.created_at_unix_ms, finished_at_unix_ms=excluded.finished_at_unix_ms, snapshot_json=excluded.snapshot_json, result_path=excluded.result_path",
                params![job.snapshot.id, job.snapshot.plugin_id, job.snapshot.capability_id, job.snapshot.state.as_str(), job.snapshot.created_at_unix_ms as i64, job.snapshot.finished_at_unix_ms.map(|value| value as i64), snapshot_json, job.result_path.map(|value| value.to_string_lossy().into_owned())],
            ).context("persist job")?;
            Ok::<_, anyhow::Error>(())
        }).await.context("join job persistence")?
    }

    pub(crate) async fn recover_jobs(&self) -> Result<Vec<StoredJob>> {
        let path = self.database_path.as_ref().clone();
        tokio::task::spawn_blocking(move || {
            let mut connection = open_connection(&path)?;
            let transaction = connection.transaction().context("begin job recovery")?;
            let mut statement = transaction.prepare("SELECT snapshot_json, result_path FROM jobs ORDER BY created_at_unix_ms ASC").context("read persisted jobs")?;
            let mut jobs = Vec::new();
            for row in statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)))? {
                let (snapshot_json, result_path) = row.context("read persisted job row")?;
                let mut snapshot: JobSnapshot = serde_json::from_str(&snapshot_json).context("parse persisted job snapshot")?;
                let mut changed = false;
                if snapshot.state.is_active() {
                    // In-flight NPU calls cannot be resumed safely after a
                    // Core restart, so fail them instead of replaying work.
                    snapshot.state = JobState::Failed;
                    snapshot.finished_at_unix_ms = Some(now_ms());
                    snapshot.error = Some("interrupted by core restart".into());
                    changed = true;
                }
                let result_path = result_path.map(PathBuf::from);
                if changed {
                    let updated = serde_json::to_string(&snapshot).context("serialize interrupted job")?;
                    transaction.execute("UPDATE jobs SET state=?1, finished_at_unix_ms=?2, snapshot_json=?3 WHERE id=?4", params![snapshot.state.as_str(), snapshot.finished_at_unix_ms.map(|value| value as i64), updated, snapshot.id]).context("mark interrupted job")?;
                }
                jobs.push(StoredJob { snapshot, result_path });
            }
            drop(statement);
            transaction.commit().context("commit job recovery")?;
            Ok::<_, anyhow::Error>(jobs)
        }).await.context("join job recovery")?
    }

    pub(crate) async fn delete_jobs(&self, ids: Vec<String>) -> Result<()> {
        if ids.is_empty() { return Ok(()); }
        let path = self.database_path.as_ref().clone();
        tokio::task::spawn_blocking(move || {
            let mut connection = open_connection(&path)?;
            let transaction = connection.transaction().context("begin job deletion")?;
            for id in ids { transaction.execute("DELETE FROM jobs WHERE id=?1", [id]).context("delete retained job")?; }
            transaction.commit().context("commit job deletion")?;
            Ok::<_, anyhow::Error>(())
        }).await.context("join job deletion")?
    }
}

fn open_connection(path: &PathBuf) -> Result<Connection> {
    let connection = Connection::open(path).with_context(|| format!("open SQLite database {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("secure SQLite database {}", path.display()))?;
    }
    connection.pragma_update(None, "journal_mode", "WAL").context("enable SQLite WAL")?;
    connection.pragma_update(None, "foreign_keys", "ON").context("enable SQLite foreign keys")?;
    connection.busy_timeout(std::time::Duration::from_secs(5)).context("configure SQLite busy timeout")?;
    Ok(connection)
}

fn initialize_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS events (
            id INTEGER PRIMARY KEY,
            timestamp_unix_ms INTEGER NOT NULL,
            severity TEXT NOT NULL,
            category TEXT NOT NULL,
            kind TEXT NOT NULL,
            message TEXT NOT NULL,
            metadata TEXT,
            plugin_id TEXT,
            lease_id TEXT,
            job_id TEXT,
            request_id TEXT,
            source TEXT,
            actor TEXT,
            client_ip TEXT,
            peer_ip TEXT,
            http_method TEXT,
            http_path TEXT,
            http_status INTEGER,
            duration_us INTEGER,
            request_bytes INTEGER,
            response_bytes INTEGER
        );
        CREATE INDEX IF NOT EXISTS events_timestamp_idx ON events(timestamp_unix_ms DESC, id DESC);
        CREATE INDEX IF NOT EXISTS events_plugin_idx ON events(plugin_id, id DESC);
        CREATE INDEX IF NOT EXISTS events_lease_idx ON events(lease_id, id DESC);
        CREATE INDEX IF NOT EXISTS events_job_idx ON events(job_id, id DESC);
        CREATE TABLE IF NOT EXISTS jobs (
            id TEXT PRIMARY KEY,
            plugin_id TEXT NOT NULL,
            capability_id TEXT NOT NULL,
            state TEXT NOT NULL,
            created_at_unix_ms INTEGER NOT NULL,
            finished_at_unix_ms INTEGER,
            snapshot_json TEXT NOT NULL,
            result_path TEXT
        );
        CREATE INDEX IF NOT EXISTS jobs_created_idx ON jobs(created_at_unix_ms ASC);",
    ).context("initialize SQLite schema")
}

fn migrate_event_schema(connection: &Connection) -> Result<()> {
    let mut statement = connection
        .prepare("PRAGMA table_info(events)")
        .context("inspect event schema")?;
    let existing = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<std::collections::HashSet<_>>>()
        .context("read event schema")?;
    drop(statement);
    for (name, kind) in [
        ("actor", "TEXT"),
        ("client_ip", "TEXT"),
        ("peer_ip", "TEXT"),
        ("http_method", "TEXT"),
        ("http_path", "TEXT"),
        ("http_status", "INTEGER"),
        ("duration_us", "INTEGER"),
        ("request_bytes", "INTEGER"),
        ("response_bytes", "INTEGER"),
    ] {
        if !existing.contains(name) {
            connection
                .execute(&format!("ALTER TABLE events ADD COLUMN {name} {kind}"), [])
                .with_context(|| format!("add events.{name} column"))?;
        }
    }
    connection
        .execute_batch(
            "CREATE INDEX IF NOT EXISTS events_request_idx ON events(request_id, id DESC);
             CREATE INDEX IF NOT EXISTS events_actor_idx ON events(actor, id DESC);
             CREATE INDEX IF NOT EXISTS events_client_ip_idx ON events(client_ip, id DESC);
             CREATE INDEX IF NOT EXISTS events_http_status_idx ON events(http_status, id DESC);
             CREATE INDEX IF NOT EXISTS events_source_idx ON events(source, id DESC);",
        )
        .context("create audit event indexes")
}

fn query_events(path: &PathBuf, query: EventQuery) -> Result<EventPage> {
    let connection = open_connection(path)?;
    let mut sql = String::from("SELECT id, timestamp_unix_ms, severity, category, kind, message, metadata, plugin_id, lease_id, job_id, request_id, source, actor, client_ip, peer_ip, http_method, http_path, http_status, duration_us, request_bytes, response_bytes FROM events WHERE 1=1");
    let mut values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    macro_rules! clause {
        ($value:expr, $column:literal) => { if let Some(value) = $value { sql.push_str(concat!(" AND ", $column, " = ?")); values.push(Box::new(value)); } };
    }
    anyhow::ensure!(query.after.is_none() || query.cursor.is_none(), "after and cursor/before cannot be combined");
    if let Some(cursor) = query.cursor { sql.push_str(" AND id < ?"); values.push(Box::new(cursor as i64)); }
    if let Some(after) = query.after { sql.push_str(" AND id > ?"); values.push(Box::new(after as i64)); }
    clause!(query.severity.map(|value| value.as_str().to_owned()), "severity");
    clause!(query.category, "category");
    clause!(query.kind, "kind");
    clause!(query.plugin_id, "plugin_id");
    clause!(query.lease_id, "lease_id");
    clause!(query.job_id, "job_id");
    clause!(query.request_id, "request_id");
    clause!(query.actor, "actor");
    clause!(query.client_ip, "client_ip");
    clause!(query.http_method, "http_method");
    clause!(query.http_status.map(i64::from), "http_status");
    clause!(query.source, "source");
    if let Some(source) = query.exclude_source {
        sql.push_str(" AND (source IS NULL OR source != ?)");
        values.push(Box::new(source));
    }
    if let Some(raw) = query.q.as_deref().map(str::trim).filter(|value| !value.is_empty()) {
        let pattern = like_contains(raw);
        sql.push_str(" AND (message LIKE ? ESCAPE '\\' OR IFNULL(http_path,'') LIKE ? ESCAPE '\\' OR IFNULL(request_id,'') LIKE ? ESCAPE '\\' OR IFNULL(actor,'') LIKE ? ESCAPE '\\')");
        for _ in 0..4 {
            values.push(Box::new(pattern.clone()));
        }
    }
    match query.outcome.as_deref() {
        None | Some("") => {}
        Some("success") => {
            sql.push_str(" AND COALESCE(http_status, CASE severity WHEN 'error' THEN 500 ELSE 200 END) < 400")
        }
        Some("failure") => {
            sql.push_str(" AND COALESCE(http_status, CASE severity WHEN 'error' THEN 500 ELSE 200 END) >= 400")
        }
        Some(other) => anyhow::bail!("invalid outcome '{other}'"),
    }
    if let Some(raw) = query.path_prefix.as_deref().map(str::trim).filter(|value| !value.is_empty()) {
        sql.push_str(" AND IFNULL(http_path,'') LIKE ? ESCAPE '\\'");
        values.push(Box::new(like_prefix(raw)));
    }
    if let Some(from) = query.from_unix_ms { sql.push_str(" AND timestamp_unix_ms >= ?"); values.push(Box::new(from as i64)); }
    if let Some(to) = query.to_unix_ms { sql.push_str(" AND timestamp_unix_ms <= ?"); values.push(Box::new(to as i64)); }
    let ascending = query.after.is_some();
    sql.push_str(if ascending { " ORDER BY id ASC LIMIT ?" } else { " ORDER BY id DESC LIMIT ?" });
    let limit = query.limit.clamp(1, EVENT_LIMIT_MAX);
    values.push(Box::new((limit + 1) as i64));
    let refs: Vec<&dyn rusqlite::ToSql> = values.iter().map(|value| value.as_ref()).collect();
    let mut statement = connection.prepare(&sql).context("prepare event query")?;
    let mut items = statement.query_map(refs.as_slice(), event_from_row)?.collect::<rusqlite::Result<Vec<_>>>().context("read event query")?;
    let next_cursor = if items.len() > limit { items.pop(); items.last().map(|event| event.id) } else { None };
    Ok(EventPage { items, next_cursor })
}

fn event_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RuntimeEvent> {
    let metadata: Option<String> = row.get(6)?;
    let severity: String = row.get(2)?;
    Ok(RuntimeEvent {
        id: row.get::<_, i64>(0)? as u64,
        timestamp_unix_ms: row.get::<_, i64>(1)? as u64,
        severity: EventSeverity::parse(&severity).map_err(|error| rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string()))))?,
        category: row.get(3)?, kind: row.get(4)?, message: row.get(5)?,
        metadata: metadata.map(|value| serde_json::from_str(&value).map_err(|error| rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, Box::new(error)))).transpose()?,
        plugin_id: row.get(7)?, lease_id: row.get(8)?, job_id: row.get(9)?, request_id: row.get(10)?, source: row.get(11)?,
        actor: row.get(12)?,
        client_ip: row.get(13)?,
        peer_ip: row.get(14)?,
        http_method: row.get(15)?,
        http_path: row.get(16)?,
        http_status: row.get::<_, Option<i64>>(17)?.and_then(|value| value.try_into().ok()),
        duration_us: row.get::<_, Option<i64>>(18)?.map(|value| value.max(0) as u64),
        request_bytes: row.get::<_, Option<i64>>(19)?.map(|value| value.max(0) as u64),
        response_bytes: row.get::<_, Option<i64>>(20)?.map(|value| value.max(0) as u64),
    })
}

#[derive(Deserialize)]
struct LegacyEvent {
    id: u64,
    timestamp_unix_ms: u64,
    severity: EventSeverity,
    category: String,
    kind: String,
    message: String,
    #[serde(default)]
    detail: Option<String>,
    #[serde(default)]
    metadata: Option<Value>,
    #[serde(default)]
    plugin_id: Option<String>,
    #[serde(default)]
    lease_id: Option<String>,
    #[serde(default)]
    job_id: Option<String>,
    #[serde(default)]
    request_id: Option<String>,
    #[serde(default)]
    source: Option<String>,
}

fn migrate_event_journals(connection: &Connection, state_root: &std::path::Path) -> Result<()> {
    for name in ["events.jsonl.1", "events.jsonl"] {
        let source = state_root.join(name);
        if !source.is_file() { continue; }
        let raw = fs::read_to_string(&source).with_context(|| format!("read legacy event journal {}", source.display()))?;
        let transaction = connection.unchecked_transaction().context("begin legacy event migration")?;
        for line in raw.lines().filter(|line| !line.trim().is_empty()) {
            let event: LegacyEvent = serde_json::from_str(line).with_context(|| format!("parse legacy event in {}", source.display()))?;
            let metadata = event.metadata.or_else(|| event.detail.map(|detail| serde_json::json!({"detail": detail})));
            transaction.execute(
                "INSERT OR IGNORE INTO events (id, timestamp_unix_ms, severity, category, kind, message, metadata, plugin_id, lease_id, job_id, request_id, source) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![event.id as i64, event.timestamp_unix_ms as i64, event.severity.as_str(), event.category, event.kind, event.message, metadata.map(|value| value.to_string()), event.plugin_id, event.lease_id, event.job_id, event.request_id, event.source],
            ).context("import legacy event")?;
        }
        transaction.commit().context("commit legacy event migration")?;
        let migrated = PathBuf::from(format!("{}.migrated", source.display()));
        fs::rename(&source, &migrated).with_context(|| format!("mark legacy event journal migrated {}", source.display()))?;
    }
    Ok(())
}

impl EventRetention {
    fn from_env() -> Result<Self> {
        Ok(Self {
            days: parse_retention_value(
                "RKSERVE_EVENT_RETENTION_DAYS",
                DEFAULT_RETENTION_DAYS,
                1,
                3_650,
            )?,
            max_rows: parse_retention_value(
                "RKSERVE_EVENT_MAX_ROWS",
                DEFAULT_MAX_ROWS,
                1_000,
                10_000_000,
            )?,
        })
    }
}

fn parse_retention_value(name: &str, default: u64, minimum: u64, maximum: u64) -> Result<u64> {
    let Some(raw) = std::env::var_os(name) else {
        return Ok(default);
    };
    let value = raw
        .to_string_lossy()
        .parse::<u64>()
        .with_context(|| format!("parse {name}"))?;
    anyhow::ensure!(
        (minimum..=maximum).contains(&value),
        "{name} must be between {minimum} and {maximum}"
    );
    Ok(value)
}

fn prune_events(connection: &Connection, retention: &EventRetention) -> Result<()> {
    let cutoff = now_ms().saturating_sub(retention.days.saturating_mul(86_400_000));
    connection
        .execute(
            "DELETE FROM events WHERE timestamp_unix_ms < ?1",
            [cutoff as i64],
        )
        .context("prune expired events")?;
    connection
        .execute(
            "DELETE FROM events WHERE id IN (
                SELECT id FROM events ORDER BY id DESC LIMIT -1 OFFSET ?1
            )",
            [retention.max_rows as i64],
        )
        .context("prune excess events")?;
    Ok(())
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis().try_into().unwrap_or(u64::MAX)
}

fn like_contains(raw: &str) -> String {
    let mut pattern = String::from("%");
    pattern.push_str(&like_escaped(raw));
    pattern.push('%');
    pattern
}

fn like_prefix(raw: &str) -> String {
    let mut pattern = like_escaped(raw);
    pattern.push('%');
    pattern
}

fn like_escaped(raw: &str) -> String {
    let mut escaped = String::new();
    for character in raw.chars() {
        if matches!(character, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::{like_contains, like_prefix};

    #[test]
    fn like_contains_escapes_wildcards() {
        assert_eq!(like_contains("a%b_c\\d"), "%a\\%b\\_c\\\\d%");
    }

    #[test]
    fn like_prefix_anchors_start() {
        assert_eq!(like_prefix("/api/v1/plugins"), "/api/v1/plugins%");
        assert_eq!(like_prefix("a%b"), "a\\%b%");
    }
}
