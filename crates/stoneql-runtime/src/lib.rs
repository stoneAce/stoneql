//! Host-independent connection lifecycle. No renderer/HTTP access or credential persistence.
mod execution;
mod files;
mod mysql;
mod postgres;
mod read_only;
mod script;
#[cfg(feature = "sqlite")]
mod sqlite;
mod ssh;
mod transfer;
pub use execution::{
    Access, EndReason, ExecutionError, ExecutionLimits, ImportError, ImportRefusal,
    ImportRefusalReason, JobId, JobOptions, LeaseId, OutcomeId, TransactionEnd,
    mysql_state_matches, postgres_commit, postgres_state_matches,
};
pub use read_only::{QueryLimits, ReadError};
pub use script::ScriptError;
#[cfg(feature = "sqlite")]
use sqlx::SqlitePool;
use sqlx::{MySqlPool, PgPool};
pub use ssh::{ENDPOINT_PREFIX, HostKey, SshAuth, SshConfig, describe_host_key, endpoint_root};
use std::{
    collections::HashMap,
    fmt,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use stoneql_core::Engine;
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, oneshot},
    time::Instant,
};
pub use transfer::{
    ExportFormat, ExportOptions, ExportProgress, ExportRequest, ExportSummary, InsertVerb,
    MAX_EXPORT_TABLES, SQL_BATCH_ROWS, TransferError, quote_identifier, string_literal,
};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct OwnerId(Uuid);
impl OwnerId {
    /// Created by a trusted host session, never supplied by an untrusted request.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}
impl Default for OwnerId {
    fn default() -> Self {
        Self::new()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ConnectionId(Uuid);
impl ConnectionId {
    pub fn parse(value: &str) -> Result<Self, LifecycleError> {
        Uuid::parse_str(value)
            .map(Self)
            .map_err(|_| LifecycleError::Forbidden)
    }
    pub fn opaque(self) -> String {
        self.0.to_string()
    }
}

/// Network engine profile (MySQL, MariaDB or PostgreSQL). Never derives Debug/Serialize:
/// credentials and certificate material stay in memory.
pub struct DirectServer {
    pub engine: Engine,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub database: String,
    pub root_certificate_pem: Option<Vec<u8>>,
    /// Verified TLS (certificate chain and host name). `false` is an explicit choice for a direct
    /// MySQL/MariaDB connection, without a CA, and then no TLS is attempted at all. A TLS failure
    /// never falls back to plaintext and plaintext is never upgraded.
    pub tls: bool,
    /// TLS off only: how the password is protected when the server asks for a full sign-in.
    pub rsa_key: PasswordRsaKey,
}
/// RSA protection of a MySQL/MariaDB password sent without TLS, when the server asks for a full
/// sign-in (`caching_sha2_password` with an empty authentication cache, `sha256_password`). It
/// protects only the password exchange; queries and results are not encrypted.
#[derive(Clone, PartialEq, Eq)]
pub enum PasswordRsaKey {
    /// No RSA exchange: such a sign-in fails with [`LifecycleError::EncryptedLoginRequired`].
    None,
    /// A trusted RSA public key (PEM) supplied by the user; it is never requested from the server.
    Trusted(Vec<u8>),
    /// Explicit opt-in: the key is requested from the server unverified, so a server
    /// impersonating the real one can supply its own key and read the password.
    RetrieveUnverified,
}
impl PasswordRsaKey {
    /// A PEM `PUBLIC KEY` that parses as an RSA key, at most 16 KiB, without private material.
    /// Trailing NUL bytes and whitespace are ignored, as the driver does (MySQL's own
    /// `public_key.pem` ends with a NUL byte).
    pub fn is_valid_public_key_pem(pem: &[u8]) -> bool {
        use rsa::pkcs8::DecodePublicKey;
        pem.len() <= 16_384
            && std::str::from_utf8(pem).is_ok_and(|pem| {
                let pem = pem.trim_end_matches(|c: char| c == '\0' || c.is_ascii_whitespace());
                !pem.contains("PRIVATE KEY") && rsa::PublicKey::from_public_key_pem(pem).is_ok()
            })
    }
}
/// Loopback host names (this computer).
pub fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "::1"
}
impl DirectServer {
    fn validate(&self) -> Result<(), LifecycleError> {
        if !self.tls
            && (!matches!(self.engine, Engine::MySql | Engine::MariaDb)
                || self.root_certificate_pem.is_some())
        {
            return Err(LifecycleError::InvalidConfiguration);
        }
        // RSA options only apply without TLS, and a trusted key must be a valid RSA public key.
        match &self.rsa_key {
            PasswordRsaKey::None => {}
            _ if self.tls => return Err(LifecycleError::InvalidConfiguration),
            PasswordRsaKey::Trusted(pem) if !PasswordRsaKey::is_valid_public_key_pem(pem) => {
                return Err(LifecycleError::InvalidConfiguration);
            }
            PasswordRsaKey::Trusted(_) | PasswordRsaKey::RetrieveUnverified => {}
        }
        if !matches!(
            self.engine,
            Engine::MySql | Engine::MariaDb | Engine::PostgreSql
        ) || self.host.is_empty()
            || self.host.len() > 253
            || self
                .host
                .chars()
                .any(|character| character.is_whitespace() || character == '\0')
            || self.port == 0
            || self.username.contains('\0')
            || self.database.contains('\0')
            || self.password.contains('\0')
            || self.username.is_empty()
            || self.username.len() > 256
            || self.database.is_empty()
            || self.database.len() > 256
            || self.password.len() > 16_384
            || self
                .root_certificate_pem
                .as_ref()
                .is_some_and(|pem| pem.is_empty() || pem.len() > 1_048_576)
        {
            return Err(LifecycleError::InvalidConfiguration);
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifecycleError {
    InvalidConfiguration,
    Capacity,
    Forbidden,
    Unavailable,
    Timeout,
    EngineMismatch,
    /// No enrolled SSH host key, or the server offered an unsupported host certificate.
    HostKeyUnknown,
    /// The SSH server presented a key different from the enrolled one. Never auto-accepted.
    HostKeyChanged,
    SshAuthentication,
    AgentUnavailable,
    /// Recognized but unsupported on this host/build (for example RSA SSH keys).
    Unsupported,
    Cancelled,
    /// TLS off: the server asked for a full sign-in (`caching_sha2_password` with an empty
    /// authentication cache, `sha256_password`), which needs the server's RSA public key, and no
    /// trusted key is set and fetching it was not allowed. Nothing was sent.
    EncryptedLoginRequired,
    /// TLS off: the server accepts only secure transport (MySQL 3159). Never retried with TLS.
    SecureTransportRequired,
    /// The handle is running a job or holds a transaction; nothing was changed.
    Busy,
    /// The server denied this account access to the requested database (MySQL 1044).
    DatabaseAccessDenied,
    /// The requested database does not exist (MySQL 1049).
    UnknownDatabase,
}
impl fmt::Display for LifecycleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidConfiguration => "Invalid connection configuration.",
            Self::Capacity => "Connection capacity reached.",
            Self::Forbidden => "Connection is not available to this session.",
            Self::Unavailable => "Connection failed. Check authentication, TLS and availability.",
            Self::Timeout => "Connection deadline exceeded.",
            Self::EngineMismatch => "The connected database engine does not match the profile.",
            Self::HostKeyUnknown => "The SSH host key is not enrolled.",
            Self::HostKeyChanged => "The SSH host key changed; connection refused.",
            Self::SshAuthentication => "SSH authentication failed.",
            Self::AgentUnavailable => "The SSH agent is unavailable.",
            Self::Unsupported => "This transport option is not supported.",
            Self::Cancelled => "Connection setup was cancelled.",
            Self::EncryptedLoginRequired => {
                "The server asked for a full sign-in, which without TLS needs its RSA public key."
            }
            Self::SecureTransportRequired => "The server accepts only TLS connections.",
            Self::Busy => "The connection is busy; finish the running work or transaction first.",
            Self::DatabaseAccessDenied => "The server denied this account access to that database.",
            Self::UnknownDatabase => "That database does not exist on the server.",
        })
    }
}
impl std::error::Error for LifecycleError {}
fn connect_error(error: sqlx::Error) -> LifecycleError {
    match &error {
        sqlx::Error::PoolTimedOut => LifecycleError::Timeout,
        // The vendored driver's fixed text when a full sign-in without TLS needs the server's RSA
        // public key, no trusted key is set and retrieval is not allowed (nothing was sent).
        sqlx::Error::Configuration(message)
            if message
                .to_string()
                .starts_with("RSA public key retrieval is not allowed") =>
        {
            LifecycleError::EncryptedLoginRequired
        }
        // ER_SECURE_TRANSPORT_REQUIRED: the server refuses insecure transport.
        _ if error
            .as_database_error()
            .and_then(|error| error.try_downcast_ref::<sqlx::mysql::MySqlDatabaseError>())
            .is_some_and(|error| error.number() == 3159) =>
        {
            LifecycleError::SecureTransportRequired
        }
        _ => LifecycleError::Unavailable,
    }
}
/// One bounded single-connection pool per logical handle. Engine-specific work stays in the
/// engine modules; the registry only owns lifetime, ownership and cleanup.
#[derive(Clone)]
enum EnginePool {
    MySql(MySqlPool),
    Postgres(PgPool),
    #[cfg(feature = "sqlite")]
    Sqlite(SqlitePool),
}
impl EnginePool {
    async fn close(&self) {
        match self {
            Self::MySql(pool) => pool.close().await,
            Self::Postgres(pool) => pool.close().await,
            #[cfg(feature = "sqlite")]
            Self::Sqlite(pool) => pool.close().await,
        }
    }
    fn is_closed(&self) -> bool {
        match self {
            Self::MySql(pool) => pool.is_closed(),
            Self::Postgres(pool) => pool.is_closed(),
            #[cfg(feature = "sqlite")]
            Self::Sqlite(pool) => pool.is_closed(),
        }
    }
    fn dialect(&self) -> read_only::Dialect {
        match self {
            Self::MySql(_) => read_only::Dialect::MySql,
            Self::Postgres(_) => read_only::Dialect::Postgres,
            #[cfg(feature = "sqlite")]
            Self::Sqlite(_) => read_only::Dialect::Sqlite,
        }
    }
}
/// What a registered handle reports about itself.
struct Details {
    engine: Engine,
    engine_version: String,
    access: Access,
}
struct Entry {
    engine: Engine,
    engine_version: String,
    owner: OwnerId,
    pool: EnginePool,
    /// Read-write is an explicit connect-time choice; read-only handles refuse jobs/leases.
    access: Access,
    /// Exclusive work permit (one job or one transaction) for read-write handles; the
    /// connections it guards are dedicated and never pooled.
    work: Arc<Semaphore>,
    /// SSH transport owned by this handle; closed after its pool, never shared.
    tunnel: Option<ssh::Tunnel>,
    /// Last use by a query, job, transaction or transfer (status checks do not count). The
    /// handle, which keeps the sign-in details in its connect options, ends after `idle` unused.
    last_used: Arc<Mutex<Instant>>,
    idle: Duration,
    _permit: OwnedSemaphorePermit,
    _expiry_cancel: oneshot::Sender<()>,
}
impl Entry {
    fn usable(&self, owner: OwnerId) -> bool {
        self.owner == owner
            && self.last_used.lock().expect("handle use lock").elapsed() < self.idle
            && !self.pool.is_closed()
            && self
                .tunnel
                .as_ref()
                .is_none_or(|tunnel| !tunnel.is_closed())
    }
    fn touch(&self) {
        *self.last_used.lock().expect("handle use lock") = Instant::now();
    }
    /// The single close path for disconnect, expiry, reset and shutdown.
    async fn close(self) {
        self.pool.close().await;
        if let Some(tunnel) = self.tunnel {
            tunnel.close().await;
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum GrantKind {
    #[cfg(feature = "sqlite")]
    Sqlite,
    SshKey,
}
struct Pending {
    owner: OwnerId,
    attempt: Option<String>,
    cancel: oneshot::Sender<()>,
}
struct Inner {
    generation: AtomicU64,
    entries: Mutex<HashMap<ConnectionId, Entry>>,
    grants: Mutex<HashMap<Uuid, (OwnerId, GrantKind, PathBuf)>>,
    pending: Mutex<HashMap<u64, Pending>>,
    next_attempt: AtomicU64,
    slots: Arc<Semaphore>,
    deadline: Duration,
    /// A handle ends after this long without use. Its pool keeps no idle database connection
    /// ([`POOL_IDLE_TIMEOUT`]): each use opens one again with the kept sign-in details.
    idle_lifetime: Duration,
    execution: execution::ExecutionState,
}
/// Removes a pending setup on every exit path, including a dropped (cancelled) future.
struct AttemptGuard(Arc<Inner>, u64);
impl Drop for AttemptGuard {
    fn drop(&mut self) {
        self.0.pending.lock().expect("pending lock").remove(&self.1);
    }
}
/// A registered, cancellable connection setup (F-02). A host registers it before work that may
/// block (for example resolving saved secrets), so [`Connections::cancel_connect`] and a reset
/// reach that work too. Dropping it unregisters the setup.
pub struct Attempt {
    _guard: AttemptGuard,
    cancelled: oneshot::Receiver<()>,
}
impl Attempt {
    /// Resolves when the setup is cancelled or the registry is reset. Cancel-safe: may be raced
    /// repeatedly against other work.
    pub async fn cancelled(&mut self) {
        let _ = (&mut self.cancelled).await;
    }
}
/// An idle pooled database connection is closed after this long; the next use opens a new one
/// with the handle's sign-in details, so a server restart or a dropped connection is recovered
/// without the renderer. Dedicated job and transaction connections are not pooled.
pub(crate) const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// Clone shares one bounded registry; every handle is scoped to its trusted owner.
#[derive(Clone)]
pub struct Connections(Arc<Inner>);
impl Connections {
    /// Outstanding SQLite file grants per registry; reset/shutdown revoke them all.
    pub const MAX_FILE_GRANTS: usize = 32;
    pub fn new(
        capacity: usize,
        deadline: Duration,
        idle_lifetime: Duration,
    ) -> Result<Self, LifecycleError> {
        if !(1..=64).contains(&capacity)
            || deadline.is_zero()
            || deadline > Duration::from_secs(30)
            || idle_lifetime.is_zero()
            || idle_lifetime > Duration::from_secs(86_400)
        {
            return Err(LifecycleError::InvalidConfiguration);
        }
        Ok(Self(Arc::new(Inner {
            generation: AtomicU64::new(0),
            entries: Mutex::new(HashMap::new()),
            grants: Mutex::new(HashMap::new()),
            pending: Mutex::new(HashMap::new()),
            next_attempt: AtomicU64::new(0),
            slots: Arc::new(Semaphore::new(capacity)),
            deadline,
            idle_lifetime,
            execution: execution::ExecutionState::new(),
        })))
    }
    /// Opens a verified-TLS MySQL, MariaDB or PostgreSQL handle.
    pub async fn connect(
        &self,
        owner: OwnerId,
        config: DirectServer,
    ) -> Result<ConnectionId, LifecycleError> {
        self.connect_with(owner, config, None, None).await
    }
    /// Opens a handle directly or through an owned SSH tunnel. `attempt` names this setup for
    /// [`Connections::cancel_connect`]. Cancellation, deadline, failure, reset and shutdown all
    /// drop partial transport state; only a registered handle owns a tunnel.
    pub async fn connect_with(
        &self,
        owner: OwnerId,
        config: DirectServer,
        tunnel: Option<SshConfig>,
        attempt: Option<&str>,
    ) -> Result<ConnectionId, LifecycleError> {
        self.connect_with_access(owner, config, tunnel, attempt, Access::ReadOnly)
            .await
    }
    /// As [`Connections::connect_with`], with an explicit access mode. Database privileges
    /// remain authoritative; read-write only enables the job and transaction APIs.
    pub async fn connect_with_access(
        &self,
        owner: OwnerId,
        config: DirectServer,
        tunnel: Option<SshConfig>,
        attempt: Option<&str>,
        access: Access,
    ) -> Result<ConnectionId, LifecycleError> {
        let attempt = self.begin_attempt(owner, attempt)?;
        self.connect_attempt(owner, config, tunnel, attempt, access)
            .await
    }
    /// As [`Connections::connect_with_access`], inside an attempt the caller registered first
    /// with [`Connections::begin_attempt`]. The capacity permit is taken here, after any work
    /// the caller did inside the attempt, so a slow secret prompt never holds a slot.
    pub async fn connect_attempt(
        &self,
        owner: OwnerId,
        config: DirectServer,
        tunnel: Option<SshConfig>,
        mut attempt: Attempt,
        access: Access,
    ) -> Result<ConnectionId, LifecycleError> {
        config.validate()?;
        let engine = config.engine;
        // Gate (D025-9): read-write over SSH is enabled for MySQL/MariaDB (A15, approved
        // 2026-11-17; tunnelled cancellation under forwarding-slot exhaustion is fixture-tested
        // in `execution::ssh_fixture`). PostgreSQL over SSH stays read-only. Refused before any
        // network access.
        if tunnel.is_some()
            && access == Access::ReadWrite
            && !matches!(engine, Engine::MySql | Engine::MariaDb)
        {
            return Err(LifecycleError::Unsupported);
        }
        if let Some(tunnel) = &tunnel {
            tunnel.validate()?;
        }
        let key_text = match &tunnel {
            Some(SshConfig {
                auth: SshAuth::KeyFile { grant, .. },
                ..
            }) => Some(self.read_key_file(owner, grant)?),
            _ => None,
        };
        let generation = self.0.generation.load(Ordering::SeqCst);
        let permit = self.permit()?;
        let deadline = self.0.deadline;
        let setup = async {
            let tunnel = match tunnel {
                Some(ssh_config) => {
                    let socket = match config.engine {
                        Engine::PostgreSql => format!(".s.PGSQL.{}", config.port),
                        _ => "mysql.sock".to_owned(),
                    };
                    Some(
                        tokio::time::timeout(
                            deadline,
                            ssh::open(ssh_config, key_text, &config.host, config.port, &socket),
                        )
                        .await
                        .map_err(|_| LifecycleError::Timeout)??,
                    )
                }
                None => None,
            };
            let endpoint = tunnel.as_ref().map(ssh::Tunnel::endpoint);
            let (pool, version) = match config.engine {
                Engine::PostgreSql => postgres::open(&config, endpoint, deadline)
                    .await
                    .map(|(pool, version)| (EnginePool::Postgres(pool), version))?,
                _ => mysql::open(&config, endpoint, deadline)
                    .await
                    .map(|(pool, version)| (EnginePool::MySql(pool), version))?,
            };
            Ok::<_, LifecycleError>((pool, version, tunnel))
        };
        let (pool, version, tunnel) = tokio::select! {
            result = tokio::time::timeout(deadline * 2, setup) => {
                result.map_err(|_| LifecycleError::Timeout)??
            }
            _ = attempt.cancelled() => return Err(LifecycleError::Cancelled),
        };
        self.register_or_close(
            owner,
            pool,
            tunnel,
            permit,
            Details {
                engine,
                engine_version: version,
                access,
            },
            generation,
        )
        .await
    }
    /// Registers an owned setup; a duplicate attempt name for the same owner is refused.
    pub fn begin_attempt(
        &self,
        owner: OwnerId,
        attempt: Option<&str>,
    ) -> Result<Attempt, LifecycleError> {
        let (cancel, cancelled) = oneshot::channel();
        let mut pending = self.0.pending.lock().expect("pending lock");
        if attempt.is_some_and(|attempt| {
            pending
                .values()
                .any(|entry| entry.owner == owner && entry.attempt.as_deref() == Some(attempt))
        }) {
            return Err(LifecycleError::InvalidConfiguration);
        }
        let id = self.0.next_attempt.fetch_add(1, Ordering::SeqCst);
        pending.insert(
            id,
            Pending {
                owner,
                attempt: attempt.map(str::to_owned),
                cancel,
            },
        );
        Ok(Attempt {
            _guard: AttemptGuard(self.0.clone(), id),
            cancelled,
        })
    }
    /// Cancels an owned in-flight setup. Returns false if it already finished or is unknown.
    pub fn cancel_connect(&self, owner: OwnerId, attempt: &str) -> bool {
        let mut pending = self.0.pending.lock().expect("pending lock");
        let Some(id) = pending
            .iter()
            .find(|(_, entry)| entry.owner == owner && entry.attempt.as_deref() == Some(attempt))
            .map(|(id, _)| *id)
        else {
            return false;
        };
        let entry = pending.remove(&id).expect("found attempt");
        entry.cancel.send(()).is_ok()
    }
    fn cancel_all_attempts(&self) {
        for (_, entry) in self.0.pending.lock().expect("pending lock").drain() {
            let _ = entry.cancel.send(());
        }
    }
    /// Reads the server's SSH host key for explicit enrollment; never trusts it.
    pub async fn scan_ssh_host_key(
        &self,
        host: &str,
        port: u16,
    ) -> Result<HostKey, LifecycleError> {
        if self.0.slots.is_closed() {
            return Err(LifecycleError::Capacity);
        }
        ssh::scan(host, port, self.0.deadline).await
    }
    /// Trusted host file-picker result → owner-scoped SSH private-key grant. The key is read
    /// only at connect time and never returned.
    pub fn grant_ssh_key_file(
        &self,
        owner: OwnerId,
        path: &Path,
    ) -> Result<String, LifecycleError> {
        let canonical = files::authorize(path)?;
        self.grant(owner, GrantKind::SshKey, canonical)
    }
    pub fn ssh_key_file_name(&self, owner: OwnerId, grant: &str) -> Result<String, LifecycleError> {
        file_name(&self.granted_path(owner, grant, GrantKind::SshKey)?)
    }
    fn read_key_file(&self, owner: OwnerId, grant: &str) -> Result<String, LifecycleError> {
        let path = self.granted_path(owner, grant, GrantKind::SshKey)?;
        let canonical = files::authorize(&path).map_err(|_| LifecycleError::Forbidden)?;
        let metadata = std::fs::metadata(&canonical).map_err(|_| LifecycleError::Forbidden)?;
        if metadata.len() > ssh::MAX_KEY_FILE_BYTES {
            return Err(LifecycleError::InvalidConfiguration);
        }
        let bytes = std::fs::read(&canonical).map_err(|_| LifecycleError::Forbidden)?;
        String::from_utf8(bytes).map_err(|_| LifecycleError::InvalidConfiguration)
    }
    #[cfg(feature = "sqlite")]
    /// Trusted host file-picker result → owner-scoped opaque grant. The renderer never supplies
    /// a path; it can only name a grant previously issued to its own session.
    pub fn grant_sqlite_file(&self, owner: OwnerId, path: &Path) -> Result<String, LifecycleError> {
        let canonical = files::authorize(path)?;
        self.grant(owner, GrantKind::Sqlite, canonical)
    }
    /// Grants are typed: a SQLite grant never authorizes reading a file as an SSH key.
    fn grant(
        &self,
        owner: OwnerId,
        kind: GrantKind,
        canonical: PathBuf,
    ) -> Result<String, LifecycleError> {
        let mut grants = self.0.grants.lock().expect("file grant lock");
        if let Some((id, _)) = grants
            .iter()
            .find(|(_, (grant_owner, grant_kind, granted))| {
                *grant_owner == owner && *grant_kind == kind && *granted == canonical
            })
        {
            return Ok(id.to_string());
        }
        if grants.len() >= Self::MAX_FILE_GRANTS {
            return Err(LifecycleError::Capacity);
        }
        let id = Uuid::new_v4();
        grants.insert(id, (owner, kind, canonical));
        Ok(id.to_string())
    }
    #[cfg(feature = "sqlite")]
    /// File name of an owned grant, for display only.
    pub fn sqlite_file_name(&self, owner: OwnerId, grant: &str) -> Result<String, LifecycleError> {
        file_name(&self.granted_path(owner, grant, GrantKind::Sqlite)?)
    }
    fn granted_path(
        &self,
        owner: OwnerId,
        grant: &str,
        kind: GrantKind,
    ) -> Result<PathBuf, LifecycleError> {
        let id = Uuid::parse_str(grant).map_err(|_| LifecycleError::Forbidden)?;
        self.0
            .grants
            .lock()
            .expect("file grant lock")
            .get(&id)
            .filter(|(grant_owner, grant_kind, _)| *grant_owner == owner && *grant_kind == kind)
            .map(|(_, _, path)| path.clone())
            .ok_or(LifecycleError::Forbidden)
    }
    #[cfg(feature = "sqlite")]
    /// Opens an owned SQLite grant read-only on SQLx's dedicated worker thread.
    pub async fn connect_sqlite(
        &self,
        owner: OwnerId,
        grant: &str,
    ) -> Result<ConnectionId, LifecycleError> {
        self.connect_sqlite_with_access(owner, grant, Access::ReadOnly)
            .await
    }
    #[cfg(feature = "sqlite")]
    /// Read-write SQLite handles keep the read-only pool for `read_only`; jobs and leases open
    /// separate read-write connections to the same re-verified canonical file.
    pub async fn connect_sqlite_with_access(
        &self,
        owner: OwnerId,
        grant: &str,
        access: Access,
    ) -> Result<ConnectionId, LifecycleError> {
        let path = self.granted_path(owner, grant, GrantKind::Sqlite)?;
        let generation = self.0.generation.load(Ordering::SeqCst);
        let permit = self.permit()?;
        let (pool, version) = sqlite::open(&path, self.0.deadline).await?;
        let engine = Engine::Sqlite;
        self.register_or_close(
            owner,
            EnginePool::Sqlite(pool),
            None,
            permit,
            Details {
                engine,
                engine_version: version,
                access,
            },
            generation,
        )
        .await
    }
    fn permit(&self) -> Result<OwnedSemaphorePermit, LifecycleError> {
        self.0
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| LifecycleError::Capacity)
    }
    async fn register_or_close(
        &self,
        owner: OwnerId,
        pool: EnginePool,
        tunnel: Option<ssh::Tunnel>,
        permit: OwnedSemaphorePermit,
        details: Details,
        generation: u64,
    ) -> Result<ConnectionId, LifecycleError> {
        let mut tunnel = tunnel;
        match self.register(
            owner,
            pool.clone(),
            &mut tunnel,
            permit,
            details,
            generation,
        ) {
            Ok(id) => Ok(id),
            Err(error) => {
                pool.close().await;
                if let Some(tunnel) = tunnel {
                    tunnel.close().await;
                }
                Err(error)
            }
        }
    }
    /// On success the tunnel moves into the registry entry; on refusal it stays with the caller.
    fn register(
        &self,
        owner: OwnerId,
        pool: EnginePool,
        tunnel: &mut Option<ssh::Tunnel>,
        permit: OwnedSemaphorePermit,
        details: Details,
        generation: u64,
    ) -> Result<ConnectionId, LifecycleError> {
        let Details {
            engine,
            engine_version,
            access,
        } = details;
        let id = ConnectionId(Uuid::new_v4());
        let (cancel, cancelled) = oneshot::channel();
        let (idle, last_used) = (self.0.idle_lifetime, Arc::new(Mutex::new(Instant::now())));
        let watched = last_used.clone();
        // F-03: a dead SSH session releases the handle through the same close path as expiry.
        let tunnel_closed = tunnel.as_ref().map(ssh::Tunnel::closed);
        {
            let mut entries = self.0.entries.lock().expect("connection registry lock");
            if self.0.slots.is_closed() {
                return Err(LifecycleError::Capacity);
            }
            if self.0.generation.load(Ordering::SeqCst) != generation {
                return Err(LifecycleError::Forbidden);
            }
            entries.insert(
                id,
                Entry {
                    engine,
                    engine_version,
                    owner,
                    pool,
                    access,
                    work: Arc::new(Semaphore::new(ExecutionLimits::RUNNING_JOBS_PER_HANDLE)),
                    tunnel: tunnel.take(),
                    last_used,
                    idle,
                    _permit: permit,
                    _expiry_cancel: cancel,
                },
            );
        }
        let registry = Arc::downgrade(&self.0);
        tokio::spawn(async move {
            let tunnel_closed = async {
                match tunnel_closed {
                    Some(closed) => closed.await,
                    None => std::future::pending().await,
                }
            };
            tokio::pin!(tunnel_closed);
            let mut cancelled = cancelled;
            // Idle expiry: each wake re-reads the last use and sleeps again until it is old enough.
            loop {
                let idle_end = *watched.lock().expect("handle use lock") + idle;
                tokio::select! {
                    _ = tokio::time::sleep_until(idle_end) => {
                        if watched.lock().expect("handle use lock").elapsed() >= idle {
                            break;
                        }
                    },
                    () = &mut tunnel_closed => break,
                    _ = &mut cancelled => return,
                }
            }
            let Some(inner) = registry.upgrade() else {
                return;
            };
            // Same order as disconnect/reset/shutdown: the handle becomes unusable first, so no
            // job or transaction can start on it while its work is released.
            let entry = inner
                .entries
                .lock()
                .expect("connection registry lock")
                .remove(&id);
            if let Some(entry) = entry {
                execution::release(&inner, Some(id)).await;
                entry.close().await;
            }
        });
        Ok(id)
    }
    fn owned_pool(&self, owner: OwnerId, id: ConnectionId) -> Option<EnginePool> {
        self.0
            .entries
            .lock()
            .expect("connection registry lock")
            .get(&id)
            .filter(|entry| entry.usable(owner))
            .map(|entry| {
                entry.touch();
                entry.pool.clone()
            })
    }
    pub fn engine_version(
        &self,
        owner: OwnerId,
        id: ConnectionId,
    ) -> Result<String, LifecycleError> {
        self.0
            .entries
            .lock()
            .expect("connection registry lock")
            .get(&id)
            .filter(|entry| entry.usable(owner))
            .map(|entry| entry.engine_version.clone())
            .ok_or(LifecycleError::Forbidden)
    }
    pub fn engine(&self, owner: OwnerId, id: ConnectionId) -> Result<Engine, LifecycleError> {
        self.0
            .entries
            .lock()
            .expect("connection registry lock")
            .get(&id)
            .filter(|entry| entry.usable(owner))
            .map(|entry| entry.engine)
            .ok_or(LifecycleError::Forbidden)
    }
    pub fn access(&self, owner: OwnerId, id: ConnectionId) -> Result<Access, LifecycleError> {
        self.0
            .entries
            .lock()
            .expect("connection registry lock")
            .get(&id)
            .filter(|entry| entry.usable(owner))
            .map(|entry| entry.access)
            .ok_or(LifecycleError::Forbidden)
    }
    pub fn is_connected(&self, owner: OwnerId, id: ConnectionId) -> bool {
        // A status check is not a use: it never keeps an idle handle alive.
        self.0
            .entries
            .lock()
            .expect("connection registry lock")
            .get(&id)
            .is_some_and(|entry| entry.usable(owner))
    }
    /// Reopens a MySQL/MariaDB handle's session on another database, keeping the handle, its
    /// access, credentials and SSH tunnel. Refused while the handle runs a job or holds a
    /// transaction. The new session is opened and verified before it replaces the old one, so
    /// a denied or failed switch leaves the handle exactly as it was. Returns the database the
    /// server reports as current.
    pub async fn switch_database(
        &self,
        owner: OwnerId,
        id: ConnectionId,
        database: &str,
    ) -> Result<String, LifecycleError> {
        if database.is_empty() || database.len() > 64 || database.chars().any(char::is_control) {
            return Err(LifecycleError::InvalidConfiguration);
        }
        let (pool, work, access) = {
            let entries = self.0.entries.lock().expect("connection registry lock");
            let entry = entries
                .get(&id)
                .filter(|entry| entry.usable(owner))
                .ok_or(LifecycleError::Forbidden)?;
            let EnginePool::MySql(pool) = &entry.pool else {
                return Err(LifecycleError::Unsupported);
            };
            (pool.clone(), entry.work.clone(), entry.access)
        };
        // Jobs and transactions hold the work permit for their whole run.
        let _permit = match access {
            Access::ReadWrite => Some(work.try_acquire_owned().map_err(|_| LifecycleError::Busy)?),
            Access::ReadOnly => None,
        };
        let options = (*pool.connect_options()).clone().database(database);
        let deadline = self.0.deadline;
        let opened = tokio::time::timeout(
            deadline,
            sqlx::mysql::MySqlPoolOptions::new()
                .max_connections(1)
                .min_connections(0)
                .acquire_timeout(deadline)
                .idle_timeout(POOL_IDLE_TIMEOUT)
                .connect_with(options),
        )
        .await
        .map_err(|_| LifecycleError::Timeout)?
        .map_err(|error| {
            match error
                .as_database_error()
                .and_then(|error| error.try_downcast_ref::<sqlx::mysql::MySqlDatabaseError>())
                .map(|error| error.number())
            {
                Some(1044) => LifecycleError::DatabaseAccessDenied,
                Some(1049) => LifecycleError::UnknownDatabase,
                _ => connect_error(error),
            }
        })?;
        let current = tokio::time::timeout(
            deadline,
            sqlx::query_scalar::<_, Option<String>>("SELECT DATABASE()").fetch_one(&opened),
        )
        .await;
        let current = match current {
            Ok(Ok(Some(current))) if current == database => current,
            _ => {
                opened.close().await;
                return Err(LifecycleError::Unavailable);
            }
        };
        let swapped = {
            let mut entries = self.0.entries.lock().expect("connection registry lock");
            match entries.get_mut(&id).filter(|entry| entry.usable(owner)) {
                Some(entry) => Ok(std::mem::replace(
                    &mut entry.pool,
                    EnginePool::MySql(opened.clone()),
                )),
                None => Err(()),
            }
        };
        let Ok(old) = swapped else {
            opened.close().await;
            return Err(LifecycleError::Forbidden);
        };
        // In-flight reads finish on the old session; it closes once they return.
        tokio::spawn(async move { old.close().await });
        Ok(current)
    }
    pub async fn disconnect(&self, owner: OwnerId, id: ConnectionId) -> Result<(), LifecycleError> {
        // The handle is removed first, so a racing submit/begin sees it gone and refuses;
        // then jobs are cancelled and leases rolled back before the pool and tunnel close.
        let entry = {
            let mut entries = self.0.entries.lock().expect("connection registry lock");
            if !entries.get(&id).is_some_and(|entry| entry.owner == owner) {
                return Err(LifecycleError::Forbidden);
            }
            entries.remove(&id).expect("checked entry")
        };
        execution::release(&self.0, Some(id)).await;
        entry.close().await;
        Ok(())
    }
    /// Trusted host reset revokes all handles, file grants and pending registrations, cancels
    /// in-flight setups and closes owned tunnels, retaining capacity. Closing pools waits for
    /// bounded active queries; it does not confirm server cancellation. Job records are
    /// released (their renderer is gone); unknown outcomes stay listable until acknowledged.
    pub async fn reset(&self) {
        let entries: Vec<_> = {
            let mut entries = self.0.entries.lock().expect("connection registry lock");
            self.0.generation.fetch_add(1, Ordering::SeqCst);
            self.0.grants.lock().expect("file grant lock").clear();
            entries.drain().map(|(_, entry)| entry).collect()
        };
        self.cancel_all_attempts();
        execution::release(&self.0, None).await;
        self.0.execution.forget_jobs();
        for entry in entries {
            entry.close().await;
        }
    }

    /// Host shutdown closes every pool. Never retries user SQL or claims server cancellation.
    pub async fn shutdown(&self) {
        self.0.slots.close();
        self.0.grants.lock().expect("file grant lock").clear();
        self.cancel_all_attempts();
        let entries: Vec<_> = self
            .0
            .entries
            .lock()
            .expect("connection registry lock")
            .drain()
            .map(|(_, entry)| entry)
            .collect();
        execution::release(&self.0, None).await;
        for entry in entries {
            entry.close().await;
        }
    }
    /// Registered handles still holding an SSH tunnel; for lifecycle evidence only.
    pub fn open_tunnels(&self) -> usize {
        self.0
            .entries
            .lock()
            .expect("connection registry lock")
            .values()
            .filter(|entry| entry.tunnel.is_some())
            .count()
    }
    /// In-flight setups; for lifecycle evidence only.
    pub fn pending_setups(&self) -> usize {
        self.0.pending.lock().expect("pending lock").len()
    }
}
fn file_name(path: &Path) -> Result<String, LifecycleError> {
    Ok(path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::mysql::MySqlPoolOptions;
    fn config() -> DirectServer {
        DirectServer {
            engine: Engine::MySql,
            host: "127.0.0.1".into(),
            port: 3306,
            username: "fixture".into(),
            password: "do_not_log_secret".into(),
            database: "fixture".into(),
            root_certificate_pem: None,
            tls: true,
            rsa_key: super::PasswordRsaKey::None,
        }
    }
    /// A throwaway 2048-bit RSA public key (test data only).
    const TEST_PUBLIC_KEY: &str = include_str!("../tests/fixtures/rsa-public-key.pem");
    #[test]
    fn plaintext_is_an_explicit_mysql_choice_and_tls_options_never_fall_back() {
        let plain = |engine: Engine, host: &str| DirectServer {
            engine,
            host: host.into(),
            tls: false,
            rsa_key: super::PasswordRsaKey::None,
            ..config()
        };
        for host in ["localhost", "LOCALHOST", "127.0.0.1", "::1"] {
            assert!(is_loopback_host(host));
        }
        // TLS off is an explicit choice for any host, local or remote.
        for host in [
            "localhost",
            "127.0.0.1",
            "::1",
            "192.0.2.10",
            "db.example.com",
        ] {
            for engine in [Engine::MySql, Engine::MariaDb] {
                assert_eq!(plain(engine, host).validate(), Ok(()), "{host}");
                let options = mysql::options(&plain(engine, host), None).unwrap();
                assert!(matches!(
                    options.get_ssl_mode(),
                    sqlx::mysql::MySqlSslMode::Disabled
                ));
            }
        }
        for host in ["127.0.0.2", "db.example.com", "[::1]"] {
            assert!(!is_loopback_host(host));
        }
        assert_eq!(
            plain(Engine::PostgreSql, "127.0.0.1").validate(),
            Err(LifecycleError::InvalidConfiguration)
        );
        // A CA has no meaning without TLS: refused instead of silently ignored.
        let mut with_ca = plain(Engine::MySql, "127.0.0.1");
        with_ca.root_certificate_pem = Some(b"-----BEGIN CERTIFICATE-----".to_vec());
        assert_eq!(
            with_ca.validate(),
            Err(LifecycleError::InvalidConfiguration)
        );
        // TLS on: always verify-identity, on loopback and remote alike.
        for host in ["127.0.0.1", "db.example.com"] {
            let options = mysql::options(
                &DirectServer {
                    host: host.into(),
                    ..config()
                },
                None,
            )
            .unwrap();
            assert!(matches!(
                options.get_ssl_mode(),
                sqlx::mysql::MySqlSslMode::VerifyIdentity
            ));
        }
        // RSA options: only without TLS, and a trusted key must be an RSA public key.
        let with_key = |tls: bool, rsa_key: PasswordRsaKey| DirectServer {
            tls,
            rsa_key,
            ..config()
        };
        let trusted = || PasswordRsaKey::Trusted(TEST_PUBLIC_KEY.as_bytes().to_vec());
        assert_eq!(with_key(false, trusted()).validate(), Ok(()));
        // MySQL's public_key.pem ends with a NUL byte.
        let with_nul = PasswordRsaKey::Trusted(format!("{TEST_PUBLIC_KEY}\0").into_bytes());
        assert_eq!(with_key(false, with_nul).validate(), Ok(()));
        assert_eq!(
            with_key(false, PasswordRsaKey::RetrieveUnverified).validate(),
            Ok(())
        );
        for rsa_key in [trusted(), PasswordRsaKey::RetrieveUnverified] {
            assert_eq!(
                with_key(true, rsa_key).validate(),
                Err(LifecycleError::InvalidConfiguration)
            );
        }
        let with_private = format!("{TEST_PUBLIC_KEY}-----BEGIN PRIVATE KEY-----");
        for pem in [
            &b""[..],
            b"-----BEGIN PUBLIC KEY-----\nnot base64\n-----END PUBLIC KEY-----\n",
            b"-----BEGIN CERTIFICATE-----\n-----END CERTIFICATE-----\n",
            with_private.as_bytes(),
            &[b'a'; 16_385][..],
        ] {
            assert_eq!(
                with_key(false, PasswordRsaKey::Trusted(pem.to_vec())).validate(),
                Err(LifecycleError::InvalidConfiguration)
            );
        }
        // The driver's refusal to fetch the key and the secure-transport refusal are
        // classified, not reported as a generic failure.
        assert_eq!(
            connect_error(sqlx::Error::Configuration(
                "RSA public key retrieval is not allowed; set a trusted server public key, allow retrieval or use TLS.".into()
            )),
            LifecycleError::EncryptedLoginRequired
        );
        assert_eq!(
            connect_error(sqlx::Error::Protocol("other".into())),
            LifecycleError::Unavailable
        );
    }
    fn details() -> Details {
        Details {
            engine: Engine::MySql,
            engine_version: "fixture".into(),
            access: Access::ReadOnly,
        }
    }
    fn register(connections: &Connections, owner: OwnerId) -> ConnectionId {
        let permit = connections.0.slots.clone().try_acquire_owned().unwrap();
        let pool = lazy_pool();
        connections
            .register(
                owner,
                pool,
                &mut None,
                permit,
                details(),
                connections.0.generation.load(Ordering::SeqCst),
            )
            .unwrap()
    }
    fn lazy_pool() -> EnginePool {
        EnginePool::MySql(
            MySqlPoolOptions::new()
                .max_connections(1)
                .min_connections(0)
                .connect_lazy_with(mysql::options(&config(), None).unwrap()),
        )
    }
    /// F-02: a host-registered attempt is reached by reset and unregistered when dropped.
    #[tokio::test]
    async fn host_attempts_are_cancelled_by_reset_and_released_on_drop() {
        let connections =
            Connections::new(8, Duration::from_secs(3), Duration::from_secs(60)).unwrap();
        let owner = OwnerId::new();
        let mut held = connections.begin_attempt(owner, Some("attempt_1")).unwrap();
        assert!(
            connections.begin_attempt(owner, Some("attempt_1")).is_err(),
            "duplicate name"
        );
        assert_eq!(connections.pending_setups(), 1);
        connections.reset().await;
        tokio::time::timeout(Duration::from_secs(1), held.cancelled())
            .await
            .expect("reset cancels a held attempt");
        let dropped = connections.begin_attempt(owner, Some("attempt_2")).unwrap();
        assert_eq!(connections.pending_setups(), 1);
        drop(dropped);
        assert_eq!(connections.pending_setups(), 0);
        assert!(connections.begin_attempt(owner, Some("attempt_2")).is_ok());
        drop(held);
    }
    #[tokio::test]
    async fn handles_are_owner_scoped_and_disconnect_releases_capacity() {
        let connections =
            Connections::new(1, Duration::from_secs(1), Duration::from_secs(60)).unwrap();
        let owner = OwnerId::new();
        let other = OwnerId::new();
        let id = register(&connections, owner);
        assert!(connections.is_connected(owner, id));
        assert!(!connections.is_connected(other, id));
        assert_eq!(
            connections.disconnect(other, id).await,
            Err(LifecycleError::Forbidden)
        );
        assert!(connections.is_connected(owner, id));
        assert!(connections.0.slots.clone().try_acquire_owned().is_err());
        connections.disconnect(owner, id).await.unwrap();
        assert!(!connections.is_connected(owner, id));
        assert!(connections.0.slots.clone().try_acquire_owned().is_ok());
    }
    #[tokio::test(start_paused = true)]
    async fn expiry_closes_pool_and_releases_capacity_without_polling() {
        let connections =
            Connections::new(1, Duration::from_secs(1), Duration::from_secs(60)).unwrap();
        let owner = OwnerId::new();
        let id = register(&connections, owner);
        let pool = connections.0.entries.lock().unwrap()[&id].pool.clone();
        tokio::time::advance(Duration::from_secs(61)).await;
        tokio::task::yield_now().await;
        assert!(!connections.is_connected(owner, id));
        assert!(pool.is_closed());
        assert!(connections.0.slots.clone().try_acquire_owned().is_ok());
    }
    /// The lifetime is idle time: each use starts it again, status checks do not, and an
    /// unused handle still ends on time.
    #[tokio::test(start_paused = true)]
    async fn use_keeps_a_handle_open_and_status_checks_do_not() {
        let connections =
            Connections::new(1, Duration::from_secs(1), Duration::from_secs(60)).unwrap();
        let owner = OwnerId::new();
        let id = register(&connections, owner);
        for _ in 0..3 {
            tokio::time::advance(Duration::from_secs(40)).await;
            assert!(connections.owned_pool(owner, id).is_some(), "a use");
        }
        tokio::task::yield_now().await;
        assert!(connections.is_connected(owner, id), "120 s old, 40 s idle");
        for _ in 0..3 {
            tokio::time::advance(Duration::from_secs(20)).await;
            tokio::task::yield_now().await;
            let _ = connections.is_connected(owner, id);
        }
        tokio::task::yield_now().await;
        assert!(
            !connections.is_connected(owner, id),
            "60 s of status checks only"
        );
        assert!(connections.0.slots.clone().try_acquire_owned().is_ok());
    }
    #[tokio::test]
    async fn shutdown_closes_all_pools_and_rejects_new_work() {
        let connections =
            Connections::new(2, Duration::from_secs(1), Duration::from_secs(60)).unwrap();
        let owner = OwnerId::new();
        let id = register(&connections, owner);
        connections.shutdown().await;
        assert!(!connections.is_connected(owner, id));
        assert_eq!(
            connections.connect(owner, config()).await,
            Err(LifecycleError::Capacity)
        );
    }
    #[tokio::test]
    async fn shutdown_rejects_an_inflight_connection_before_registration() {
        let connections =
            Connections::new(1, Duration::from_secs(1), Duration::from_secs(60)).unwrap();
        let permit = connections.0.slots.clone().try_acquire_owned().unwrap();
        let pool = lazy_pool();
        connections.shutdown().await;
        assert_eq!(
            connections.register(
                OwnerId::new(),
                pool.clone(),
                &mut None,
                permit,
                details(),
                0
            ),
            Err(LifecycleError::Capacity)
        );
        pool.close().await;
        assert!(connections.0.entries.lock().unwrap().is_empty());
    }
    #[tokio::test]
    async fn reset_revokes_handles_closes_pools_and_preserves_new_capacity() {
        let connections =
            Connections::new(2, Duration::from_secs(1), Duration::from_secs(60)).unwrap();
        let owner = OwnerId::new();
        let first = register(&connections, owner);
        let second = register(&connections, OwnerId::new());
        let pools: Vec<_> = connections
            .0
            .entries
            .lock()
            .unwrap()
            .values()
            .map(|entry| entry.pool.clone())
            .collect();
        connections.reset().await;
        assert!(!connections.is_connected(owner, first));
        assert!(pools.iter().all(EnginePool::is_closed));
        assert!(connections.0.entries.lock().unwrap().get(&second).is_none());
        let next = register(&connections, owner);
        assert!(connections.is_connected(owner, next));
        connections.shutdown().await;
    }
    #[tokio::test]
    async fn reset_rejects_late_registration_from_the_previous_generation() {
        let connections =
            Connections::new(1, Duration::from_secs(1), Duration::from_secs(60)).unwrap();
        let generation = connections.0.generation.load(Ordering::SeqCst);
        let permit = connections.0.slots.clone().try_acquire_owned().unwrap();
        let pool = lazy_pool();
        connections.reset().await;
        assert_eq!(
            connections.register(
                OwnerId::new(),
                pool.clone(),
                &mut None,
                permit,
                details(),
                generation
            ),
            Err(LifecycleError::Forbidden)
        );
        pool.close().await;
        assert!(connections.0.slots.clone().try_acquire_owned().is_ok());
        assert!(connections.0.entries.lock().unwrap().is_empty());
    }
    #[tokio::test]
    async fn stalled_handshake_obeys_deadline_without_leaking_capacity() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.unwrap();
            std::future::pending::<()>().await;
        });
        let connections =
            Connections::new(1, Duration::from_millis(100), Duration::from_secs(60)).unwrap();
        let mut input = config();
        input.port = port;
        assert_eq!(
            connections.connect(OwnerId::new(), input).await,
            Err(LifecycleError::Timeout)
        );
        assert!(connections.0.slots.clone().try_acquire_owned().is_ok());
        assert!(connections.0.entries.lock().unwrap().is_empty());
        server.abort();
    }
    #[tokio::test]
    async fn pending_setup_is_cancelled_only_by_its_owner_reset_or_shutdown() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((socket, _)) = listener.accept().await {
                held.push(socket);
            }
        });
        let connections =
            Connections::new(2, Duration::from_secs(3), Duration::from_secs(60)).unwrap();
        let owner = OwnerId::new();
        let start = |attempt: &'static str| {
            let connections = connections.clone();
            let mut input = config();
            input.port = port;
            tokio::spawn(async move {
                connections
                    .connect_with(owner, input, None, Some(attempt))
                    .await
            })
        };
        let first = start("attempt_a");
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(connections.pending_setups(), 1);
        let mut duplicate = config();
        duplicate.port = port;
        assert_eq!(
            connections
                .connect_with(owner, duplicate, None, Some("attempt_a"))
                .await,
            Err(LifecycleError::InvalidConfiguration)
        );
        assert!(!connections.cancel_connect(OwnerId::new(), "attempt_a"));
        assert!(connections.cancel_connect(owner, "attempt_a"));
        assert_eq!(first.await.unwrap(), Err(LifecycleError::Cancelled));
        assert!(!connections.cancel_connect(owner, "attempt_a"));
        let second = start("attempt_b");
        tokio::time::sleep(Duration::from_millis(50)).await;
        connections.reset().await;
        assert_eq!(second.await.unwrap(), Err(LifecycleError::Cancelled));
        let third = start("attempt_c");
        tokio::time::sleep(Duration::from_millis(50)).await;
        connections.shutdown().await;
        assert_eq!(third.await.unwrap(), Err(LifecycleError::Cancelled));
        assert_eq!(connections.pending_setups(), 0);
        assert!(connections.0.entries.lock().unwrap().is_empty());
        server.abort();
    }
    #[cfg(feature = "sqlite")]
    #[test]
    fn file_grants_are_typed_and_owner_scoped() {
        let path = std::env::temp_dir().join(format!("stoneql-grant-{}", Uuid::new_v4()));
        std::fs::write(&path, b"synthetic key text").unwrap();
        let path = std::fs::canonicalize(&path).unwrap();
        let connections =
            Connections::new(1, Duration::from_secs(1), Duration::from_secs(60)).unwrap();
        let owner = OwnerId::new();
        let sqlite = connections.grant_sqlite_file(owner, &path).unwrap();
        let key = connections.grant_ssh_key_file(owner, &path).unwrap();
        assert_ne!(sqlite, key);
        assert!(connections.ssh_key_file_name(owner, &sqlite).is_err());
        assert!(connections.sqlite_file_name(owner, &key).is_err());
        assert!(connections.ssh_key_file_name(OwnerId::new(), &key).is_err());
        assert_eq!(
            connections.read_key_file(owner, &key).unwrap(),
            "synthetic key text"
        );
        assert_eq!(
            connections.read_key_file(owner, &sqlite),
            Err(LifecycleError::Forbidden)
        );
        std::fs::write(&path, vec![b'k'; 16_385]).unwrap();
        assert_eq!(
            connections.read_key_file(owner, &key),
            Err(LifecycleError::InvalidConfiguration)
        );
        std::fs::remove_file(&path).unwrap();
        assert_eq!(
            connections.read_key_file(owner, &key),
            Err(LifecycleError::Forbidden)
        );
    }
    #[tokio::test]
    async fn postgres_read_write_over_ssh_is_refused_before_network_access() {
        let connections =
            Connections::new(1, Duration::from_secs(1), Duration::from_secs(60)).unwrap();
        let mut postgres = config();
        postgres.engine = Engine::PostgreSql;
        postgres.port = 5432;
        let tunnel = SshConfig {
            host: "192.0.2.1".into(),
            port: 22,
            username: "fixture".into(),
            host_key: None,
            auth: SshAuth::Agent(None),
        };
        assert_eq!(
            connections
                .connect_with_access(
                    OwnerId::new(),
                    postgres,
                    Some(tunnel),
                    None,
                    Access::ReadWrite
                )
                .await,
            Err(LifecycleError::Unsupported)
        );
        assert_eq!(connections.pending_setups(), 0);
    }
    #[test]
    fn protocol_identifiers_reject_embedded_nuls_instead_of_truncating() {
        for field in ["host", "username", "database"] {
            let mut input = config();
            match field {
                "host" => input.host.push('\0'),
                "username" => input.username.push('\0'),
                _ => input.database.push('\0'),
            }
            assert!(matches!(
                mysql::options(&input, None),
                Err(LifecycleError::InvalidConfiguration)
            ));
            input.engine = Engine::PostgreSql;
            assert!(matches!(
                postgres::options(&input, None),
                Err(LifecycleError::InvalidConfiguration)
            ));
        }
    }
    #[test]
    fn postgres_refuses_unix_sockets_and_engine_mismatch() {
        let mut input = config();
        input.engine = Engine::PostgreSql;
        input.port = 5432;
        assert!(postgres::options(&input, None).is_ok());
        input.host = "/var/run/postgresql".into();
        assert!(matches!(
            postgres::options(&input, None),
            Err(LifecycleError::InvalidConfiguration)
        ));
        assert!(matches!(
            mysql::options(
                &DirectServer {
                    engine: Engine::PostgreSql,
                    ..config()
                },
                None
            ),
            Err(LifecycleError::InvalidConfiguration)
        ));
    }
    #[test]
    fn configuration_and_error_messages_do_not_expose_credentials() {
        let mut input = config();
        input.engine = Engine::Sqlite;
        assert!(matches!(
            input.validate(),
            Err(LifecycleError::InvalidConfiguration)
        ));
        for error in [
            LifecycleError::Unavailable,
            LifecycleError::Timeout,
            LifecycleError::EngineMismatch,
        ] {
            assert!(!error.to_string().contains(&input.password));
        }
        assert!(Connections::new(65, Duration::from_secs(1), Duration::from_secs(60)).is_err());
    }
}
