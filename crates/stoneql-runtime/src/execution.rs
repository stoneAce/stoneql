//! Bounded read-write SQL jobs, transaction leases and best-effort cancellation.
//!
//! Every job and lease runs on its own physical connection opened from the handle's verified
//! connect options (and SSH endpoint). These connections never enter a pool: they are closed
//! when the job or lease ends, so session state (variables, `USE`, temporary tables, locks,
//! open transactions) cannot reach another job or the read-only path. Nothing here retries SQL.
#[cfg(feature = "sqlite")]
use super::sqlite;
use super::{
    ConnectionId, Connections, EnginePool, Inner, LifecycleError, OwnerId, ReadError,
    connect_error, mysql, postgres,
    script::{self, ScriptError, Statement},
};
use futures_util::{StreamExt, stream::BoxStream};
use journal::{Entry, Journal};
use sqlx::{
    Connection, Database, Either, Executor, MySql, MySqlConnection, PgConnection, Postgres, Row,
    SqlSafeStr, Statement as _, TypeInfo,
    mysql::{MySqlConnectOptions, MySqlDatabaseError},
    postgres::PgConnectOptions,
};
#[cfg(feature = "sqlite")]
use sqlx::{Sqlite, SqliteConnection};
#[cfg(feature = "sqlite")]
use std::path::PathBuf;
#[cfg(feature = "sqlite")]
use std::sync::atomic::{AtomicBool, Ordering};
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
use stoneql_contracts::v1::{
    self, CancellationDto, CellValue, Column, ConstraintKindDto, EngineDto, JobStateDto, JobUpdate,
    JournalDamage, JournalProblemDto, LeaseStateDto, OutcomeDto, ResultBatch, StatementFailure,
    StatementReport, StatementStatusDto, TransactionEndReasonDto, UnresolvedJob, UnresolvedOutcome,
    UnresolvedOutcomes,
};
use stoneql_core::{Engine, ExecutionOutcome, Outcome};
use tokio::{
    sync::{Notify, OwnedMutexGuard, OwnedSemaphorePermit, Semaphore, mpsc},
    time::Instant,
};
use uuid::Uuid;

/// Every execution bound in one place. PROPOSED working defaults (values from the M006 safety
/// contracts proposal) implemented under the user's 2026-10-26 continuation instruction;
/// awaiting review, not approved budgets.
pub struct ExecutionLimits;
impl ExecutionLimits {
    /// Running jobs per handle. A busy handle refuses work with BUSY; nothing is queued, so a
    /// write never runs later after the user believed it abandoned. Per owner this is bounded by
    /// the registry's handle capacity.
    pub const RUNNING_JOBS_PER_HANDLE: usize = 1;
    /// Job records per owner (running plus finished with unread pages). This also bounds the
    /// bytes an owner can leave buffered (B-7): a record holds at most `BUFFERED_PAGES` unread
    /// pages plus the page its worker is about to send, and a page is at most `PAGE_BYTES`
    /// unless one row alone is larger (up to `MAX_BYTES_PER_RESULT`). Typical worst case
    /// 8 × 3 × 256 KiB = 6 MiB per owner; a single-row-page worst case 8 × 3 × 1 MiB = 24 MiB.
    pub const MAX_JOB_RECORDS_PER_OWNER: usize = 8;
    /// Open transactions: one per handle (it makes the handle exclusive), two per owner.
    pub const MAX_LEASES_PER_OWNER: usize = 2;
    pub const MAX_STATEMENTS: usize = 100;
    pub const MAX_SCRIPT_BYTES: usize = 65_536;
    /// SQL import (A20, MySQL/MariaDB). PROPOSED values, benchmarked for sign-off: the whole
    /// file is held once in memory as the validated snapshot it executes from.
    pub const MAX_IMPORT_BYTES: usize = 1024 * 1024 * 1024;
    pub const MAX_IMPORT_STATEMENT_BYTES: usize = 16 * 1024 * 1024;
    /// Bounds the snapshot's statement index (16 bytes each).
    pub const MAX_IMPORT_STATEMENTS: usize = 1_000_000;
    /// Per statement; the whole import is bounded by `IMPORT_DEADLINE`.
    pub const IMPORT_STATEMENT_DEADLINE: Duration = Self::MAX_STATEMENT_DEADLINE;
    pub const IMPORT_DEADLINE: Duration = Duration::from_secs(2 * 3600);
    pub const DEFAULT_STATEMENT_DEADLINE: Duration = Duration::from_secs(30);
    pub const MAX_STATEMENT_DEADLINE: Duration = Duration::from_secs(300);
    pub const JOB_DEADLINE: Duration = Duration::from_secs(600);
    pub const DEFAULT_ROWS_PER_RESULT: usize = 1000;
    pub const MAX_ROWS_PER_RESULT: usize = 100_000;
    pub const MAX_BYTES_PER_RESULT: usize = 1_048_576;
    pub const MAX_RESULT_SETS: u32 = 16;
    /// Rows delivered by one job across all result sets; later sets are truncated and flagged.
    pub const MAX_BYTES_PER_JOB: usize = 4 * 1_048_576;
    pub const MAX_METADATA_BYTES: usize = 262_144;
    /// One page is the unit of backpressure; at most `BUFFERED_PAGES` wait for the consumer
    /// and the database stream is not read meanwhile.
    pub const PAGE_ROWS: usize = 500;
    pub const PAGE_BYTES: usize = 262_144;
    pub const BUFFERED_PAGES: usize = 2;
    /// After a cancel or deadline, how long the running statement may take to end before its
    /// connection is abandoned. Sized for the proposed cancel→terminal p95 ≤ 2 s.
    pub const CANCEL_GRACE: Duration = Duration::from_millis(1500);
    /// Opening the separate cancel connection (one SSH channel when tunnelled).
    pub const CANCEL_CONNECT_DEADLINE: Duration = Duration::from_secs(1);
    pub const FETCH_WAIT_MAX: Duration = Duration::from_secs(1);
    /// A job blocked on a full page buffer this long is treated as abandoned and interrupted.
    pub const CONSUMER_IDLE: Duration = Duration::from_secs(60);
    pub const TERMINAL_RETENTION: Duration = Duration::from_secs(300);
    pub const LEASE_IDLE: Duration = Duration::from_secs(60);
    pub const LEASE_MAX_LIFETIME: Duration = Duration::from_secs(900);
    pub const COMMIT_DEADLINE: Duration = Duration::from_secs(10);
    pub const ROLLBACK_DEADLINE: Duration = Duration::from_secs(2);
    /// Upper bound for releasing a handle's jobs and leases on close, reset or shutdown.
    pub const RELEASE_BOUND: Duration = Duration::from_secs(5);
    /// Resolved (or acknowledged) ended-lease outcomes kept per owner for a later
    /// `end_transaction` call (oldest of that owner evicted first), so one owner's ends never
    /// evict another owner's (R-6). Unacknowledged unknown outcomes are never evicted.
    pub const MAX_ENDED_LEASES_PER_OWNER: usize = 32;
    /// Resolved (or acknowledged) records kept across all owners; the oldest is evicted first.
    /// The desktop host has one owner per app process (kept across renderer reloads).
    pub const MAX_ENDED_LEASES: usize = 256;
    /// Unknown outcomes (lease ends and jobs) an owner may leave unacknowledged. At this count
    /// new jobs and transactions are refused (`UnresolvedOutcomes`) until the user acknowledges
    /// some; none is dropped. Work already admitted can still add its own: at most
    /// `MAX_LEASES_PER_OWNER` + `MAX_JOB_RECORDS_PER_OWNER` more (plus jobs a reset detached
    /// that are still ending).
    pub const MAX_UNRESOLVED_OUTCOMES_PER_OWNER: usize = 16;
    /// Lock waits for read-write SQLite connections (read-only stays at 1 s, D015).
    pub const SQLITE_WRITE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);
    /// `;` characters one SQLite statement may contain (each is checked with a prepare); a
    /// statement with more is refused rather than checked partially.
    pub const SQLITE_MAX_SEMICOLONS: usize = 256;
    /// Checking the session's transaction state after a job (R-3).
    pub const STATE_PROBE_DEADLINE: Duration = Duration::from_secs(2);
    /// PostgreSQL server-side timeout (L-3, Contract 5.3 ordering): each job sets
    /// `statement_timeout` this much **before** its statement deadline (see
    /// `postgres_statement_timeout`), so a timed-out statement normally ends on the server with
    /// SQLSTATE 57014 and its reply arrives before the client deadline; the client deadline and
    /// interrupt remain the backstop. Covers the reply's network/tunnel latency and the server's
    /// interrupt-check latency.
    pub const POSTGRES_TIMEOUT_MARGIN: Duration = Duration::from_secs(1);

    /// The PostgreSQL `statement_timeout` for a statement deadline: strictly shorter than the
    /// deadline, by `POSTGRES_TIMEOUT_MARGIN` but at most half the deadline, and never 0 ms
    /// (which PostgreSQL treats as "no timeout").
    pub fn postgres_statement_timeout(deadline: Duration) -> Duration {
        let timeout = deadline.saturating_sub(Self::POSTGRES_TIMEOUT_MARGIN.min(deadline / 2));
        Duration::from_millis(timeout.as_millis().max(1) as u64)
    }
}

macro_rules! opaque_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub struct $name(Uuid);
        impl $name {
            pub fn parse(value: &str) -> Result<Self, ExecutionError> {
                Uuid::parse_str(value)
                    .map(Self)
                    .map_err(|_| ExecutionError::Forbidden)
            }
            pub fn opaque(self) -> String {
                self.0.to_string()
            }
        }
    };
}
opaque_id!(JobId);
opaque_id!(LeaseId);
// A recorded outcome: the id of the lease or job it belongs to.
opaque_id!(OutcomeId);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    ReadOnly,
    ReadWrite,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionError {
    /// Unknown handle/job/lease, or one owned by another principal.
    Forbidden,
    /// Read-write work on a handle opened read-only.
    ReadOnlyHandle,
    InvalidLimits,
    Capacity,
    /// The handle is running a job or holds an open transaction, or the lease is in use.
    Busy,
    /// A statement in the transaction failed or was cancelled: only rollback is accepted.
    RollbackOnly,
    /// Refused before any statement ran; `usize` is the zero-based statement index.
    Script(usize, ScriptError),
    Connection(LifecycleError),
    /// The server refused to start the transaction.
    Database,
    /// `MAX_UNRESOLVED_OUTCOMES_PER_OWNER` unknown outcomes await acknowledgement.
    UnresolvedOutcomes,
    /// The durable unknown-outcome journal cannot be opened, validated or written, or a damaged
    /// journal awaits acknowledgement. Nothing was sent.
    JournalUnavailable,
}
impl std::fmt::Display for ExecutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Script(_, error) => error.fmt(f),
            Self::Connection(error) => error.fmt(f),
            other => f.write_str(match other {
                Self::Forbidden => "This job, transaction or connection is not available.",
                Self::ReadOnlyHandle => "This connection was opened read-only.",
                Self::InvalidLimits => "Invalid execution limits.",
                Self::Capacity => "Execution capacity reached.",
                Self::Busy => "The transaction or connection is busy.",
                Self::RollbackOnly => {
                    "A statement in this transaction failed; only rollback is possible."
                }
                Self::UnresolvedOutcomes => {
                    "Nothing was sent: acknowledge the unknown outcomes listed first. The server may or may not have applied them."
                }
                Self::JournalUnavailable => {
                    "Nothing was sent: StoneQL cannot save unknown outcomes durably right now. See the notice about the unknown-outcome journal."
                }
                _ => "The database refused to start the transaction.",
            }),
        }
    }
}
impl std::error::Error for ExecutionError {}

pub struct JobOptions {
    pub sql: String,
    pub lease: Option<LeaseId>,
    pub max_rows_per_result: usize,
    /// Applies to each statement; the whole job is also bounded by `JOB_DEADLINE`.
    pub statement_deadline: Duration,
}
impl JobOptions {
    pub fn new(sql: impl Into<String>) -> Self {
        Self {
            sql: sql.into(),
            lease: None,
            max_rows_per_result: ExecutionLimits::DEFAULT_ROWS_PER_RESULT,
            statement_deadline: ExecutionLimits::DEFAULT_STATEMENT_DEADLINE,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndReason {
    Requested,
    Expired,
    ConnectionLost,
    Closed,
}
/// `Committed`/`RolledBack` only after the server confirmed COMMIT/ROLLBACK; otherwise
/// `Unknown`. `commit_sent`/`rollback_sent` record whether COMMIT/ROLLBACK was ever attempted on
/// the lease's session, so an unknown outcome states what the server may have applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransactionEnd {
    pub outcome: Outcome,
    pub reason: EndReason,
    /// COMMIT was attempted: set just before it is written, so it is also set when the
    /// connection failed before any byte left. It never says the server received COMMIT.
    pub commit_sent: bool,
    /// ROLLBACK was attempted, with the same meaning as `commit_sent`.
    pub rollback_sent: bool,
}
impl From<TransactionEnd> for v1::TransactionEnd {
    fn from(end: TransactionEnd) -> Self {
        Self {
            outcome: match end.outcome {
                Outcome::Committed => OutcomeDto::Committed,
                Outcome::RolledBack => OutcomeDto::RolledBack,
                Outcome::Rejected => OutcomeDto::Rejected,
                Outcome::Unknown => OutcomeDto::Unknown,
            },
            reason: match end.reason {
                EndReason::Requested => TransactionEndReasonDto::Requested,
                EndReason::Expired => TransactionEndReasonDto::Expired,
                EndReason::ConnectionLost => TransactionEndReasonDto::ConnectionLost,
                EndReason::Closed => TransactionEndReasonDto::Closed,
            },
            commit_sent: end.commit_sent,
            rollback_sent: end.rollback_sent,
        }
    }
}
impl TransactionEnd {
    /// A journaled end (always unknown) restored at startup.
    fn restored(end: &v1::TransactionEnd) -> Self {
        Self {
            outcome: Outcome::Unknown,
            reason: match end.reason {
                TransactionEndReasonDto::Requested => EndReason::Requested,
                TransactionEndReasonDto::Expired => EndReason::Expired,
                TransactionEndReasonDto::ConnectionLost => EndReason::ConnectionLost,
                TransactionEndReasonDto::Closed => EndReason::Closed,
            },
            commit_sent: end.commit_sent,
            rollback_sent: end.rollback_sent,
        }
    }
    /// Recorded before anything is sent; replaced only by what the server confirms.
    fn unknown(reason: EndReason) -> Self {
        Self {
            outcome: Outcome::Unknown,
            reason,
            commit_sent: false,
            rollback_sent: false,
        }
    }
}

/// Non-secret connection context of a recorded outcome: the engine and the database (or SQLite
/// file) name. Never host, user, credentials or SQL.
#[derive(Clone, Debug, Default)]
struct Origin {
    engine: Option<EngineDto>,
    database: String,
}
impl Origin {
    fn of(engine: Engine, target: &Target) -> Self {
        Self {
            engine: Some(match engine {
                Engine::MySql => EngineDto::MySql,
                Engine::MariaDb => EngineDto::MariaDb,
                Engine::PostgreSql => EngineDto::PostgreSql,
                Engine::Sqlite => EngineDto::Sqlite,
            }),
            database: target.database(),
        }
    }
}
impl From<&Origin> for Origin {
    fn from(origin: &Origin) -> Self {
        origin.clone()
    }
}
impl From<&str> for Origin {
    fn from(database: &str) -> Self {
        Self {
            engine: None,
            database: database.to_owned(),
        }
    }
}
impl From<String> for Origin {
    fn from(database: String) -> Self {
        Self {
            engine: None,
            database,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Physical connections and server-side interruption.

#[derive(Clone)]
enum Target {
    MySql(MySqlConnectOptions),
    Postgres(PgConnectOptions),
    #[cfg(feature = "sqlite")]
    Sqlite(PathBuf),
}
impl Target {
    /// Display context for a recorded outcome: the database (or SQLite file) name, bounded.
    /// Never host, user, credentials or SQL.
    fn database(&self) -> String {
        let name = match self {
            Self::MySql(options) => options.get_database().unwrap_or_default().to_owned(),
            Self::Postgres(options) => options.get_database().unwrap_or_default().to_owned(),
            #[cfg(feature = "sqlite")]
            Self::Sqlite(path) => path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
        };
        name.chars().take(128).collect()
    }
    fn of(pool: &EnginePool) -> Self {
        match pool {
            EnginePool::MySql(pool) => Self::MySql((*pool.connect_options()).clone()),
            EnginePool::Postgres(pool) => Self::Postgres((*pool.connect_options()).clone()),
            #[cfg(feature = "sqlite")]
            EnginePool::Sqlite(pool) => {
                Self::Sqlite(pool.connect_options().get_filename().to_path_buf())
            }
        }
    }
    async fn open(&self) -> Result<Physical, LifecycleError> {
        Ok(match self {
            Self::MySql(options) => {
                let mut connection = MySqlConnection::connect_with(options)
                    .await
                    .map_err(connect_error)?;
                let id = sqlx::query_scalar::<_, u64>("SELECT CONNECTION_ID()")
                    .fetch_one(&mut connection)
                    .await
                    .map_err(connect_error)?;
                Physical {
                    session: Session::MySql(connection),
                    interrupt: Interrupt::MySql(options.clone(), id),
                }
            }
            Self::Postgres(options) => {
                let mut connection = PgConnection::connect_with(options)
                    .await
                    .map_err(connect_error)?;
                let pid = sqlx::query_scalar::<_, i32>("SELECT pg_backend_pid()")
                    .fetch_one(&mut connection)
                    .await
                    .map_err(connect_error)?;
                Physical {
                    session: Session::Postgres(connection),
                    interrupt: Interrupt::Postgres(options.clone(), pid),
                }
            }
            #[cfg(feature = "sqlite")]
            Self::Sqlite(path) => {
                // Re-verify the granted canonical file at every open (D014).
                super::files::verify(path)?;
                let mut connection = SqliteConnection::connect_with(&sqlite::write_options(path))
                    .await
                    .map_err(connect_error)?;
                let flag = Arc::new(AtomicBool::new(false));
                let interrupted = flag.clone();
                connection
                    .lock_handle()
                    .await
                    .map_err(connect_error)?
                    .set_progress_handler(1_000, move || !interrupted.load(Ordering::SeqCst));
                Physical {
                    session: Session::Sqlite(connection),
                    interrupt: Interrupt::Sqlite(flag),
                }
            }
        })
    }
}

enum Session {
    MySql(MySqlConnection),
    Postgres(PgConnection),
    #[cfg(feature = "sqlite")]
    Sqlite(SqliteConnection),
}
/// Best-effort server cancellation for one physical session. MySQL/MariaDB send `KILL QUERY`
/// and PostgreSQL `pg_cancel_backend` over a separate short-lived connection with the same
/// verified options (one SSH channel when tunnelled); SQLite interrupts its worker through the
/// progress handler.
enum Interrupt {
    MySql(MySqlConnectOptions, u64),
    Postgres(PgConnectOptions, i32),
    #[cfg(feature = "sqlite")]
    Sqlite(Arc<AtomicBool>),
    /// Test-only: interrupts the SQLite worker like `Sqlite`, then never completes, modelling a
    /// KILL/cancel that was written but whose reply never arrived (L-8).
    #[cfg(all(test, feature = "sqlite"))]
    Stalled(Arc<AtomicBool>),
}
impl Interrupt {
    /// True only when the signal was delivered; never proof that anything stopped. Fails fast
    /// (`CANCEL_CONNECT_DEADLINE`) when no cancel connection can be opened.
    async fn send(&self) -> bool {
        let deadline = ExecutionLimits::CANCEL_CONNECT_DEADLINE;
        match self {
            Self::MySql(options, id) => {
                let Ok(Ok(mut connection)) =
                    tokio::time::timeout(deadline, MySqlConnection::connect_with(options)).await
                else {
                    return false;
                };
                // `id` is the server-assigned numeric connection id, never request data.
                let sent =
                    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("KILL QUERY {id}")).into_sql_str())
                        .execute(&mut connection)
                        .await
                        .is_ok();
                let _ = connection.close().await;
                sent
            }
            Self::Postgres(options, pid) => {
                let Ok(Ok(mut connection)) =
                    tokio::time::timeout(deadline, PgConnection::connect_with(options)).await
                else {
                    return false;
                };
                let sent = sqlx::query_scalar::<_, bool>("SELECT pg_cancel_backend($1)")
                    .bind(*pid)
                    .fetch_one(&mut connection)
                    .await
                    .unwrap_or(false);
                let _ = connection.close().await;
                sent
            }
            #[cfg(feature = "sqlite")]
            Self::Sqlite(flag) => {
                flag.store(true, Ordering::SeqCst);
                true
            }
            #[cfg(all(test, feature = "sqlite"))]
            Self::Stalled(flag) => {
                flag.store(true, Ordering::SeqCst);
                std::future::pending().await
            }
        }
    }
    fn rearm(&self) {
        match self {
            #[cfg(feature = "sqlite")]
            Self::Sqlite(flag) => flag.store(false, Ordering::SeqCst),
            #[cfg(all(test, feature = "sqlite"))]
            Self::Stalled(flag) => flag.store(false, Ordering::SeqCst),
            _ => {}
        }
    }
}

struct Physical {
    session: Session,
    interrupt: Interrupt,
}
impl Physical {
    /// Fixed, trusted transaction statements only (never request data).
    async fn control(&mut self, sql: &'static str) -> Result<(), sqlx::Error> {
        let statement = sqlx::raw_sql(sqlx::AssertSqlSafe(sql).into_sql_str());
        match &mut self.session {
            Session::MySql(connection) => statement.execute(connection).await.map(drop),
            Session::Postgres(connection) => statement.execute(connection).await.map(drop),
            #[cfg(feature = "sqlite")]
            Session::Sqlite(connection) => statement.execute(connection).await.map(drop),
        }
    }
    async fn begin(&mut self) -> Result<(), sqlx::Error> {
        self.control(match self.session {
            Session::MySql(_) => "START TRANSACTION",
            _ => "BEGIN",
        })
        .await
    }
    /// Whether the session's transaction state matches StoneQL's model, checked on the server:
    /// inside a lease a transaction must still be open; outside one, autocommit must be on and
    /// no transaction open. Anything else, including a failed or slow probe, is a mismatch.
    /// MySQL/MariaDB: `@@autocommit`, and `SET TRANSACTION READ WRITE`, which the server refuses
    /// (ER 1568) exactly while a transaction is active (it only sets the next transaction's
    /// access mode, and the connection is never reused). SQLite: `BEGIN` fails only inside a
    /// transaction (outside a lease it is rolled back at once). PostgreSQL: see
    /// `postgres_state_matches`; `succeeded` says whether every statement of the job succeeded,
    /// because only a failed statement may leave a lease's transaction block aborted.
    async fn state_matches(&mut self, lease: bool, succeeded: bool) -> bool {
        let probe = async {
            match &mut self.session {
                Session::MySql(connection) => mysql_state_matches(connection, lease).await,
                #[cfg(feature = "sqlite")]
                Session::Sqlite(connection) => {
                    let begin = sqlx::raw_sql(sqlx::AssertSqlSafe("BEGIN").into_sql_str())
                        .execute(&mut *connection)
                        .await;
                    match begin {
                        Ok(_) => {
                            let _ = sqlx::raw_sql(sqlx::AssertSqlSafe("ROLLBACK").into_sql_str())
                                .execute(&mut *connection)
                                .await;
                            !lease
                        }
                        Err(sqlx::Error::Database(error))
                            if error.message().contains("within a transaction") =>
                        {
                            lease
                        }
                        Err(_) => false,
                    }
                }
                Session::Postgres(connection) => {
                    postgres_state_matches(connection, lease, !succeeded).await
                }
            }
        };
        tokio::time::timeout(ExecutionLimits::STATE_PROBE_DEADLINE, probe)
            .await
            .unwrap_or(false)
    }
    /// Discards the connection. Closing never claims anything about server-side state.
    async fn close(self) {
        let close = async move {
            let _ = match self.session {
                Session::MySql(connection) => connection.close().await,
                Session::Postgres(connection) => connection.close().await,
                #[cfg(feature = "sqlite")]
                Session::Sqlite(connection) => connection.close().await,
            };
        };
        let _ = tokio::time::timeout(ExecutionLimits::ROLLBACK_DEADLINE, close).await;
    }
    /// Ends a lease and always discards its connection. COMMIT/ROLLBACK are reported only as
    /// the server confirmed them. `record` receives the unknown outcome (with the sent facts)
    /// **before** each COMMIT/ROLLBACK is sent and the final outcome once known, so a caller cut
    /// off by a bound (release, shutdown) never loses the outcome. `record` returns whether the
    /// outcome is durable (journaled): COMMIT is sent only after its unknown outcome is, otherwise
    /// the lease is rolled back instead (ROLLBACK is always allowed: a lost ROLLBACK reply with
    /// COMMIT never sent cannot hide applied transaction work). On PostgreSQL/SQLite a COMMIT
    /// refused with a database error aborts the transaction, so ROLLBACK follows and is reported
    /// only when confirmed; on MySQL/MariaDB that abort is not proven, so the outcome is unknown.
    ///
    /// A `diverged` lease (a job found the session's transaction state not matching the lease)
    /// sends neither: its connection is discarded and the outcome stays unknown, because a
    /// confirmed COMMIT/ROLLBACK would no longer say what happened to the lease's work.
    async fn conclude(
        mut self,
        commit: bool,
        reason: EndReason,
        diverged: bool,
        record: impl Fn(TransactionEnd) -> bool,
    ) -> TransactionEnd {
        self.interrupt.rearm();
        let mut end = TransactionEnd::unknown(reason);
        if diverged {
            record(end);
            self.close().await;
            return end;
        }
        if commit {
            end.commit_sent = true;
            if !record(end) {
                // Write-ahead failed: COMMIT is never sent without a durable unknown record.
                end.commit_sent = false;
                end.rollback_sent = true;
                record(end);
                end.outcome = self.rollback().await;
                record(end);
                self.close().await;
                return end;
            }
            let statement = match self.session {
                Session::Postgres(_) => POSTGRES_COMMIT,
                _ => "COMMIT",
            };
            match tokio::time::timeout(ExecutionLimits::COMMIT_DEADLINE, self.control(statement))
                .await
            {
                Ok(Ok(())) => end.outcome = Outcome::Committed,
                // PostgreSQL 25P01: no transaction block, so COMMIT never ran and a ROLLBACK
                // would be answered OK without rolling anything back: unknown.
                Ok(Err(error @ sqlx::Error::Database(_)))
                    if !matches!(self.session, Session::MySql(_))
                        && !(matches!(self.session, Session::Postgres(_))
                            && sql_state(&error).as_deref() == Some("25P01")) =>
                {
                    end.rollback_sent = true;
                    record(end);
                    end.outcome = self.rollback().await;
                }
                _ => {}
            }
        } else {
            end.rollback_sent = true;
            record(end);
            end.outcome = self.rollback().await;
        }
        record(end);
        self.close().await;
        end
    }
    async fn rollback(&mut self) -> Outcome {
        match tokio::time::timeout(ExecutionLimits::ROLLBACK_DEADLINE, self.control("ROLLBACK"))
            .await
        {
            Ok(Ok(())) => Outcome::RolledBack,
            _ => Outcome::Unknown,
        }
    }
}

/// PostgreSQL COMMIT, sent as one simple-query message (R-10). On an aborted transaction block a
/// plain COMMIT is answered OK (command tag `ROLLBACK`), which SQLx reports as success. The
/// savepoint fails first there (SQLSTATE 25P02), and outside a block (25P01), and PostgreSQL
/// then skips the rest of the message, so COMMIT runs only on a healthy open block. Being one
/// message, COMMIT is still the first thing a lost reply can hit.
const POSTGRES_COMMIT: &str = "SAVEPOINT stoneql_commit; COMMIT";

/// The PostgreSQL COMMIT StoneQL sends (public so the disposable-fixture harness can check it
/// against a real server).
pub async fn postgres_commit(connection: &mut PgConnection) -> Result<(), sqlx::Error> {
    sqlx::raw_sql(sqlx::AssertSqlSafe(POSTGRES_COMMIT).into_sql_str())
        .execute(connection)
        .await
        .map(drop)
}

/// The PostgreSQL part of `Physical::state_matches` (public for the fixture harness).
/// `SAVEPOINT` fails with SQLSTATE 25P01 exactly outside a transaction block and with 25P02
/// inside an aborted one; inside a healthy block it succeeds and is released at once, which
/// changes nothing. `in_transaction`: a block must be open; `aborted_allowed`: an aborted block
/// also matches (the job had a failed statement, so the lease is rollback-only anyway).
pub async fn postgres_state_matches(
    connection: &mut PgConnection,
    in_transaction: bool,
    aborted_allowed: bool,
) -> bool {
    let control = |sql: &'static str| sqlx::raw_sql(sqlx::AssertSqlSafe(sql).into_sql_str());
    match control("SAVEPOINT stoneql_state")
        .execute(&mut *connection)
        .await
    {
        Ok(_) => {
            in_transaction
                && control("RELEASE SAVEPOINT stoneql_state")
                    .execute(&mut *connection)
                    .await
                    .is_ok()
        }
        Err(error) => match sql_state(&error).as_deref() {
            Some("25P01") => !in_transaction,
            Some("25P02") => in_transaction && aborted_allowed,
            _ => false,
        },
    }
}

/// The MySQL/MariaDB part of `Physical::state_matches` (public so the disposable-fixture
/// harness can check it against real servers). `in_transaction`: a transaction must be active;
/// otherwise autocommit must be on and none active.
pub async fn mysql_state_matches(connection: &mut MySqlConnection, in_transaction: bool) -> bool {
    if !in_transaction {
        let autocommit = sqlx::query_scalar::<_, i64>("SELECT CAST(@@autocommit AS SIGNED)")
            .persistent(false)
            .fetch_one(&mut *connection)
            .await;
        if !matches!(autocommit, Ok(1)) {
            return false;
        }
    }
    let probe = sqlx::raw_sql(sqlx::AssertSqlSafe("SET TRANSACTION READ WRITE").into_sql_str())
        .execute(&mut *connection)
        .await;
    match probe {
        Ok(_) => !in_transaction,
        // ER_CANT_CHANGE_TX_CHARACTERISTICS: raised exactly while a transaction is active.
        Err(error) if mysql_number(&error) == Some(1568) => in_transaction,
        Err(_) => false,
    }
}

// ---------------------------------------------------------------------------------------------
// Job control, progress and pages.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reason {
    Cancel,
    Deadline,
    Abandoned,
    Closed,
}
struct Control {
    reason: Mutex<Option<Reason>>,
    notify: Notify,
    job_deadline: Instant,
    /// The running statement's deadline, never later than the job deadline.
    deadline: Mutex<Instant>,
}
impl Control {
    fn arm(&self, statement: Duration) {
        *self.deadline.lock().expect("job control lock") =
            (Instant::now() + statement).min(self.job_deadline);
    }
    fn deadline(&self) -> Instant {
        *self.deadline.lock().expect("job control lock")
    }
    fn trip(&self, reason: Reason) {
        let mut current = self.reason.lock().expect("job control lock");
        if current.is_none() {
            *current = Some(reason);
        }
        drop(current);
        self.notify.notify_waiters();
    }
    fn reason(&self) -> Option<Reason> {
        *self.reason.lock().expect("job control lock")
    }
    fn is_interrupted(&self) -> bool {
        if self.reason().is_none() && Instant::now() >= self.deadline() {
            self.trip(Reason::Deadline);
        }
        self.reason().is_some()
    }
    /// Resolves when the job is cancelled, abandoned, closed or reaches a deadline.
    async fn interrupted(&self) -> Reason {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(reason) = self.reason() {
                return reason;
            }
            tokio::select! {
                _ = notified => {}
                _ = tokio::time::sleep_until(self.deadline()) => self.trip(Reason::Deadline),
            }
        }
    }
}

struct Progress {
    state: JobStateDto,
    outcome: Option<ExecutionOutcome>,
    statements: Vec<StatementReport>,
    server_cancel_sent: bool,
    /// A statement was ended by the PostgreSQL `statement_timeout` StoneQL set (L-3).
    server_timeout: bool,
    /// The interrupted work was observed to stop.
    stop_confirmed: bool,
    lease: Option<LeaseStateDto>,
    finished_at: Option<Instant>,
    /// SQL imports keep compact counters instead of one report per statement.
    import: Option<import::ImportProgress>,
}
struct Job {
    owner: OwnerId,
    handle: ConnectionId,
    control: Control,
    progress: Mutex<Progress>,
    finished: Notify,
}
impl Job {
    fn new(
        owner: OwnerId,
        handle: ConnectionId,
        deadline: Duration,
        statements: Vec<StatementReport>,
        lease: Option<LeaseStateDto>,
        import: Option<import::ImportProgress>,
    ) -> Arc<Self> {
        let job_deadline = Instant::now() + deadline;
        Arc::new(Job {
            owner,
            handle,
            control: Control {
                reason: Mutex::new(None),
                notify: Notify::new(),
                job_deadline,
                deadline: Mutex::new(job_deadline),
            },
            progress: Mutex::new(Progress {
                state: JobStateDto::Running,
                outcome: None,
                statements,
                server_cancel_sent: false,
                server_timeout: false,
                stop_confirmed: false,
                lease,
                finished_at: None,
                import,
            }),
            finished: Notify::new(),
        })
    }
    fn update(&self, change: impl FnOnce(&mut Progress)) {
        change(&mut self.progress.lock().expect("job progress lock"));
    }
    fn is_finished(&self) -> bool {
        self.progress
            .lock()
            .expect("job progress lock")
            .finished_at
            .is_some()
    }
    async fn wait_finished(&self) {
        loop {
            let notified = self.finished.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.is_finished() {
                return;
            }
            notified.await;
        }
    }
}
struct JobEntry {
    job: Arc<Job>,
    pages: Arc<tokio::sync::Mutex<mpsc::Receiver<ResultBatch>>>,
}

/// Accumulates decoded rows into bounded pages and applies backpressure: `flush` waits while
/// `BUFFERED_PAGES` pages are unread, so the database stream is not polled meanwhile.
struct Sink {
    tx: mpsc::Sender<ResultBatch>,
    statement: u32,
    next_result: u32,
    result_sets: u32,
    rows_per_result: usize,
    job_bytes: usize,
    batch: Option<ResultBatch>,
    batch_bytes: usize,
    set_rows: usize,
    set_bytes: usize,
    set_truncated: bool,
    /// The current statement's result data (rows, or a whole page or set) was read but not
    /// delivered because the job was interrupted (B-11). Reset per statement.
    withheld: bool,
    /// SQL import: result sets are driven to their end and discarded, never delivered, so a
    /// statement that returns rows cannot stall the import waiting for a reader.
    discard: bool,
}
impl Sink {
    fn new(tx: mpsc::Sender<ResultBatch>, rows_per_result: usize) -> Self {
        Self {
            discard: false,
            tx,
            statement: 0,
            next_result: 0,
            result_sets: 0,
            rows_per_result,
            job_bytes: 0,
            batch: None,
            batch_bytes: 0,
            set_rows: 0,
            set_bytes: 0,
            set_truncated: false,
            withheld: false,
        }
    }
    fn begin(&mut self, columns: Vec<Column>) {
        let size = serde_json::to_vec(&columns).map_or(usize::MAX, |json| json.len());
        // Past the job's result-set or byte bounds, sets are reported (flagged) without data.
        let withheld = self.discard
            || self.result_sets >= ExecutionLimits::MAX_RESULT_SETS
            || size > ExecutionLimits::MAX_METADATA_BYTES
            || self.job_bytes.saturating_add(size) > ExecutionLimits::MAX_BYTES_PER_JOB;
        if !withheld {
            self.job_bytes += size;
        }
        self.result_sets += 1;
        self.batch = Some(ResultBatch {
            statement: self.statement,
            result: self.next_result,
            columns: Some(if withheld { Vec::new() } else { columns }),
            rows: Vec::new(),
            last: false,
            truncated: false,
        });
        self.next_result += 1;
        self.batch_bytes = 0;
        self.set_rows = 0;
        self.set_bytes = 0;
        self.set_truncated = withheld;
    }
    /// Whether the next row is delivered. A row refused because of an interrupt is withheld.
    fn accepts(&mut self, control: &Control) -> bool {
        if control.is_interrupted() {
            self.withheld = true;
            return false;
        }
        !self.set_truncated
    }
    async fn push(&mut self, values: Vec<CellValue>, control: &Control) {
        let size =
            serde_json::to_vec(&values).map_or(usize::MAX, |json| json.len().saturating_add(1));
        if self.set_rows >= self.rows_per_result
            || self.set_bytes.saturating_add(size) > ExecutionLimits::MAX_BYTES_PER_RESULT
            || self.job_bytes.saturating_add(size) > ExecutionLimits::MAX_BYTES_PER_JOB
        {
            self.set_truncated = true;
            return;
        }
        let full = self.batch.as_ref().is_some_and(|batch| {
            !batch.rows.is_empty()
                && (batch.rows.len() >= ExecutionLimits::PAGE_ROWS
                    || self.batch_bytes + size > ExecutionLimits::PAGE_BYTES)
        });
        if full {
            self.flush(false, control).await;
        }
        if let Some(batch) = &mut self.batch {
            batch.rows.push(values);
            self.batch_bytes += size;
            self.set_bytes += size;
            self.job_bytes += size;
            self.set_rows += 1;
        }
    }
    async fn flush(&mut self, last: bool, control: &Control) {
        let Some(mut batch) = self.batch.take() else {
            return;
        };
        if self.discard {
            return;
        }
        batch.last = last;
        batch.truncated = self.set_truncated;
        if !last {
            self.batch = Some(ResultBatch {
                statement: batch.statement,
                result: batch.result,
                columns: None,
                rows: Vec::new(),
                last: false,
                truncated: false,
            });
            self.batch_bytes = 0;
        }
        // Interrupted jobs discard pages: the statement is only driven to its end.
        if control.is_interrupted() {
            self.withheld = true;
            return;
        }
        let mut dropped = false;
        tokio::select! {
            // After an interrupt nothing more is delivered, even if the consumer frees a slot.
            biased;
            _ = control.interrupted() => dropped = true,
            sent = self.tx.send(batch) => if sent.is_err() { control.trip(Reason::Abandoned) },
            _ = tokio::time::sleep(ExecutionLimits::CONSUMER_IDLE) => {
                control.trip(Reason::Abandoned);
                dropped = true;
            }
        }
        self.withheld |= dropped;
    }
}

struct Completed {
    affected: u64,
    result_sets: u32,
}
/// Drives one statement's stream to its end. Result-set boundaries come from the driver's
/// `QueryResult` items; rows past the bounds, or after an interrupt, are read and discarded so
/// the statement's end is still observed.
type Items<'s, DB> = BoxStream<
    's,
    Result<Either<<DB as Database>::QueryResult, <DB as Database>::Row>, sqlx::Error>,
>;
async fn drive<DB: Database>(
    mut stream: Items<'_, DB>,
    declared: Vec<Column>,
    decode: fn(&DB::Row, usize) -> Result<CellValue, ReadError>,
    affected: fn(&DB::QueryResult) -> u64,
    sink: &mut Sink,
    control: &Control,
) -> Result<Completed, sqlx::Error> {
    let mut done = Completed {
        affected: 0,
        result_sets: 0,
    };
    let mut open = false;
    while let Some(item) = stream.next().await {
        match item? {
            Either::Right(row) => {
                if !open {
                    let columns = if row.columns().is_empty() {
                        declared.clone()
                    } else {
                        columns(row.columns())
                    };
                    sink.begin(columns);
                    open = true;
                    done.result_sets += 1;
                }
                if sink.accepts(control) {
                    match (0..row.len()).map(|index| decode(&row, index)).collect() {
                        Ok(values) => sink.push(values, control).await,
                        Err(_) => sink.set_truncated = true,
                    }
                }
            }
            Either::Left(result) => {
                done.affected = done.affected.saturating_add(affected(&result));
                if open {
                    sink.flush(true, control).await;
                    open = false;
                } else if done.result_sets == 0 && !declared.is_empty() {
                    // A result set with zero rows still reports its prepared columns.
                    sink.begin(declared.clone());
                    sink.flush(true, control).await;
                    done.result_sets += 1;
                }
            }
        }
    }
    if open {
        sink.flush(true, control).await;
    }
    Ok(done)
}
fn columns<C: sqlx::Column>(columns: &[C]) -> Vec<Column> {
    columns
        .iter()
        .map(|column| Column {
            name: column.name().into(),
            database_type: column.type_info().name().into(),
            nullable: None,
        })
        .collect()
}
fn sql_state(error: &sqlx::Error) -> Option<String> {
    error
        .as_database_error()
        .and_then(|error| error.code())
        .map(|code| code.into_owned())
}
fn mysql_number(error: &sqlx::Error) -> Option<u16> {
    error
        .as_database_error()
        .and_then(|error| error.try_downcast_ref::<MySqlDatabaseError>())
        .map(MySqlDatabaseError::number)
}

/// One user statement on the job's physical session (binary protocol where the engine can
/// prepare it, so the reviewed lossless decoders apply).
async fn statement(
    session: &mut Session,
    sql: &str,
    sink: &mut Sink,
    control: &Control,
) -> Result<Completed, sqlx::Error> {
    // Explicitly requested SQL-editor text, unmodified; the script policy ran at submit.
    let text = || sqlx::AssertSqlSafe(sql.to_owned()).into_sql_str();
    match session {
        Session::MySql(connection) => match (&mut *connection).prepare(text()).await {
            Ok(prepared) => {
                let declared = columns(prepared.columns());
                let stream = (&mut *connection).fetch_many(prepared.query().persistent(false));
                drive::<MySql>(
                    stream,
                    declared,
                    mysql::decode_cell,
                    |done| done.rows_affected(),
                    sink,
                    control,
                )
                .await
            }
            // ER_UNSUPPORTED_PS (USE, routine DDL, LOCK TABLES…): text protocol instead.
            Err(error) if mysql_number(&error) == Some(1295) => {
                let stream = sqlx::raw_sql(text()).fetch_many(&mut *connection);
                drive::<MySql>(
                    stream,
                    Vec::new(),
                    mysql::decode_text_cell,
                    |done| done.rows_affected(),
                    sink,
                    control,
                )
                .await
            }
            Err(error) => Err(error),
        },
        Session::Postgres(connection) => {
            let prepared = (&mut *connection).prepare(text()).await?;
            let declared = columns(prepared.columns());
            let stream = (&mut *connection).fetch_many(prepared.query().persistent(false));
            drive::<Postgres>(
                stream,
                declared,
                postgres::decode_cell,
                |done| done.rows_affected(),
                sink,
                control,
            )
            .await
        }
        #[cfg(feature = "sqlite")]
        Session::Sqlite(connection) => {
            let prepared = (&mut *connection).prepare(text()).await?;
            let declared = columns(prepared.columns());
            let stream = (&mut *connection).fetch_many(prepared.query().persistent(false));
            drive::<Sqlite>(
                stream,
                declared,
                sqlite::decode_cell,
                |done| done.rows_affected(),
                sink,
                control,
            )
            .await
        }
    }
}

/// R-1: sqlx-sqlite runs *every* statement in the text it is given (its `VirtualStatement`
/// prepares and steps the tail left by `sqlite3_prepare`), so the text must be exactly one
/// statement as SQLite itself parses it, not as StoneQL's splitter does. Every `;` with text
/// after it is a candidate end; the prefix up to it is prepared (never executed). A statement is
/// accepted only if each such prefix fails with a parse error (`incomplete input` inside a
/// trigger body, an unterminated token, a syntax error): those depend only on the text, so SQLite
/// cannot end the statement there when it runs. A prefix that prepares (a real statement end,
/// or a `;` in a comment) or fails for any other reason (a missing table) refuses it. NUL bytes
/// end SQLite's input early and are refused too.
#[cfg(feature = "sqlite")]
async fn sqlite_single_statement(connection: &mut SqliteConnection, sql: &str) -> bool {
    if sql.contains('\0') {
        return false;
    }
    let ends: Vec<usize> = sql
        .match_indices(';')
        .map(|(index, _)| index)
        .filter(|&index| !sql[index + 1..].trim().is_empty())
        .collect();
    if ends.len() > ExecutionLimits::SQLITE_MAX_SEMICOLONS {
        return false;
    }
    for end in ends {
        let prefix = sqlx::AssertSqlSafe(sql[..=end].to_owned()).into_sql_str();
        match (&mut *connection).prepare(prefix).await {
            Err(sqlx::Error::Database(error)) if sqlite_parse_error(error.message()) => {}
            _ => return false,
        }
    }
    true
}
#[cfg(feature = "sqlite")]
fn sqlite_parse_error(message: &str) -> bool {
    message == "incomplete input"
        || message.starts_with("unrecognized token:")
        || message.ends_with(": syntax error")
}

type Classified = (
    StatementStatusDto,
    Option<StatementFailure>,
    Option<(u64, u32)>,
);
/// What one sent statement became, shared by jobs and SQL imports.
fn classify(guarded: &Guarded<Result<Completed, sqlx::Error>>) -> Classified {
    match &guarded.result {
        Some(Ok(done)) => (
            StatementStatusDto::Succeeded,
            None,
            Some((done.affected, done.result_sets)),
        ),
        // Interrupted after it was sent: even an observed end (a cancel error) cannot rule
        // out effects, so the statement is unknown.
        Some(Err(_)) if guarded.interrupted => (StatementStatusDto::Unknown, None, None),
        Some(Err(error @ sqlx::Error::Database(_))) if !session_lost(error) => {
            (StatementStatusDto::Failed, Some(failure(error)), None)
        }
        // Transport/protocol failure or abandoned: the server may have applied it.
        Some(Err(_)) | None => (StatementStatusDto::Unknown, None, None),
    }
}
struct Guarded<T> {
    /// `None` when the statement did not end within the cancel grace period.
    result: Option<T>,
    interrupted: bool,
    /// Whether the server interrupt was delivered; `None` when it was still in flight at the
    /// end of the grace period (written, or about to be, with no reply): it may still reach the
    /// session later, so the connection must not run anything else (L-8).
    signal: Option<bool>,
}
/// Runs `work`; on an interrupt, sends the best-effort server cancel while continuing to drive
/// `work` for at most `CANCEL_GRACE`. The connection is abandoned only after the cancel was
/// attempted, so a MySQL autocommit write is never left running by a dropped client alone.
async fn guarded<T>(
    work: impl Future<Output = T>,
    control: &Control,
    interrupt: &Interrupt,
) -> Guarded<T> {
    tokio::pin!(work);
    tokio::select! {
        biased;
        result = &mut work => return Guarded { result: Some(result), interrupted: false, signal: Some(false) },
        _ = control.interrupted() => {}
    }
    let until = Instant::now() + ExecutionLimits::CANCEL_GRACE;
    let (signal, result) = tokio::join!(
        tokio::time::timeout_at(until, interrupt.send()),
        tokio::time::timeout_at(until, &mut work)
    );
    Guarded {
        result: result.ok(),
        interrupted: true,
        signal: signal.ok(),
    }
}

/// The server ended the session itself (PostgreSQL admin/crash shutdown or connection class
/// 08; MySQL/MariaDB shutdown or killed connection): treated as connection loss, not as a
/// statement failure, because the statement's fate and the session are both gone.
fn session_lost(error: &sqlx::Error) -> bool {
    matches!(mysql_number(error), Some(1053 | 1927))
        || error
            .as_database_error()
            .and_then(|error| error.code())
            .is_some_and(|code| code.starts_with("08") || code.starts_with("57P"))
}
/// SQLSTATE 57014 (query_canceled) that is not another session's cancel. The caller attributes
/// it to the `statement_timeout` StoneQL set only when StoneQL sent no interrupt and the statement
/// ran at least that long. F-5: the message is localized by `lc_messages`, so it is only used to
/// exclude PostgreSQL's English "user request" cancel; with another language that cancel, if it
/// lands after the timeout point, is reported as the timeout (wording only: either way the
/// statement failed and, as PostgreSQL's timeout is disabled before a commit starts, was not
/// committed; inside a transaction block the block is aborted).
fn statement_timeout_error(error: &sqlx::Error) -> bool {
    error.as_database_error().is_some_and(|error| {
        error.code().as_deref() == Some("57014") && !error.message().contains("user request")
    })
}
fn failure(error: &sqlx::Error) -> StatementFailure {
    use sqlx::error::ErrorKind;
    let database = error.as_database_error();
    StatementFailure {
        constraint: database.and_then(|error| match error.kind() {
            ErrorKind::UniqueViolation => Some(ConstraintKindDto::Unique),
            ErrorKind::ForeignKeyViolation => Some(ConstraintKindDto::ForeignKey),
            ErrorKind::NotNullViolation => Some(ConstraintKindDto::NotNull),
            ErrorKind::CheckViolation => Some(ConstraintKindDto::Check),
            _ => None,
        }),
        denied: database.is_some_and(|error| {
            error
                .try_downcast_ref::<sqlx::mysql::MySqlDatabaseError>()
                .is_some_and(|error| {
                    matches!(error.number(), 1044 | 1045 | 1142 | 1143 | 1227 | 1370)
                })
                || (error
                    .try_downcast_ref::<sqlx::postgres::PgDatabaseError>()
                    .is_some()
                    && error.code().as_deref() == Some("42501"))
        }),
        // SQLSTATE only; database message text can contain row values and stays private.
        sql_state: database
            .and_then(|error| error.code())
            .filter(|code| code.len() == 5 && code.bytes().all(|byte| byte.is_ascii_alphanumeric()))
            .map(|code| code.into_owned()),
    }
}

// ---------------------------------------------------------------------------------------------
// Leases.

struct LeaseState {
    rollback_only: bool,
    /// A job found the session's transaction state not matching the lease (R-3): the lease is
    /// rollback-only and its end sends nothing and reports unknown.
    diverged: bool,
    last_used: Instant,
}
struct Lease {
    owner: OwnerId,
    handle: ConnectionId,
    origin: Origin,
    created: Instant,
    slot: Arc<tokio::sync::Mutex<Option<Physical>>>,
    state: Mutex<LeaseState>,
    /// The handle's exclusive work permit: while the lease exists, other work gets BUSY.
    _permit: OwnedSemaphorePermit,
}
impl Lease {
    fn touch(&self) {
        self.state.lock().expect("lease lock").last_used = Instant::now();
    }
    fn diverged(&self) -> bool {
        self.state.lock().expect("lease lock").diverged
    }
    fn due(&self) -> Instant {
        (self.state.lock().expect("lease lock").last_used + ExecutionLimits::LEASE_IDLE)
            .min(self.created + ExecutionLimits::LEASE_MAX_LIFETIME)
    }
}

pub(crate) struct ExecutionState {
    jobs: Mutex<HashMap<JobId, JobEntry>>,
    leases: Mutex<HashMap<LeaseId, Arc<Lease>>>,
    /// Leases being opened per owner (L-4). Changed only while `leases` is locked, so the
    /// per-owner count (open plus opening) is checked and reserved atomically.
    opening: Mutex<HashMap<OwnerId, usize>>,
    /// Recorded lease ends and unknown job outcomes, oldest first.
    ended: Mutex<Vec<Recorded>>,
    /// Jobs still running when a reset released their records (`RELEASE_BOUND` exceeded).
    /// They may still record an unknown outcome; reported as `pending` until finished.
    detached: Mutex<Vec<Arc<Job>>>,
    /// The durable journal of unresolved unknown outcomes and in-flight work; `None` for a
    /// volatile registry (runtime tests and harnesses that never attach one). Lock order:
    /// `ended`, then `journal`.
    journal: Mutex<Option<Journal>>,
}
/// What a recorded outcome describes. SQL-free: no statement text, values or driver text.
enum Facts {
    Lease(TransactionEnd),
    /// Only jobs whose outcome is unknown are recorded here.
    Job(UnresolvedJob),
    /// A journal that could not be read intact at startup.
    Damage(JournalDamage),
}
struct Recorded {
    id: Uuid,
    owner: OwnerId,
    origin: Origin,
    /// Unix seconds.
    at: u64,
    facts: Facts,
    /// The user acknowledged it: hidden from the unresolved list and evictable. The recorded
    /// outcome itself never changes.
    acknowledged: bool,
    /// Restored from the journal at startup (recorded before a restart).
    restored: bool,
    /// Restored work that was still in flight when the app exited.
    exited: bool,
}
impl Recorded {
    /// Unknown and not acknowledged: never evicted, and counted against
    /// `MAX_UNRESOLVED_OUTCOMES_PER_OWNER`.
    fn unresolved(&self) -> bool {
        !self.acknowledged
            && match &self.facts {
                Facts::Lease(end) => end.outcome == Outcome::Unknown,
                Facts::Job(_) | Facts::Damage(_) => true,
            }
    }
    /// Resolved or acknowledged: subject to the retention bounds.
    fn evictable(&self) -> bool {
        !self.unresolved()
    }
    fn dto(&self) -> UnresolvedOutcome {
        let (transaction, job, journal_damage) = match &self.facts {
            Facts::Lease(end) => (Some((*end).into()), None, None),
            Facts::Job(job) => (None, Some(job.clone()), None),
            Facts::Damage(damage) => (None, None, Some(damage.clone())),
        };
        UnresolvedOutcome {
            outcome_id: self.id.to_string(),
            engine: self.origin.engine.clone(),
            database: self.origin.database.clone(),
            seconds_ago: u32::try_from(journal::now().saturating_sub(self.at)).unwrap_or(u32::MAX),
            restored: self.restored,
            app_exited: self.exited,
            transaction,
            job,
            journal_damage,
        }
    }
    /// The journal entry of an unresolved record.
    fn entry(&self) -> Entry {
        let outcome = self.dto();
        Entry {
            id: outcome.outcome_id,
            recorded_at: self.at,
            engine: outcome.engine,
            database: outcome.database,
            in_flight: self.exited,
            transaction: outcome.transaction,
            job: outcome.job,
            damage: outcome.journal_damage,
        }
    }
    /// A journal entry restored at startup, owned by the host's owner of this run.
    fn restore(entry: &Entry, owner: OwnerId) -> Option<Self> {
        let facts = match (&entry.transaction, &entry.job, &entry.damage) {
            (Some(end), None, None) => Facts::Lease(TransactionEnd::restored(end)),
            (None, Some(job), None) => Facts::Job(job.clone()),
            (None, None, Some(damage)) => Facts::Damage(damage.clone()),
            _ => return None,
        };
        Some(Self {
            id: Uuid::parse_str(&entry.id).ok()?,
            owner,
            origin: Origin {
                engine: entry.engine.clone(),
                database: entry.database.clone(),
            },
            at: entry.recorded_at,
            facts,
            acknowledged: false,
            restored: true,
            exited: entry.in_flight,
        })
    }
}
/// Evicts the oldest resolved or acknowledged records beyond the per-owner and overall bounds.
/// Unresolved records are never evicted.
fn trim(ended: &mut Vec<Recorded>, owner: OwnerId) {
    let owned = |record: &Recorded| record.owner == owner && record.evictable();
    while ended.iter().filter(|record| owned(record)).count()
        > ExecutionLimits::MAX_ENDED_LEASES_PER_OWNER
    {
        let oldest = ended
            .iter()
            .position(owned)
            .expect("owner has an evictable record");
        ended.remove(oldest);
    }
    while ended.iter().filter(|record| record.evictable()).count()
        > ExecutionLimits::MAX_ENDED_LEASES
    {
        let oldest = ended
            .iter()
            .position(Recorded::evictable)
            .expect("an evictable record exists");
        ended.remove(oldest);
    }
}
/// One reserved per-owner lease slot; released on drop unless the lease was registered.
struct Reservation<'a> {
    state: &'a ExecutionState,
    owner: OwnerId,
}
impl Reservation<'_> {
    /// Registers the lease and releases the reservation under one `leases` lock, so the
    /// per-owner count never sees both or neither.
    fn register(self, id: LeaseId, lease: Arc<Lease>) {
        let (state, owner) = (self.state, self.owner);
        std::mem::forget(self);
        let mut leases = state.leases.lock().expect("lease lock");
        leases.insert(id, lease);
        state.unreserve(owner);
    }
}
impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        let _leases = self.state.leases.lock().expect("lease lock");
        self.state.unreserve(self.owner);
    }
}
impl ExecutionState {
    pub(crate) fn new() -> Self {
        Self {
            jobs: Mutex::new(HashMap::new()),
            leases: Mutex::new(HashMap::new()),
            opening: Mutex::new(HashMap::new()),
            ended: Mutex::new(Vec::new()),
            detached: Mutex::new(Vec::new()),
            journal: Mutex::new(None),
        }
    }
    /// Writes (`Some`) or removes (`None`) one journal entry durably. True when durable, or when
    /// this registry has no journal (volatile). Callers holding `ended` lock it first.
    fn persist(&self, id: Uuid, entry: Option<Entry>) -> bool {
        match self.journal.lock().expect("journal lock").as_mut() {
            None => true,
            Some(journal) => match entry {
                Some(entry) => journal.put(entry),
                None => journal.remove(&id.to_string()),
            },
        }
    }
    /// Removes the journal entry of work whose outcome is now known (resolved, or nothing was
    /// sent). Unlike an acknowledgement this is never undone by a failed write: it completes
    /// with the next successful write.
    fn forget(&self, id: Uuid) -> bool {
        self.journal
            .lock()
            .expect("journal lock")
            .as_mut()
            .is_none_or(|journal| journal.conclude(&id.to_string()))
    }
    /// Write-ahead record of work about to be sent, so a crash before it concludes is restored
    /// as `unknown` (the app exited) with these facts.
    fn journal_in_flight(
        &self,
        id: Uuid,
        origin: &Origin,
        transaction: Option<TransactionEnd>,
        job: Option<UnresolvedJob>,
    ) -> bool {
        self.persist(
            id,
            Some(Entry {
                id: id.to_string(),
                recorded_at: journal::now(),
                engine: origin.engine.clone(),
                database: origin.database.clone(),
                in_flight: true,
                transaction: transaction.map(Into::into),
                job,
                damage: None,
            }),
        )
    }
    /// An in-flight job: statement `index` of `count` is about to be sent.
    fn journal_job(
        &self,
        id: JobId,
        origin: &Origin,
        count: usize,
        index: usize,
        lease: bool,
    ) -> bool {
        self.journal_in_flight(
            id.0,
            origin,
            None,
            Some(UnresolvedJob {
                statement_count: count as u32,
                unknown_statement: Some(index as u32),
                in_transaction: lease,
                cancellation: CancellationDto::None,
                server_cancel_sent: false,
                deadline_exceeded: false,
            }),
        )
    }
    /// Concluded without an unknown outcome (or nothing was sent): no longer journaled.
    fn concluded(&self, id: Uuid) -> bool {
        self.forget(id)
    }
    /// Durability required: until a journal is opened, new read-write work is refused.
    pub(crate) fn require_journal(&self) {
        self.journal
            .lock()
            .expect("journal lock")
            .get_or_insert_with(Journal::unopened);
    }
    /// Opens the journal and restores its entries as unresolved outcomes of `owner` (marked as
    /// recorded before a restart). Restored outcomes count against the unresolved cap.
    pub(crate) fn open_journal(&self, dir: &Path, owner: OwnerId) -> Option<JournalProblemDto> {
        let journal = Journal::open(dir);
        let problem = journal.problem();
        let mut ended = self.ended.lock().expect("ended lease lock");
        let mut restored: Vec<_> = journal
            .entries()
            .iter()
            .filter_map(|entry| Recorded::restore(entry, owner))
            .collect();
        restored.sort_by_key(|record| record.at);
        ended.splice(0..0, restored);
        *self.journal.lock().expect("journal lock") = Some(journal);
        problem
    }
    /// Whether a durable record can be written now (always for a volatile registry). After a
    /// failed write the snapshot is retried first, so a freed disk recovers without a restart.
    fn journal_ready(&self) -> bool {
        self.journal
            .lock()
            .expect("journal lock")
            .as_mut()
            .is_none_or(Journal::ready)
    }
    fn journal_problem(&self) -> Option<JournalProblemDto> {
        self.journal
            .lock()
            .expect("journal lock")
            .as_ref()
            .and_then(Journal::problem)
    }
    /// Reserves one of the owner's `MAX_LEASES_PER_OWNER` slots before any await (L-4).
    fn reserve(&self, owner: OwnerId) -> Result<Reservation<'_>, ExecutionError> {
        let leases = self.leases.lock().expect("lease lock");
        let mut opening = self.opening.lock().expect("lease opening lock");
        let open = leases.values().filter(|lease| lease.owner == owner).count();
        let pending = opening.entry(owner).or_insert(0);
        if open + *pending >= ExecutionLimits::MAX_LEASES_PER_OWNER {
            if *pending == 0 {
                opening.remove(&owner);
            }
            return Err(ExecutionError::Capacity);
        }
        *pending += 1;
        Ok(Reservation { state: self, owner })
    }
    /// Callers hold the `leases` lock.
    fn unreserve(&self, owner: OwnerId) {
        let mut opening = self.opening.lock().expect("lease opening lock");
        if let Some(pending) = opening.get_mut(&owner) {
            *pending -= 1;
            if *pending == 0 {
                opening.remove(&owner);
            }
        }
    }
    /// Inserts or replaces the recorded end of a lease (an unknown outcome is recorded first
    /// and replaced only by a server-confirmed one); a replaced record becomes the newest.
    /// Each owner keeps its newest `MAX_ENDED_LEASES_PER_OWNER` resolved or acknowledged ends,
    /// so another owner's ends never evict them; all owners together keep `MAX_ENDED_LEASES`.
    /// An unacknowledged unknown end is never evicted.
    ///
    /// Returns whether the record is durable: an unresolved one is journaled (written before
    /// it is visible in memory), a resolved or acknowledged one removed from the journal.
    fn record_end(
        &self,
        id: LeaseId,
        owner: OwnerId,
        origin: impl Into<Origin>,
        end: TransactionEnd,
    ) -> bool {
        let mut ended = self.ended.lock().expect("ended lease lock");
        let mut acknowledged = false;
        if let Some(index) = ended
            .iter()
            .position(|record| record.id == id.0 && record.owner == owner)
        {
            let previous = ended.remove(index);
            // A changed unknown (for example COMMIT now attempted) needs a new acknowledgement.
            acknowledged = previous.acknowledged
                && (end.outcome != Outcome::Unknown
                    || matches!(previous.facts, Facts::Lease(before) if before == end));
        }
        let record = Recorded {
            id: id.0,
            owner,
            origin: origin.into(),
            at: journal::now(),
            facts: Facts::Lease(end),
            acknowledged,
            restored: false,
            exited: false,
        };
        let durable = if record.unresolved() {
            self.persist(id.0, Some(record.entry()))
        } else {
            self.forget(id.0)
        };
        ended.push(record);
        trim(&mut ended, owner);
        durable
    }
    /// Records a job whose outcome is unknown, independently of its job record, so it survives
    /// the record's release (final page, `TERMINAL_RETENTION`, reset).
    /// It replaces the job's in-flight journal entry before it is visible.
    fn record_job(&self, id: JobId, owner: OwnerId, origin: impl Into<Origin>, job: UnresolvedJob) {
        let mut ended = self.ended.lock().expect("ended lease lock");
        let record = Recorded {
            id: id.0,
            owner,
            origin: origin.into(),
            at: journal::now(),
            facts: Facts::Job(job),
            acknowledged: false,
            restored: false,
            exited: false,
        };
        self.persist(id.0, Some(record.entry()));
        ended.push(record);
        trim(&mut ended, owner);
    }
    fn recorded(&self, id: LeaseId, owner: OwnerId) -> Option<TransactionEnd> {
        self.ended
            .lock()
            .expect("ended lease lock")
            .iter()
            .find(|record| record.id == id.0 && record.owner == owner)
            .and_then(|record| match record.facts {
                Facts::Lease(end) => Some(end),
                Facts::Job(_) | Facts::Damage(_) => None,
            })
    }
    /// Refuses new read-write work while the journal cannot be written (fail closed), while a
    /// damaged journal awaits acknowledgement, or while the owner has
    /// `MAX_UNRESOLVED_OUTCOMES_PER_OWNER` unacknowledged unknown outcomes (restored ones
    /// included). Nothing is evicted to make room.
    fn admit(&self, owner: OwnerId) -> Result<(), ExecutionError> {
        if !self.journal_ready() {
            return Err(ExecutionError::JournalUnavailable);
        }
        let ended = self.ended.lock().expect("ended lease lock");
        let mut unresolved = ended
            .iter()
            .filter(|record| record.owner == owner && record.unresolved());
        if unresolved
            .clone()
            .any(|record| matches!(record.facts, Facts::Damage(_)))
        {
            return Err(ExecutionError::JournalUnavailable);
        }
        if unresolved.by_ref().count() >= ExecutionLimits::MAX_UNRESOLVED_OUTCOMES_PER_OWNER {
            return Err(ExecutionError::UnresolvedOutcomes);
        }
        Ok(())
    }
    fn unresolved(&self, owner: OwnerId) -> UnresolvedOutcomes {
        // Counted before the outcomes are read: a job records its unknown before it is marked
        // finished, so `pending == 0` here means every detached job's outcome is in the list.
        let pending = {
            let mut detached = self.detached.lock().expect("detached job lock");
            detached.retain(|job| !job.is_finished());
            detached.iter().filter(|job| job.owner == owner).count() as u32
        };
        UnresolvedOutcomes {
            pending,
            outcomes: self
                .ended
                .lock()
                .expect("ended lease lock")
                .iter()
                .filter(|record| record.owner == owner && record.unresolved())
                .map(Recorded::dto)
                .collect(),
            limit: ExecutionLimits::MAX_UNRESOLVED_OUTCOMES_PER_OWNER as u32,
            journal: self.journal_problem(),
        }
    }
    /// Marks an owned record acknowledged (idempotent). Never changes its outcome. It becomes
    /// the newest evictable record, so it stays queryable until newer outcomes push it out
    /// (rather than being evicted at once as the oldest). The acknowledgement is durable
    /// (removed from the journal) before it is reported; if it cannot be, nothing changes.
    fn acknowledge(&self, owner: OwnerId, id: OutcomeId) -> Result<(), ExecutionError> {
        let mut ended = self.ended.lock().expect("ended lease lock");
        let index = ended
            .iter()
            .position(|record| record.id == id.0 && record.owner == owner)
            .ok_or(ExecutionError::Forbidden)?;
        if !ended[index].acknowledged && !self.persist(id.0, None) {
            return Err(ExecutionError::JournalUnavailable);
        }
        let mut record = ended.remove(index);
        record.acknowledged = true;
        ended.push(record);
        trim(&mut ended, owner);
        Ok(())
    }
    /// Removes a lease from the open set, recording `unknown` for it first so no caller can
    /// observe it as neither open nor ended. Returns the lease only to the caller that removed
    /// it; that caller alone concludes it.
    fn take_lease(&self, id: LeaseId, reason: EndReason) -> Option<Arc<Lease>> {
        let mut leases = self.leases.lock().expect("lease lock");
        let lease = leases.get(&id)?;
        self.record_end(
            id,
            lease.owner,
            &lease.origin,
            TransactionEnd::unknown(reason),
        );
        leases.remove(&id)
    }
    /// Releases every job record (renderer reset): their pages have no reader any more. Unknown
    /// outcomes are recorded separately and stay listable; jobs still running are kept as
    /// detached until they finish, so the list reports them as pending.
    pub(crate) fn forget_jobs(&self) {
        let released = std::mem::take(&mut *self.jobs.lock().expect("job lock"));
        let mut detached = self.detached.lock().expect("detached job lock");
        detached.retain(|job| !job.is_finished());
        detached.extend(
            released
                .into_values()
                .map(|entry| entry.job)
                .filter(|job| !job.is_finished()),
        );
    }
    fn prune(&self) {
        self.jobs.lock().expect("job lock").retain(|_, entry| {
            entry
                .job
                .progress
                .lock()
                .expect("job progress lock")
                .finished_at
                .is_none_or(|at| at.elapsed() < ExecutionLimits::TERMINAL_RETENTION)
        });
    }
}

/// Interrupts jobs and ends leases of one handle (or all handles) before its pool and tunnel
/// close, within `RELEASE_BOUND`. Lease outcomes are recorded as confirmed or unknown.
pub(crate) async fn release(inner: &Inner, handle: Option<ConnectionId>) {
    release_within(inner, handle, ExecutionLimits::RELEASE_BOUND).await;
}
async fn release_within(inner: &Inner, handle: Option<ConnectionId>, bound: Duration) {
    let matches = |candidate: ConnectionId| handle.is_none_or(|handle| handle == candidate);
    let jobs: Vec<_> = inner
        .execution
        .jobs
        .lock()
        .expect("job lock")
        .values()
        .filter(|entry| matches(entry.job.handle))
        .map(|entry| entry.job.clone())
        .collect();
    // Each lease is recorded `unknown`/`closed` as it is removed, before anything is awaited:
    // a ROLLBACK cut off by `RELEASE_BOUND` leaves that record, never a missing outcome.
    let ids: Vec<_> = inner
        .execution
        .leases
        .lock()
        .expect("lease lock")
        .iter()
        .filter(|(_, lease)| matches(lease.handle))
        .map(|(id, _)| *id)
        .collect();
    let leases: Vec<_> = ids
        .into_iter()
        .filter_map(|id| {
            inner
                .execution
                .take_lease(id, EndReason::Closed)
                .map(|lease| (id, lease))
        })
        .collect();
    for job in &jobs {
        job.control.trip(Reason::Closed);
    }
    let work = async {
        futures_util::future::join_all(jobs.iter().map(|job| job.wait_finished())).await;
        let ends = leases.into_iter().map(|(id, lease)| async move {
            let physical = lease.slot.clone().lock_owned().await.take();
            if let Some(physical) = physical {
                physical
                    .conclude(false, EndReason::Closed, lease.diverged(), |end| {
                        inner
                            .execution
                            .record_end(id, lease.owner, &lease.origin, end)
                    })
                    .await;
            }
        });
        futures_util::future::join_all(ends).await;
    };
    let _ = tokio::time::timeout(bound, work).await;
}

/// A lease whose connection slot is held by the running job.
type HeldLease = (LeaseId, Arc<Lease>, OwnedMutexGuard<Option<Physical>>);
struct Admitted {
    pool: EnginePool,
    engine: Engine,
    lease: Option<HeldLease>,
    permit: Option<OwnedSemaphorePermit>,
}
struct Plan {
    id: JobId,
    job: Arc<Job>,
    statements: Vec<Statement>,
    target: Target,
    origin: Origin,
    statement_deadline: Duration,
    lease: Option<HeldLease>,
    /// The handle's exclusive work permit for a job outside a lease. Declared before `sink`
    /// so it is released before the page channel closes (`done`).
    _permit: Option<OwnedSemaphorePermit>,
    connect_deadline: Duration,
    sink: Sink,
    registry: Weak<Inner>,
}

impl Connections {
    fn writable(
        &self,
        owner: OwnerId,
        handle: ConnectionId,
    ) -> Result<(EnginePool, Arc<Semaphore>, Engine), ExecutionError> {
        let entries = self.0.entries.lock().expect("connection registry lock");
        let entry = entries
            .get(&handle)
            .filter(|entry| entry.usable(owner))
            .ok_or(ExecutionError::Forbidden)?;
        entry.touch();
        if entry.access != Access::ReadWrite {
            return Err(ExecutionError::ReadOnlyHandle);
        }
        Ok((entry.pool.clone(), entry.work.clone(), entry.engine))
    }

    /// Starts a read-write script now, or refuses: a handle running a job or holding a
    /// transaction (unless the job targets that lease) returns `Busy`. Nothing is queued.
    /// Policy refusals happen here, before any statement is sent.
    pub fn submit_job(
        &self,
        owner: OwnerId,
        handle: ConnectionId,
        options: JobOptions,
    ) -> Result<(JobId, usize), ExecutionError> {
        if !(1..=ExecutionLimits::MAX_ROWS_PER_RESULT).contains(&options.max_rows_per_result)
            || options.statement_deadline.is_zero()
            || options.statement_deadline > ExecutionLimits::MAX_STATEMENT_DEADLINE
        {
            return Err(ExecutionError::InvalidLimits);
        }
        let dialect = self.writable(owner, handle)?.0.dialect();
        let statements = script::split(&options.sql, dialect)
            .map_err(|error| ExecutionError::Script(0, error))?;
        script::check(&statements, dialect, options.lease.is_some())
            .map_err(|(index, error)| ExecutionError::Script(index, error))?;
        self.start_job(owner, handle, statements, options)
    }
    /// Starts already split and policy-checked statements (tests use it to model a splitter miss).
    fn start_job(
        &self,
        owner: OwnerId,
        handle: ConnectionId,
        statements: Vec<Statement>,
        options: JobOptions,
    ) -> Result<(JobId, usize), ExecutionError> {
        let Admitted {
            pool,
            engine,
            lease,
            permit,
        } = self.admit_work(owner, handle, options.lease)?;
        let count = statements.len();
        let (tx, rx) = mpsc::channel(ExecutionLimits::BUFFERED_PAGES);
        let job = Job::new(
            owner,
            handle,
            ExecutionLimits::JOB_DEADLINE,
            (0..count)
                .map(|index| StatementReport {
                    index: index as u32,
                    status: StatementStatusDto::NotRun,
                    in_transaction: lease.is_some(),
                    affected_rows: "0".into(),
                    result_sets: 0,
                    failure: None,
                    results_withheld: false,
                })
                .collect(),
            lease.as_ref().map(|_| LeaseStateDto::Active),
            None,
        );
        let id = JobId(Uuid::new_v4());
        let target = Target::of(&pool);
        let origin = Origin::of(engine, &target);
        self.register_job(owner, handle, id, &job, &origin, count, lease.is_some(), rx)?;
        tokio::spawn(run(Plan {
            id,
            job,
            statements,
            target,
            origin,
            statement_deadline: options.statement_deadline,
            lease,
            _permit: permit,
            connect_deadline: self.0.deadline,
            sink: Sink::new(tx, options.max_rows_per_result),
            registry: Arc::downgrade(&self.0),
        }));
        Ok((id, count))
    }

    /// Admission shared by jobs and SQL imports: a usable read-write handle, the journal and
    /// unresolved-outcome gates, the per-owner record cap, and the handle's exclusive work
    /// (its lease's connection slot, or its work permit). Nothing is queued.
    fn admit_work(
        &self,
        owner: OwnerId,
        handle: ConnectionId,
        lease: Option<LeaseId>,
    ) -> Result<Admitted, ExecutionError> {
        let (pool, work, engine) = self.writable(owner, handle)?;
        let execution = &self.0.execution;
        execution.admit(owner)?;
        execution.prune();
        if execution
            .jobs
            .lock()
            .expect("job lock")
            .values()
            .filter(|entry| entry.job.owner == owner)
            .count()
            >= ExecutionLimits::MAX_JOB_RECORDS_PER_OWNER
        {
            return Err(ExecutionError::Capacity);
        }
        let (lease, permit) = match lease {
            Some(id) => {
                let lease = execution
                    .leases
                    .lock()
                    .expect("lease lock")
                    .get(&id)
                    .filter(|lease| lease.owner == owner && lease.handle == handle)
                    .cloned()
                    .ok_or(ExecutionError::Forbidden)?;
                let guard = lease
                    .slot
                    .clone()
                    .try_lock_owned()
                    .map_err(|_| ExecutionError::Busy)?;
                if guard.is_none() {
                    return Err(ExecutionError::Busy);
                }
                if lease.state.lock().expect("lease lock").rollback_only {
                    return Err(ExecutionError::RollbackOnly);
                }
                lease.touch();
                (Some((id, lease, guard)), None)
            }
            None => (
                None,
                Some(work.try_acquire_owned().map_err(|_| ExecutionError::Busy)?),
            ),
        };
        Ok(Admitted {
            pool,
            engine,
            lease,
            permit,
        })
    }

    /// Makes a started job visible: statement 0's in-flight record is durable first
    /// (write-ahead), then the record is inserted. A disconnect/expiry removes the handle before
    /// it releases the handle's jobs; if that raced this submission the job is ended here
    /// (nothing sent) instead of running untracked.
    #[allow(clippy::too_many_arguments)]
    fn register_job(
        &self,
        owner: OwnerId,
        handle: ConnectionId,
        id: JobId,
        job: &Arc<Job>,
        origin: &Origin,
        count: usize,
        in_lease: bool,
        pages: mpsc::Receiver<ResultBatch>,
    ) -> Result<(), ExecutionError> {
        let execution = &self.0.execution;
        if !execution.journal_job(id, origin, count, 0, in_lease) {
            execution.concluded(id.0);
            return Err(ExecutionError::JournalUnavailable);
        }
        execution.jobs.lock().expect("job lock").insert(
            id,
            JobEntry {
                job: job.clone(),
                pages: Arc::new(tokio::sync::Mutex::new(pages)),
            },
        );
        if self.writable(owner, handle).is_err() {
            finish(job, ExecutionOutcome::Cancelled, true, None);
            execution.jobs.lock().expect("job lock").remove(&id);
            execution.concluded(id.0);
            return Err(ExecutionError::Forbidden);
        }
        Ok(())
    }

    /// Next bounded page and a progress snapshot. `done` is set once the job finished and every
    /// page was delivered; the job record is then released.
    pub async fn fetch_job(
        &self,
        owner: OwnerId,
        id: JobId,
        wait: Duration,
    ) -> Result<JobUpdate, ExecutionError> {
        self.0.execution.prune();
        let (job, pages) = self
            .0
            .execution
            .jobs
            .lock()
            .expect("job lock")
            .get(&id)
            .filter(|entry| entry.job.owner == owner)
            .map(|entry| (entry.job.clone(), entry.pages.clone()))
            .ok_or(ExecutionError::Forbidden)?;
        let mut pages = pages.try_lock_owned().map_err(|_| ExecutionError::Busy)?;
        let wait = wait.min(ExecutionLimits::FETCH_WAIT_MAX);
        let (batch, closed) = if wait.is_zero() {
            match pages.try_recv() {
                Ok(batch) => (Some(batch), false),
                Err(mpsc::error::TryRecvError::Empty) => (None, false),
                Err(mpsc::error::TryRecvError::Disconnected) => (None, true),
            }
        } else {
            match tokio::time::timeout(wait, pages.recv()).await {
                Ok(Some(batch)) => (Some(batch), false),
                Ok(None) => (None, true),
                Err(_) => (None, false),
            }
        };
        let reason = job.control.reason();
        let update = {
            let progress = job.progress.lock().expect("job progress lock");
            JobUpdate {
                job_id: id.opaque(),
                state: progress.state,
                outcome: progress.outcome.map(Into::into),
                cancellation: cancellation(reason, progress.stop_confirmed),
                server_cancel_sent: progress.server_cancel_sent,
                deadline_exceeded: reason == Some(Reason::Deadline) || progress.server_timeout,
                statements: progress.statements.clone(),
                batch,
                lease: progress.lease,
                done: closed,
            }
        };
        if closed {
            self.0.execution.jobs.lock().expect("job lock").remove(&id);
        }
        Ok(update)
    }

    /// Records a cancel request. Whether the work stopped, and what happened to its effects,
    /// is reported by `fetch_job`; it is never assumed here.
    pub fn cancel_job(&self, owner: OwnerId, id: JobId) -> Result<bool, ExecutionError> {
        let job = self
            .0
            .execution
            .jobs
            .lock()
            .expect("job lock")
            .get(&id)
            .filter(|entry| entry.job.owner == owner)
            .map(|entry| entry.job.clone())
            .ok_or(ExecutionError::Forbidden)?;
        if job.is_finished() {
            return Ok(false);
        }
        job.control.trip(Reason::Cancel);
        Ok(true)
    }

    /// Opens a transaction lease on a dedicated, never-pooled physical connection. The lease
    /// holds the handle's exclusive work permit, so other work on the handle gets `Busy` until
    /// it ends (commit, rollback, `LEASE_IDLE`, `LEASE_MAX_LIFETIME`, close).
    pub async fn begin_transaction(
        &self,
        owner: OwnerId,
        handle: ConnectionId,
    ) -> Result<LeaseId, ExecutionError> {
        let (pool, work, engine) = self.writable(owner, handle)?;
        let execution = &self.0.execution;
        execution.admit(owner)?;
        let reservation = execution.reserve(owner)?;
        let permit = work.try_acquire_owned().map_err(|_| ExecutionError::Busy)?;
        let id = LeaseId(Uuid::new_v4());
        let target = Target::of(&pool);
        let origin = Origin::of(engine, &target);
        // Write-ahead: an open lease is journaled before BEGIN, so a crash while it is open is
        // restored as `unknown` (closed by the app's exit, COMMIT never sent).
        let open = TransactionEnd::unknown(EndReason::Closed);
        if !execution.journal_in_flight(id.0, &origin, Some(open), None) {
            execution.concluded(id.0);
            return Err(ExecutionError::JournalUnavailable);
        }
        let opened = async {
            let mut physical = tokio::time::timeout(self.0.deadline, target.open())
                .await
                .map_err(|_| ExecutionError::Connection(LifecycleError::Timeout))?
                .map_err(ExecutionError::Connection)?;
            if !matches!(
                tokio::time::timeout(self.0.deadline, physical.begin()).await,
                Ok(Ok(()))
            ) {
                physical.close().await;
                return Err(ExecutionError::Database);
            }
            Ok(physical)
        }
        .await;
        let physical = match opened {
            Ok(physical) => physical,
            Err(error) => {
                execution.concluded(id.0);
                return Err(error);
            }
        };
        let lease = Arc::new(Lease {
            owner,
            handle,
            origin,
            created: Instant::now(),
            slot: Arc::new(tokio::sync::Mutex::new(Some(physical))),
            state: Mutex::new(LeaseState {
                rollback_only: false,
                diverged: false,
                last_used: Instant::now(),
            }),
            _permit: permit,
        });
        // Registered before the handle is re-checked: a disconnect/reset that raced this setup
        // either finds the lease (and ends it) or is seen here; the new lease is then rolled back.
        reservation.register(id, lease.clone());
        if self.writable(owner, handle).is_err() {
            let removed = self
                .0
                .execution
                .leases
                .lock()
                .expect("lease lock")
                .remove(&id);
            if removed.is_some()
                && let Some(physical) = lease.slot.lock().await.take()
            {
                // The renderer never learned this lease; only BEGIN was sent on it.
                physical
                    .conclude(false, EndReason::Closed, false, |_| true)
                    .await;
                self.0.execution.concluded(id.0);
            }
            return Err(ExecutionError::Forbidden);
        }
        // The watcher holds a weak reference so an ended lease releases the handle at once.
        tokio::spawn(expire(Arc::downgrade(&self.0), id, Arc::downgrade(&lease)));
        Ok(id)
    }

    /// Commits or rolls back an owned lease and discards its connection. A lease that already
    /// ended (requested, expiry, connection loss, close) reports its recorded outcome on every
    /// later call (`unknown` while an end is still in progress); nothing is sent again.
    pub async fn end_transaction(
        &self,
        owner: OwnerId,
        id: LeaseId,
        commit: bool,
    ) -> Result<TransactionEnd, ExecutionError> {
        let execution = &self.0.execution;
        if let Some(end) = execution.recorded(id, owner) {
            return Ok(end);
        }
        let lease = execution
            .leases
            .lock()
            .expect("lease lock")
            .get(&id)
            .filter(|lease| lease.owner == owner)
            .cloned();
        let Some(lease) = lease else {
            // It may have ended between the two lookups.
            return execution
                .recorded(id, owner)
                .ok_or(ExecutionError::Forbidden);
        };
        let Ok(mut slot) = lease.slot.clone().try_lock_owned() else {
            return execution.recorded(id, owner).ok_or(ExecutionError::Busy);
        };
        if commit && lease.state.lock().expect("lease lock").rollback_only {
            return Err(ExecutionError::RollbackOnly);
        }
        // COMMIT needs a durable unknown record first; refused here (lease left open, nothing
        // sent) while the journal cannot be written. Rollback is always allowed.
        if commit && !execution.journal_ready() {
            return Err(ExecutionError::JournalUnavailable);
        }
        if execution.take_lease(id, EndReason::Requested).is_none() {
            return execution
                .recorded(id, owner)
                .ok_or(ExecutionError::Forbidden);
        }
        Ok(match slot.take() {
            Some(physical) => {
                physical
                    .conclude(commit, EndReason::Requested, lease.diverged(), |end| {
                        execution.record_end(id, owner, &lease.origin, end)
                    })
                    .await
            }
            None => TransactionEnd::unknown(EndReason::Requested),
        })
    }

    /// Durability required (desktop host): until [`Connections::open_outcome_journal`]
    /// succeeds, new jobs and transactions are refused with `JournalUnavailable`.
    pub fn require_outcome_journal(&self) {
        self.0.execution.require_journal();
    }
    /// Opens the durable unknown-outcome journal in `dir` (a private per-user directory the host
    /// chooses; created 0700) and restores its records as unresolved outcomes of `owner`,
    /// marked as recorded before a restart. Call once per registry. Returns the problem that
    /// keeps new read-write work refused, if any.
    pub fn open_outcome_journal(&self, dir: &Path, owner: OwnerId) -> Option<JournalProblemDto> {
        self.0.execution.open_journal(dir, owner)
    }
    /// The owner's unknown outcomes not yet acknowledged, oldest first, including those of a
    /// renderer session that ended (reload) in this process and, with a journal, those
    /// restored from earlier runs (before a restart).
    pub fn unresolved_outcomes(&self, owner: OwnerId) -> UnresolvedOutcomes {
        self.0.execution.unresolved(owner)
    }
    /// Hides one of the owner's recorded outcomes after the user acknowledged it, returning the
    /// remaining unresolved ones. Idempotent while the record exists; the recorded outcome
    /// never changes (unknown is never turned into success or rollback). An unknown id or
    /// another owner's gives `Forbidden`.
    pub fn acknowledge_outcome(
        &self,
        owner: OwnerId,
        id: OutcomeId,
    ) -> Result<UnresolvedOutcomes, ExecutionError> {
        self.0.execution.acknowledge(owner, id)?;
        Ok(self.0.execution.unresolved(owner))
    }
    /// Whether the owner holds an open lease or a job record (running, or finished with its
    /// final update not yet fetched). `reset` releases both, so a renderer rotation clears it.
    pub fn has_active_work(&self, owner: OwnerId) -> bool {
        let execution = &self.0.execution;
        execution
            .leases
            .lock()
            .expect("lease lock")
            .values()
            .any(|lease| lease.owner == owner)
            || execution
                .jobs
                .lock()
                .expect("job lock")
                .values()
                .any(|entry| entry.job.owner == owner)
    }
    /// Open leases, for lifecycle evidence only.
    pub fn open_transactions(&self) -> usize {
        self.0.execution.leases.lock().expect("lease lock").len()
    }
    /// Job records not yet released, for lifecycle evidence only.
    pub fn retained_jobs(&self) -> usize {
        self.0.execution.jobs.lock().expect("job lock").len()
    }
    /// Read-write handles serialize all work; read-only handles keep M003 behaviour.
    pub(crate) fn exclusive_read(
        &self,
        owner: OwnerId,
        id: ConnectionId,
    ) -> Result<Option<OwnedSemaphorePermit>, ReadError> {
        let entries = self.0.entries.lock().expect("connection registry lock");
        let Some(entry) = entries.get(&id).filter(|entry| entry.usable(owner)) else {
            return Err(ReadError::Connection(LifecycleError::Forbidden));
        };
        if entry.access == Access::ReadOnly {
            return Ok(None);
        }
        entry
            .work
            .clone()
            .try_acquire_owned()
            .map(Some)
            .map_err(|_| ReadError::Busy)
    }
}

async fn expire(registry: Weak<Inner>, id: LeaseId, lease: Weak<Lease>) {
    loop {
        let Some(due) = lease.upgrade().map(|lease| lease.due()) else {
            return;
        };
        tokio::time::sleep_until(due).await;
        let (Some(inner), Some(lease)) = (registry.upgrade(), lease.upgrade()) else {
            return;
        };
        let current = inner
            .execution
            .leases
            .lock()
            .expect("lease lock")
            .get(&id)
            .is_some_and(|other| Arc::ptr_eq(other, &lease));
        if !current {
            return;
        }
        if lease.due() > Instant::now() {
            continue;
        }
        let Ok(mut slot) = lease.slot.clone().try_lock_owned() else {
            // A running job holds the lease; its statement/job deadlines bound the wait.
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        };
        if inner.execution.take_lease(id, EndReason::Expired).is_none() {
            return;
        }
        if let Some(physical) = slot.take() {
            physical
                .conclude(false, EndReason::Expired, lease.diverged(), |end| {
                    inner
                        .execution
                        .record_end(id, lease.owner, &lease.origin, end)
                })
                .await;
        }
        return;
    }
}

/// The job worker. Statements run in order and stop at the first that does not succeed or at
/// an interrupt. Outcomes follow the M006 taxonomy: `Cancelled` only when nothing was in
/// flight; an interrupted statement is `Unknown` even if its end was observed.
async fn run(mut plan: Plan) {
    let job = plan.job.clone();
    let control = &job.control;
    let mut lease_guard = None;
    let physical = match plan.lease.take() {
        Some((id, lease, mut guard)) => {
            let physical = guard.take();
            lease_guard = Some((id, lease, guard));
            physical
        }
        None => {
            let open = tokio::time::timeout(plan.connect_deadline, plan.target.open());
            // A job interrupted before it starts (cancel, close) never opens a connection.
            tokio::select! {
                biased;
                _ = control.interrupted() => None,
                opened = open => opened.ok().and_then(Result::ok),
            }
        }
    };
    let Some(mut connection) = physical else {
        // Nothing was sent: an interrupt is a confirmed stop, otherwise a confirmed failure.
        let outcome = if control.is_interrupted() {
            ExecutionOutcome::Cancelled
        } else {
            ExecutionOutcome::Failed
        };
        let lease_state = lease_guard.is_some().then_some(LeaseStateDto::Ended);
        end_lease_lost(&plan.registry, lease_guard.take());
        if let Some(inner) = plan.registry.upgrade() {
            inner.execution.concluded(plan.id.0);
        }
        finish(&job, outcome, true, lease_state);
        return;
    };
    connection.interrupt.rearm();
    let mut outcome = ExecutionOutcome::Succeeded;
    let mut stop_confirmed = true;
    let mut reusable = true;
    // L-3: the PostgreSQL server ends a statement before the client deadline (Contract 5.3).
    let server_timeout = ExecutionLimits::postgres_statement_timeout(plan.statement_deadline);
    if let Session::Postgres(session) = &mut connection.session {
        // A fixed number of milliseconds computed here, never request text.
        let set = sqlx::raw_sql(
            sqlx::AssertSqlSafe(format!(
                "SET statement_timeout = {}",
                server_timeout.as_millis()
            ))
            .into_sql_str(),
        );
        // On any error no user statement is sent and the job fails. Refused by the server: in
        // a lease the block may now be aborted, so the lease becomes rollback-only below.
        // Otherwise (session lost, no reply) the connection is not used again.
        match tokio::time::timeout(plan.connect_deadline, set.execute(&mut *session)).await {
            Ok(Ok(_)) => {}
            Ok(Err(error @ sqlx::Error::Database(_))) if !session_lost(&error) => {
                outcome = ExecutionOutcome::Failed
            }
            _ => {
                outcome = ExecutionOutcome::Failed;
                reusable = false;
            }
        }
    }
    for (index, text) in plan.statements.iter().enumerate() {
        if outcome != ExecutionOutcome::Succeeded {
            break;
        }
        if control.is_interrupted() {
            outcome = ExecutionOutcome::Cancelled;
            break;
        }
        // Write-ahead: the statement about to be sent is journaled first (statement 0 was at
        // submission). If that fails nothing more is sent and the job fails here.
        if index > 0
            && !plan.registry.upgrade().is_some_and(|inner| {
                inner.execution.journal_job(
                    plan.id,
                    &plan.origin,
                    plan.statements.len(),
                    index,
                    lease_guard.is_some(),
                )
            })
        {
            job.update(|progress| {
                progress.statements[index].status = StatementStatusDto::Failed;
                progress.statements[index].failure = Some(StatementFailure {
                    constraint: None,
                    sql_state: None,
                    denied: false,
                });
            });
            outcome = ExecutionOutcome::Failed;
            break;
        }
        control.arm(plan.statement_deadline);
        job.update(|progress| progress.statements[index].status = StatementStatusDto::Running);
        plan.sink.statement = index as u32;
        plan.sink.next_result = 0;
        plan.sink.withheld = false;
        let postgres = matches!(connection.session, Session::Postgres(_));
        let Physical { session, interrupt } = &mut connection;
        // R-1: nothing is sent unless SQLite parses the text as exactly one statement.
        #[cfg(feature = "sqlite")]
        if let Session::Sqlite(sqlite) = session
            && !sqlite_single_statement(sqlite, &text.sql).await
        {
            job.update(|progress| {
                let report = &mut progress.statements[index];
                report.status = StatementStatusDto::Failed;
                report.failure = Some(StatementFailure {
                    constraint: None,
                    sql_state: None,
                    denied: false,
                });
            });
            outcome = ExecutionOutcome::Failed;
            break;
        }
        let sent_at = Instant::now();
        let guarded = guarded(
            statement(session, &text.sql, &mut plan.sink, control),
            control,
            &*interrupt,
        )
        .await;
        let timed_out = postgres
            && !guarded.interrupted
            && matches!(&guarded.result, Some(Err(error)) if statement_timeout_error(error))
            && sent_at.elapsed() >= server_timeout;
        let (status, report_failure, completed) = classify(&guarded);
        if guarded.interrupted {
            stop_confirmed = guarded.result.is_some();
            // L-8: a statement that did not end, or a server interrupt still in flight, leaves
            // a session nothing else may run on (a late KILL/cancel would hit the next command).
            if guarded.result.is_none() || guarded.signal.is_none() {
                reusable = false;
            }
        }
        let withheld = plan.sink.withheld;
        job.update(|progress| {
            let report = &mut progress.statements[index];
            report.status = status;
            report.failure = report_failure;
            report.results_withheld = withheld;
            if let Some((affected, sets)) = completed {
                report.affected_rows = affected.to_string();
                report.result_sets = sets;
            }
            progress.server_cancel_sent |= guarded.signal == Some(true);
            progress.server_timeout |= timed_out;
        });
        match status {
            // Completed despite an interrupt: the next iteration stops before sending more.
            StatementStatusDto::Succeeded => continue,
            StatementStatusDto::Failed => outcome = ExecutionOutcome::Failed,
            _ => {
                outcome = ExecutionOutcome::Unknown;
                if !guarded.interrupted {
                    reusable = false;
                }
            }
        }
        break;
    }
    // R-3: a reported outcome (succeeded, failed, stopped) is kept only if the server session's
    // transaction state still matches the model; otherwise what the statements' effects became
    // is unknown (for example, a transaction left open that the discarded connection loses).
    let in_lease = lease_guard.is_some();
    let mut diverged = false;
    let sent = job
        .progress
        .lock()
        .expect("job progress lock")
        .statements
        .iter()
        .any(|report| report.status != StatementStatusDto::NotRun);
    if !reusable {
        // The session state cannot be checked, so no statement's success can be confirmed.
        if sent {
            outcome = ExecutionOutcome::Unknown;
        }
    } else if outcome != ExecutionOutcome::Unknown
        && !connection
            .state_matches(in_lease, outcome == ExecutionOutcome::Succeeded)
            .await
    {
        outcome = ExecutionOutcome::Unknown;
        diverged = true;
    }
    let mut lease_state = None;
    match lease_guard {
        Some((id, lease, mut guard)) => {
            if reusable {
                let active = outcome == ExecutionOutcome::Succeeded;
                if !active {
                    let mut state = lease.state.lock().expect("lease lock");
                    state.rollback_only = true;
                    state.diverged |= diverged;
                }
                lease_state = Some(if active {
                    LeaseStateDto::Active
                } else {
                    LeaseStateDto::RollbackOnly
                });
                connection.interrupt.rearm();
                *guard = Some(connection);
                lease.touch();
            } else {
                drop(connection);
                lease_state = Some(LeaseStateDto::Ended);
                end_lease_lost(&plan.registry, Some((id, lease, guard)));
            }
        }
        // Outside a lease the connection is always discarded, never pooled or reused.
        None if reusable && !diverged => connection.close().await,
        None => drop(connection),
    }
    // Recorded before the final update is visible, so an unknown is never only in a job
    // record that a reset or `TERMINAL_RETENTION` releases. It replaces the job's in-flight
    // journal entry; any other outcome removes that entry. Without the registry the entry
    // stays (restored after a restart as unknown: shown again rather than lost).
    let registry = plan.registry.upgrade();
    if outcome != ExecutionOutcome::Unknown
        && let Some(inner) = &registry
    {
        inner.execution.concluded(plan.id.0);
    }
    if outcome == ExecutionOutcome::Unknown
        && let Some(inner) = registry
    {
        let reason = control.reason();
        let summary = {
            let progress = job.progress.lock().expect("job progress lock");
            UnresolvedJob {
                statement_count: progress.statements.len() as u32,
                unknown_statement: progress
                    .statements
                    .iter()
                    .find(|report| report.status == StatementStatusDto::Unknown)
                    .map(|report| report.index),
                in_transaction: in_lease,
                cancellation: cancellation(reason, stop_confirmed),
                server_cancel_sent: progress.server_cancel_sent,
                deadline_exceeded: reason == Some(Reason::Deadline) || progress.server_timeout,
            }
        };
        inner
            .execution
            .record_job(plan.id, job.owner, &plan.origin, summary);
    }
    finish(&job, outcome, stop_confirmed, lease_state);
    drop(plan);
}

fn cancellation(reason: Option<Reason>, stop_confirmed: bool) -> CancellationDto {
    match (reason, stop_confirmed) {
        (None, _) => CancellationDto::None,
        (Some(_), false) => CancellationDto::Requested,
        (Some(_), true) => CancellationDto::Confirmed,
    }
}

/// The lease connection is gone with an uncertain transaction: never claim rollback.
fn end_lease_lost(registry: &Weak<Inner>, lease: Option<HeldLease>) {
    let (Some(inner), Some((id, _lease, _guard))) = (registry.upgrade(), lease) else {
        return;
    };
    // The connection was dropped, not rolled back: unknown, neither COMMIT nor ROLLBACK sent.
    inner.execution.take_lease(id, EndReason::ConnectionLost);
}

fn finish(
    job: &Job,
    outcome: ExecutionOutcome,
    stop_confirmed: bool,
    lease: Option<LeaseStateDto>,
) {
    job.update(|progress| {
        progress.state = JobStateDto::Finished;
        progress.outcome = Some(outcome);
        progress.stop_confirmed = stop_confirmed;
        if lease.is_some() {
            progress.lease = lease;
        }
        progress.finished_at = Some(Instant::now());
    });
    job.finished.notify_waiters();
}

mod import;
mod journal;
pub use import::{ImportError, ImportRefusal, ImportRefusalReason};
// The execution suite runs against SQLite fixture files (A5: `sqlite` is a default feature).
// The SSH fixture talks to the Unix-socket tunnel endpoint.
#[cfg(all(test, feature = "sqlite", unix))]
mod ssh_fixture;
#[cfg(all(test, feature = "sqlite"))]
mod tests;
