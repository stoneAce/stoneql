mod database;
mod execution;
mod host_keys;
mod secrets;
mod sql_import;
mod sqlite_file;
mod ssh_transport;
mod transfer;
use database::{
    connect_direct, database_status, disconnect_database, read_only_query, switch_database,
};
use execution::{
    acknowledge_outcome, begin_transaction, cancel_job, end_transaction, fetch_job,
    list_unresolved_outcomes, submit_job,
};
pub use host_keys::{FileHostKeyStore, HostKeyStore, HostKeyStoreError, MemoryHostKeyStore};
pub use secrets::{DEFAULT_SERVICE, OsSecretStore, SecretError, SecretStore, UnlockError};
use secrets::{
    delete_secret, reset_secret_store, secret_store_status, store_secret, unlock_secret_store,
};
pub use sql_import::authorize_import_file;
use sql_import::{pick_import_file, sql_import_status, start_sql_import};
pub use sqlite_file::authorize_sqlite_file;
use sqlite_file::{connect_sqlite, pick_sqlite_file};
pub use ssh_transport::{authorize_ssh_key_file, commit_ssh_host_key};
use ssh_transport::{
    cancel_connect, forget_ssh_host_key, pick_ssh_key_file, ssh_host_key, ssh_host_key_status,
    trust_ssh_host_key,
};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::Instant,
};
use stoneql_contracts::v1::{
    self, CloseAction, CloseDecision, CloseResult, ContractProbe, Envelope, ErrorDto, HostInfo,
    HostInfoRequest, ProbeAction, ProbeResult, ProtocolVersion,
};
use stoneql_core::{CoreError, ErrorCode};
use tauri::{Manager, Runtime, State, WebviewWindow};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
pub use transfer::authorize_export_file;
use transfer::{cancel_export, export_status, pick_export_file, start_export};

/// M006 gate for unrestricted read-write SQL (jobs and transactions). ON since the user approved
/// A15 (2026-11-17) after the execution safety tests. Connections may then request
/// `access: read_write`, except PostgreSQL over SSH (still refused, D025-9). When
/// off, the host refuses read-write before any credential lookup or network I/O. Only trusted
/// host code can change it (tests through [`app_builder_with`]); never renderer input or the
/// environment.
pub const READ_WRITE_SQL_ENABLED: bool = true;

/// Trusted host services injected at startup: the OS secret store, the SSH agent socket, the
/// read-write SQL gate and the unknown-outcome journal directory. Tests substitute a dedicated
/// keychain service, a private agent, a temporary journal directory and, for execution tests
/// only, an enabled gate.
pub struct HostServices {
    pub secrets: Arc<dyn SecretStore>,
    pub agent_socket: Option<PathBuf>,
    pub read_write_sql: bool,
    /// Directory of the durable unknown-outcome journal; `None` means
    /// `<app data dir>/outcome-journal` (macOS: `~/Library/Application Support/<identifier>/`).
    pub outcome_journal: Option<PathBuf>,
    /// Bound on resolving saved secrets (a Keychain prompt) inside a connect attempt (F-02).
    pub secret_deadline: std::time::Duration,
    /// Native SSH host-key trust (F-01). `None` = `<app data dir>/ssh-host-keys.json`, resolved
    /// at the first renderer bootstrap; until then every SSH connect is refused.
    pub host_keys: Option<Arc<dyn HostKeyStore>>,
}
impl HostServices {
    /// Proposed (M005 alpha fix plan T1, awaiting approval): long enough to answer a Keychain
    /// prompt; the renderer waits 75 s when a saved reference is used.
    pub const SECRET_DEADLINE: std::time::Duration = std::time::Duration::from_secs(60);
    /// Production: the OS credential store, the agent named by `SSH_AUTH_SOCK` at startup, the
    /// compiled-in read-write gate and the journal in the app data directory.
    pub fn system() -> Self {
        Self {
            secrets: Arc::new(OsSecretStore::new(DEFAULT_SERVICE)),
            agent_socket: agent_socket(),
            read_write_sql: READ_WRITE_SQL_ENABLED,
            outcome_journal: None,
            secret_deadline: Self::SECRET_DEADLINE,
            host_keys: None,
        }
    }
}
/// The agent named by `SSH_AUTH_SOCK`. On Windows, where it is usually unset, the built-in
/// OpenSSH agent's named pipe.
fn agent_socket() -> Option<PathBuf> {
    std::env::var_os("SSH_AUTH_SOCK")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(default_agent)
}
#[cfg(windows)]
fn default_agent() -> Option<PathBuf> {
    Some(PathBuf::from(r"\\.\pipe\openssh-ssh-agent"))
}
#[cfg(not(windows))]
fn default_agent() -> Option<PathBuf> {
    None
}
/// Name of the journal directory inside the app data directory.
pub const OUTCOME_JOURNAL_DIR: &str = "outcome-journal";
pub struct NativeSession {
    secrets: Arc<dyn SecretStore>,
    agent_socket: Option<PathBuf>,
    read_write_sql: bool,
    secret_deadline: std::time::Duration,
    host_keys: std::sync::OnceLock<Arc<dyn HostKeyStore>>,
    /// Keys native scans actually presented, by `(lowercase host, port)`, waiting for explicit
    /// enrollment: at most 16, 5 minutes each, cleared on renderer reset and shutdown.
    pending_host_keys:
        Mutex<std::collections::HashMap<(String, u16), (stoneql_runtime::HostKey, Instant)>>,
    /// M007 export grants and transfer records of the current renderer session.
    transfers: transfer::Transfers,
    imports: sql_import::ImportGrants,
    closing: AtomicBool,
    active_requests: AtomicUsize,
    /// One owner per app process, kept across renderer sessions: unknown outcomes recorded
    /// before a reload stay listable (owner-scoped) by the next renderer until acknowledged.
    /// Outcomes journaled by an earlier process are restored under this owner at startup.
    owner: stoneql_runtime::OwnerId,
    connections: stoneql_runtime::Connections,
    /// Journal directory override (tests); `None` resolves the app data directory.
    outcome_journal: Option<PathBuf>,
    journal_opened: AtomicBool,
    renderer: Mutex<Option<(String, String)>>,
    bootstrap: tauri::async_runtime::Mutex<()>,
    echoed: AtomicBool,
    allow_exit: AtomicBool,
    close_pending: AtomicBool,
    close_prompted: AtomicBool,
    fallback_open: AtomicBool,
    close_generation: AtomicU64,
    smoke: bool,
    started: Instant,
}
impl NativeSession {
    fn is_current(&self, id: &str) -> bool {
        !self.closing.load(Ordering::SeqCst)
            && !self.allow_exit.load(Ordering::SeqCst)
            && self
                .renderer
                .lock()
                .expect("renderer session lock")
                .as_ref()
                .is_some_and(|(_, current)| current == id)
    }
    /// Ends the current renderer session: its id is revoked and the registry reset (leases
    /// rolled back with recorded outcomes, jobs interrupted and their records released). Unknown
    /// outcomes stay listable by the next renderer. Callers hold `bootstrap`.
    async fn end_renderer(&self) {
        *self.renderer.lock().expect("renderer session lock") = None;
        self.pending_host_keys
            .lock()
            .expect("pending host keys lock")
            .clear();
        self.transfers.reset();
        self.imports.reset();
        self.echoed.store(false, Ordering::SeqCst);
        self.close_pending.store(false, Ordering::SeqCst);
        self.close_prompted.store(false, Ordering::SeqCst);
        self.close_generation.fetch_add(1, Ordering::SeqCst);
        self.connections.reset().await;
    }
    /// Opens the durable unknown-outcome journal once, at the first renderer bootstrap (before
    /// any session exists, so no command can list or admit work before it is restored). If the
    /// directory cannot be resolved it stays unopened and read-write work is refused.
    fn open_journal<R: Runtime>(&self, app: &tauri::AppHandle<R>) {
        if self.journal_opened.swap(true, Ordering::SeqCst) {
            return;
        }
        let data_dir = app.path().app_data_dir().ok();
        if let Some(data_dir) = &data_dir {
            let _ = self
                .host_keys
                .set(Arc::new(FileHostKeyStore::new(data_dir)));
        }
        let dir = self
            .outcome_journal
            .clone()
            .or_else(|| data_dir.map(|dir| dir.join(OUTCOME_JOURNAL_DIR)));
        if let Some(dir) = dir {
            self.connections.open_outcome_journal(&dir, self.owner);
        }
    }
    fn new(smoke: bool, services: HostServices) -> Self {
        let connections = stoneql_runtime::Connections::new(
            8,
            std::time::Duration::from_secs(3),
            database::HANDLE_IDLE_LIFETIME,
        )
        .expect("static native limits");
        // Fail closed until the journal is opened at setup: no read-write work before then.
        connections.require_outcome_journal();
        Self {
            secrets: services.secrets,
            agent_socket: services.agent_socket,
            read_write_sql: services.read_write_sql,
            secret_deadline: services.secret_deadline,
            host_keys: services
                .host_keys
                .map(std::sync::OnceLock::from)
                .unwrap_or_default(),
            pending_host_keys: Mutex::new(std::collections::HashMap::new()),
            transfers: transfer::Transfers::default(),
            imports: sql_import::ImportGrants::default(),
            closing: AtomicBool::new(false),
            active_requests: AtomicUsize::new(0),
            owner: stoneql_runtime::OwnerId::new(),
            connections,
            outcome_journal: services.outcome_journal,
            journal_opened: AtomicBool::new(false),
            renderer: Mutex::new(None),
            bootstrap: tauri::async_runtime::Mutex::new(()),
            echoed: AtomicBool::new(false),
            allow_exit: AtomicBool::new(false),
            close_pending: AtomicBool::new(false),
            close_prompted: AtomicBool::new(false),
            fallback_open: AtomicBool::new(false),
            close_generation: AtomicU64::new(0),
            smoke,
            started: Instant::now(),
        }
    }
}
fn forbidden() -> ErrorDto {
    CoreError::rejected(ErrorCode::Forbidden).into()
}
fn invalid() -> ErrorDto {
    CoreError::rejected(ErrorCode::InvalidRequest).into()
}
fn local_origin(url: &tauri::Url) -> bool {
    (url.scheme() == "tauri" && url.host_str() == Some("localhost") && url.port().is_none())
        || (url.scheme() == "http"
            && url.host_str() == Some("tauri.localhost")
            && url.port().is_none())
}
fn guard<R: Runtime>(window: &WebviewWindow<R>) -> Result<(), ErrorDto> {
    if window.label() != "main" || !window.url().map(|url| local_origin(&url)).unwrap_or(false) {
        return Err(forbidden());
    }
    Ok(())
}
fn decode<T: serde::de::DeserializeOwned + serde::Serialize>(
    message: &str,
) -> Result<Envelope<T>, ErrorDto> {
    if message.len() > 65_536 {
        return Err(invalid());
    }
    v1::decode_envelope(message).map_err(Into::into)
}
#[tauri::command]
async fn host_info<R: Runtime>(
    window: WebviewWindow<R>,
    session: State<'_, NativeSession>,
    message: String,
) -> Result<Envelope<HostInfo>, ErrorDto> {
    guard(&window)?;
    let request: Envelope<HostInfoRequest> = decode(&message)?;
    if !v1::is_opaque_id(&request.payload.nonce) {
        return Err(invalid());
    }
    // One bootstrap at a time, without admitting an unbounded queue of resets.
    let _bootstrap = session
        .bootstrap
        .try_lock()
        .map_err(|_| CoreError::rejected(ErrorCode::Capacity))?;
    if session.closing.load(Ordering::SeqCst) || session.allow_exit.load(Ordering::SeqCst) {
        return Err(forbidden());
    }
    session.open_journal(window.app_handle());
    let previous = session
        .renderer
        .lock()
        .expect("renderer session lock")
        .clone();
    let id = if let Some((_, id)) = previous.filter(|(nonce, _)| nonce == &request.payload.nonce) {
        id
    } else {
        session.end_renderer().await;
        if session.closing.load(Ordering::SeqCst) {
            return Err(forbidden());
        }
        let id = uuid::Uuid::new_v4().to_string();
        *session.renderer.lock().expect("renderer session lock") =
            Some((request.payload.nonce.clone(), id.clone()));
        id
    };
    Ok(Envelope {
        version: ProtocolVersion,
        request_id: request.request_id,
        payload: HostInfo {
            runtime: "desktop".into(),
            app_version: env!("CARGO_PKG_VERSION").into(),
            session_id: id,
            nonce: request.payload.nonce,
            simulated: true,
            read_write_sql: session.read_write_sql,
            sqlite: sqlite_file::SQLITE_SUPPORTED,
        },
    })
}
#[tauri::command]
fn contract_probe<R: Runtime>(
    window: WebviewWindow<R>,
    session: State<'_, NativeSession>,
    message: String,
) -> Result<Envelope<ProbeResult>, ErrorDto> {
    guard(&window)?;
    let request: Envelope<ContractProbe> = decode(&message)?;
    if !session.is_current(&request.payload.session_id) || request.payload.values.len() > 256 {
        return Err(forbidden());
    }
    match request.payload.action {
        ProbeAction::Echo => session.echoed.store(true, Ordering::SeqCst),
        ProbeAction::Ready => {
            if !session.echoed.load(Ordering::SeqCst) || !request.payload.values.is_empty() {
                return Err(invalid());
            }
            if session.smoke && cfg!(any(debug_assertions, feature = "smoke")) {
                println!(
                    "{{\"nativeSmokePassed\":true,\"startupMs\":{}}}",
                    session.started.elapsed().as_millis()
                );
                session.allow_exit.store(true, Ordering::SeqCst);
                window.app_handle().exit(0);
            }
        }
    }
    Ok(Envelope {
        version: ProtocolVersion,
        request_id: request.request_id,
        payload: ProbeResult {
            values: request.payload.values,
            simulated: true,
        },
    })
}
fn shutdown_and_exit<R: Runtime>(app: &tauri::AppHandle<R>) {
    let state = app.state::<NativeSession>();
    if state.closing.swap(true, Ordering::SeqCst) {
        return;
    }
    state
        .pending_host_keys
        .lock()
        .expect("pending host keys lock")
        .clear();
    state.transfers.reset();
    state.imports.reset();
    let connections = state.connections.clone();
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        connections.shutdown().await;
        handle
            .state::<NativeSession>()
            .allow_exit
            .store(true, Ordering::SeqCst);
        handle.exit(0);
    });
}
/// Trusted native close request. Renderer confirmation is required before application exit.
pub fn request_close<R: Runtime>(app: &tauri::AppHandle<R>) -> Result<(), ErrorDto> {
    let state = app.state::<NativeSession>();
    if state.closing.load(Ordering::SeqCst) {
        return Ok(());
    }
    let Some(window) = app.get_webview_window("main") else {
        shutdown_and_exit(app);
        return Ok(());
    };
    if state.fallback_open.load(Ordering::SeqCst) {
        return Ok(());
    }
    let generation = state.close_generation.fetch_add(1, Ordering::SeqCst) + 1;
    state.close_pending.store(true, Ordering::SeqCst);
    state.close_prompted.store(false, Ordering::SeqCst);
    // This script is fixed application code; no SQL, path, identifier or user data is evaluated.
    let _ = window.eval("window.dispatchEvent(new Event('stoneql-close-requested'))");
    let handle = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(2));
        let state = handle.state::<NativeSession>();
        if state.close_generation.load(Ordering::SeqCst) != generation
            || !state.close_pending.load(Ordering::SeqCst)
            || state.close_prompted.load(Ordering::SeqCst)
            || state.fallback_open.swap(true, Ordering::SeqCst)
        {
            return;
        }
        let callback_handle = handle.clone();
        let dialog = handle
            .dialog()
            .message("The workspace is not responding. Close StoneQL? Unsaved changes may be lost.")
            .title("Close StoneQL")
            .buttons(MessageDialogButtons::OkCancelCustom(
                "Close StoneQL".into(),
                "Cancel".into(),
            ));
        dialog.show(move |approved| {
            let state = callback_handle.state::<NativeSession>();
            state.fallback_open.store(false, Ordering::SeqCst);
            if state.close_generation.load(Ordering::SeqCst) != generation
                || !state.close_pending.load(Ordering::SeqCst)
            {
                return;
            }
            state.close_pending.store(false, Ordering::SeqCst);
            if approved {
                shutdown_and_exit(&callback_handle);
            }
        });
    });
    Ok(())
}
/// Native reload guard (M006 blocker 2): whether the main webview may navigate now. Reload by
/// any path (devtools, the WKWebView context menu, `location.reload()`, `Webview::reload`) or any
/// other navigation would replace the renderer that owns the session's leases and jobs. While the
/// runtime registry holds an open lease or a job record (running, or finished with unread
/// results), navigation is refused and the renderer is asked to confirm. On confirmation the
/// renderer rotates the session through `host_info` (resetting the registry: leases rolled back
/// with recorded outcomes, jobs interrupted and their records released) and only then reloads.
/// Renderer state is never consulted. Unknown outcomes stay listable by the next renderer.
pub fn navigation_allowed<R: Runtime>(app: &tauri::AppHandle<R>) -> bool {
    let session = app.state::<NativeSession>();
    !session.connections.has_active_work(session.owner)
}
/// The renderer is gone without confirming anything (its web content process terminated, F-2).
/// The host ends its session itself, exactly as a confirmed reload's rotation does, so the
/// reload that follows is not refused by [`navigation_allowed`] and the next renderer lists the
/// unknown outcomes. Returns whether the app may reload (false while it is closing).
pub async fn renderer_lost<R: Runtime>(app: &tauri::AppHandle<R>) -> bool {
    let session = app.state::<NativeSession>();
    let _bootstrap = session.bootstrap.lock().await;
    if session.closing.load(Ordering::SeqCst) || session.allow_exit.load(Ordering::SeqCst) {
        return false;
    }
    session.end_renderer().await;
    true
}
/// Ends the session for a terminated main renderer, then reloads (shared by both OS handlers).
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn recover_renderer<R: Runtime>(webview: tauri::Webview<R>) {
    tauri::async_runtime::spawn(async move {
        if renderer_lost(webview.app_handle()).await {
            let _ = webview.reload();
        }
    });
}
fn navigation_guard<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    let builder = tauri::plugin::Builder::new("stoneql-navigation-guard");
    // WebKitGTK reports a terminated web process with `web-process-terminated` and, unlike
    // macOS, Tauri has no hook for it (F-2): connect the signal on the main webview directly.
    #[cfg(target_os = "linux")]
    let builder = builder.on_webview_ready(|webview| {
        if webview.label() != "main" {
            return;
        }
        let handler = webview.clone();
        let _ = webview.with_webview(move |platform| {
            use webkit2gtk::WebViewExt;
            platform
                .inner()
                .connect_web_process_terminated(move |_, _reason| {
                    recover_renderer(handler.clone())
                });
        });
    });
    builder
        .on_navigation(|webview, _url| {
            if webview.label() != "main" || navigation_allowed(webview.app_handle()) {
                return true;
            }
            let webview = webview.clone();
            // Off the WebKit policy callback. Fixed application code; no user data is evaluated.
            std::thread::spawn(move || {
                let _ = webview.eval("window.dispatchEvent(new Event('stoneql-reload-requested'))");
            });
            false
        })
        .build()
}
#[tauri::command]
async fn resolve_close<R: Runtime>(
    window: WebviewWindow<R>,
    session: State<'_, NativeSession>,
    message: String,
) -> Result<Envelope<CloseResult>, ErrorDto> {
    guard(&window)?;
    let request: Envelope<CloseDecision> = decode(&message)?;
    if !session.is_current(&request.payload.session_id)
        || !session.close_pending.load(Ordering::SeqCst)
    {
        return Err(forbidden());
    }
    let closed = match request.payload.action {
        CloseAction::Prompted => {
            session.close_prompted.store(true, Ordering::SeqCst);
            false
        }
        CloseAction::Cancel => {
            session.close_pending.store(false, Ordering::SeqCst);
            false
        }
        CloseAction::Confirm => {
            session.close_pending.store(false, Ordering::SeqCst);
            shutdown_and_exit(window.app_handle());
            true
        }
    };
    Ok(Envelope {
        version: ProtocolVersion,
        request_id: request.request_id,
        payload: CloseResult { closed },
    })
}
pub fn app_builder<R: Runtime>(builder: tauri::Builder<R>, smoke: bool) -> tauri::Builder<R> {
    app_builder_with(builder, smoke, HostServices::system())
}
pub fn app_builder_with<R: Runtime>(
    builder: tauri::Builder<R>,
    smoke: bool,
    services: HostServices,
) -> tauri::Builder<R> {
    // Replaces Tauri's default terminate handler, which reloads at once: while work is active
    // the navigation guard refuses that reload and the window stays blank (F-2).
    #[cfg(target_os = "macos")]
    let builder = builder.on_web_content_process_terminate(|webview| {
        if webview.label() == "main" {
            recover_renderer(webview.clone());
        }
    });
    builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(navigation_guard())
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event
                && window.label() == "main"
                && !window
                    .state::<NativeSession>()
                    .allow_exit
                    .load(Ordering::SeqCst)
            {
                api.prevent_close();
                let _ = request_close(window.app_handle());
            }
        })
        .setup(move |app| {
            if smoke {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(15));
                    eprintln!("native smoke did not acknowledge rendered UI before timeout");
                    handle
                        .state::<NativeSession>()
                        .allow_exit
                        .store(true, Ordering::SeqCst);
                    handle.exit(1);
                });
            }
            Ok(())
        })
        .manage(NativeSession::new(smoke, services))
        .invoke_handler(tauri::generate_handler![
            host_info,
            contract_probe,
            resolve_close,
            connect_direct,
            read_only_query,
            disconnect_database,
            database_status,
            switch_database,
            pick_sqlite_file,
            connect_sqlite,
            ssh_host_key,
            pick_ssh_key_file,
            cancel_connect,
            ssh_host_key_status,
            trust_ssh_host_key,
            forget_ssh_host_key,
            store_secret,
            delete_secret,
            secret_store_status,
            unlock_secret_store,
            reset_secret_store,
            pick_export_file,
            start_export,
            export_status,
            cancel_export,
            submit_job,
            fetch_job,
            cancel_job,
            begin_transaction,
            end_transaction,
            list_unresolved_outcomes,
            acknowledge_outcome,
            pick_import_file,
            start_sql_import,
            sql_import_status
        ])
}
pub fn run() {
    let smoke = cfg!(any(debug_assertions, feature = "smoke"))
        && std::env::args().any(|arg| arg == "--smoke-test");
    app_builder(tauri::Builder::default(), smoke)
        .build(tauri::generate_context!())
        .expect("build StoneQL desktop host")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event
                && !app
                    .state::<NativeSession>()
                    .allow_exit
                    .load(Ordering::SeqCst)
            {
                api.prevent_exit();
                let _ = request_close(app);
            }
        });
}
