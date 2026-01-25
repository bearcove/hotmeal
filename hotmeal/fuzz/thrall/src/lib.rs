//! Thrall - Control browsers from Rust tests
//!
//! Named after the supernatural servants of demons and vampires,
//! this crate lets you control a Chrome browser for testing purposes.

use std::path::PathBuf;
use std::sync::OnceLock;

use browser_proto::BrowserClient;
pub use browser_proto::{DomNode, OwnedPatches, RoundtripResult, TestPatchResult};
use chromiumoxide::{
    Browser, BrowserConfig,
    cdp::browser_protocol::network::EnableParams,
    cdp::js_protocol::runtime::{EnableParams as RuntimeEnableParams, EventConsoleApiCalled},
    fetcher::{BrowserFetcher, BrowserFetcherOptions},
};
use futures_util::StreamExt;
use roam_stream::{ConnectionHandle, HandshakeConfig, NoDispatcher};
use roam_websocket::{WsTransport, ws_accept};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot, watch};
use tokio_tungstenite::accept_async;

static CHANNEL: OnceLock<mpsc::UnboundedSender<BrowserRequest>> = OnceLock::new();

/// Internal request enum covering all Browser service methods.
enum BrowserRequest {
    TestPatch {
        old_html: String,
        patches: OwnedPatches,
        response_tx: oneshot::Sender<Option<TestPatchResult>>,
    },
    TestRoundtrip {
        old_html: String,
        new_html: String,
        response_tx: oneshot::Sender<Option<RoundtripResult>>,
    },
    ParseToDom {
        html: String,
        response_tx: oneshot::Sender<Option<DomNode>>,
    },
}

/// Returns the channel through which one may send requests to the browser worker.
fn get_channel() -> &'static mpsc::UnboundedSender<BrowserRequest> {
    CHANNEL.get_or_init(|| {
        setup_cleanup_hooks();
        let (tx, rx) = mpsc::unbounded_channel();
        std::thread::spawn(move || {
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(run_browser_worker(rx))
        });
        tx
    })
}

/// Apply patches to HTML in the browser (blocking, synchronous).
///
/// Returns `None` if the browser connection failed.
pub fn test_patch(old_html: String, patches: OwnedPatches) -> Option<TestPatchResult> {
    let (response_tx, response_rx) = oneshot::channel();
    get_channel()
        .send(BrowserRequest::TestPatch {
            old_html,
            patches,
            response_tx,
        })
        .unwrap();
    response_rx.blocking_recv().unwrap()
}

/// Full roundtrip test in browser (blocking, synchronous).
///
/// Returns `None` if the browser connection failed.
pub fn test_roundtrip(old_html: String, new_html: String) -> Option<RoundtripResult> {
    let (response_tx, response_rx) = oneshot::channel();
    get_channel()
        .send(BrowserRequest::TestRoundtrip {
            old_html,
            new_html,
            response_tx,
        })
        .unwrap();
    response_rx.blocking_recv().unwrap()
}

/// Parse HTML in the browser and return the DOM tree (blocking, synchronous).
///
/// Returns `None` if the browser connection failed.
pub fn parse_to_dom(html: String) -> Option<DomNode> {
    let (response_tx, response_rx) = oneshot::channel();
    get_channel()
        .send(BrowserRequest::ParseToDom { html, response_tx })
        .unwrap();
    response_rx.blocking_recv().unwrap()
}

async fn run_browser_worker(mut rx: mpsc::UnboundedReceiver<BrowserRequest>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    eprintln!("[thrall] WebSocket server listening on port {}", port);

    // Get Chrome executable - either from system or download it
    let chrome_path = get_or_fetch_chrome().await;

    let headed = std::env::var("BROWSER_HEAD").is_ok();
    let no_sandbox = std::env::var("THRALL_NO_SANDBOX").is_ok() || is_running_in_ci();

    let mut config = BrowserConfig::builder()
        .arg("--allow-file-access-from-files")
        .arg("--disable-web-security");
    if no_sandbox {
        // Required when running as root in CI containers
        config = config.arg("--no-sandbox");
        eprintln!("[thrall] Running with --no-sandbox (CI mode)");
    }
    if let Some(path) = chrome_path {
        config = config.chrome_executable(path);
    }
    if headed {
        config = config.with_head().arg("--auto-open-devtools-for-tabs");
    }
    let (mut browser, mut handler) = Browser::launch(config.build().unwrap()).await.unwrap();

    if let Some(child) = browser.get_mut_child()
        && let Some(pid) = child.inner.id()
    {
        let _ = CHROME_PID.set(pid);
        eprintln!("[thrall] Chrome launched (pid {})", pid);
    }

    tokio::spawn(async move {
        while let Some(event) = handler.next().await {
            let _ = event;
        }
    });

    // Find the browser bundle - check multiple locations
    let bundle_path = find_browser_bundle().expect("Could not find browser-bundle/dist/index.html");
    let bundle_url = format!("file://{}#{}", bundle_path.display(), port);
    eprintln!("[thrall] Loading bundle from: {}", bundle_url);

    let page = browser.new_page(&bundle_url).await.unwrap();

    if std::env::var("BROWSER_CONSOLE").is_ok() {
        page.execute(RuntimeEnableParams::default()).await.ok();
        let mut console_events = page
            .event_listener::<EventConsoleApiCalled>()
            .await
            .unwrap();
        tokio::spawn(async move {
            while let Some(event) = console_events.next().await {
                let args: Vec<String> = event
                    .args
                    .iter()
                    .filter_map(|arg| arg.value.as_ref().map(|v| v.to_string()))
                    .collect();
                eprintln!("[console] {}", args.join(" "));
            }
        });
    }

    if headed {
        page.execute(EnableParams::default()).await.ok();
        eprintln!("[thrall] Network logging enabled");
    }

    eprintln!("[thrall] Page loaded, WASM will connect automatically");

    let (conn_tx, conn_rx) = watch::channel::<Option<ConnectionHandle>>(None);
    let listener_handle = tokio::spawn(accept_connections(listener, conn_tx));

    while let Some(req) = rx.recv().await {
        let handle = loop {
            let current = conn_rx.borrow().clone();
            if let Some(h) = current {
                break h;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await
        };

        let client = BrowserClient::new(handle);

        match req {
            BrowserRequest::TestPatch {
                old_html,
                patches,
                response_tx,
            } => match client.test_patch(old_html, patches).await {
                Ok(result) => {
                    let _ = response_tx.send(Some(result));
                }
                Err(e) => {
                    eprintln!("[thrall] test_patch error: {:?}", e);
                    let _ = response_tx.send(None);
                }
            },
            BrowserRequest::TestRoundtrip {
                old_html,
                new_html,
                response_tx,
            } => match client.test_roundtrip(old_html, new_html).await {
                Ok(result) => {
                    let _ = response_tx.send(Some(result));
                }
                Err(e) => {
                    eprintln!("[thrall] test_roundtrip error: {:?}", e);
                    let _ = response_tx.send(None);
                }
            },
            BrowserRequest::ParseToDom { html, response_tx } => {
                match client.parse_to_dom(html).await {
                    Ok(result) => {
                        let _ = response_tx.send(Some(result));
                    }
                    Err(e) => {
                        eprintln!("[thrall] parse_to_dom error: {:?}", e);
                        let _ = response_tx.send(None);
                    }
                }
            }
        }
    }

    listener_handle.abort();
}

/// Detect if we're running in a CI environment.
fn is_running_in_ci() -> bool {
    // GitHub Actions
    std::env::var("CI").is_ok() ||
    std::env::var("GITHUB_ACTIONS").is_ok() ||
    // GitLab CI
    std::env::var("GITLAB_CI").is_ok() ||
    // Other common CI systems
    std::env::var("JENKINS_URL").is_ok() ||
    std::env::var("CIRCLECI").is_ok() ||
    std::env::var("TRAVIS").is_ok()
}

/// Get Chrome executable path, downloading if necessary.
///
/// Returns `None` if Chrome is available on the system PATH (chromiumoxide will find it).
/// Returns `Some(path)` if we downloaded Chrome ourselves.
async fn get_or_fetch_chrome() -> Option<PathBuf> {
    // First, check if Chrome is available on the system
    // chromiumoxide checks common locations, so we only need to download
    // if it fails to launch

    // Try to use CHROME_PATH env var if set
    if let Ok(chrome_path) = std::env::var("CHROME_PATH") {
        let path = PathBuf::from(chrome_path);
        if path.exists() {
            eprintln!("[thrall] Using Chrome from CHROME_PATH: {}", path.display());
            return Some(path);
        }
    }

    // Check if we should skip auto-download (e.g., if Chrome is installed)
    if std::env::var("THRALL_NO_DOWNLOAD").is_ok() {
        eprintln!("[thrall] THRALL_NO_DOWNLOAD set, using system Chrome");
        return None;
    }

    // Download Chrome to a cache directory
    let cache_dir = dirs_next::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("thrall")
        .join("chrome");

    // Check if we already have a downloaded Chrome
    let marker_file = cache_dir.join(".fetched");
    if marker_file.exists() {
        // Find the executable in the cache
        if let Some(exe) = find_chrome_executable(&cache_dir) {
            eprintln!("[thrall] Using cached Chrome: {}", exe.display());
            return Some(exe);
        }
    }

    eprintln!("[thrall] Downloading Chrome to {}...", cache_dir.display());

    tokio::fs::create_dir_all(&cache_dir).await.ok()?;

    let fetcher = BrowserFetcher::new(
        BrowserFetcherOptions::builder()
            .with_path(&cache_dir)
            .build()
            .ok()?,
    );

    match fetcher.fetch().await {
        Ok(info) => {
            // Create marker file
            tokio::fs::write(&marker_file, "").await.ok();
            eprintln!(
                "[thrall] Chrome downloaded: {}",
                info.executable_path.display()
            );
            Some(info.executable_path)
        }
        Err(e) => {
            eprintln!("[thrall] Failed to download Chrome: {:?}", e);
            eprintln!("[thrall] Will try system Chrome instead");
            None
        }
    }
}

/// Find Chrome executable in a directory (platform-specific).
fn find_chrome_executable(dir: &std::path::Path) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let app = dir
            .join("chrome-mac")
            .join("Chromium.app")
            .join("Contents")
            .join("MacOS")
            .join("Chromium");
        if app.exists() {
            return Some(app);
        }
    }

    #[cfg(target_os = "linux")]
    {
        let exe = dir.join("chrome-linux").join("chrome");
        if exe.exists() {
            return Some(exe);
        }
    }

    #[cfg(target_os = "windows")]
    {
        let exe = dir.join("chrome-win").join("chrome.exe");
        if exe.exists() {
            return Some(exe);
        }
    }

    None
}

/// Find the browser bundle directory.
fn find_browser_bundle() -> Option<std::path::PathBuf> {
    let index_html = "index.html";

    // Try CARGO_MANIFEST_DIR first (most reliable in cargo test)
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let manifest_path = std::path::PathBuf::from(&manifest_dir);
        eprintln!("[thrall] CARGO_MANIFEST_DIR: {}", manifest_dir);

        // From hotmeal/fuzz (when running cargo test from fuzz dir)
        let path1 = manifest_path
            .join("browser-bundle")
            .join("dist")
            .join(index_html);
        if path1.exists() {
            return Some(path1);
        }

        // From hotmeal/fuzz/thrall -> hotmeal/fuzz/browser-bundle
        if let Some(parent) = manifest_path.parent() {
            let path2 = parent.join("browser-bundle").join("dist").join(index_html);
            if path2.exists() {
                return Some(path2);
            }
        }

        // From hotmeal -> hotmeal/fuzz/browser-bundle
        let path3 = manifest_path
            .join("fuzz")
            .join("browser-bundle")
            .join("dist")
            .join(index_html);
        if path3.exists() {
            return Some(path3);
        }
    }

    // Fallback: try relative to current working directory
    if let Ok(cwd) = std::env::current_dir() {
        eprintln!("[thrall] CWD: {}", cwd.display());

        // Check: ./browser-bundle/dist/index.html
        let path4 = cwd.join("browser-bundle").join("dist").join(index_html);
        if path4.exists() {
            return Some(path4);
        }

        // Check: ./fuzz/browser-bundle/dist/index.html
        let path5 = cwd
            .join("fuzz")
            .join("browser-bundle")
            .join("dist")
            .join(index_html);
        if path5.exists() {
            return Some(path5);
        }
    }

    None
}

async fn accept_connections(
    listener: TcpListener,
    conn_tx: watch::Sender<Option<ConnectionHandle>>,
) {
    loop {
        eprintln!(
            "[thrall] Waiting for browser to connect on {:?}...",
            listener.local_addr()
        );
        match listener.accept().await {
            Ok((stream, addr)) => {
                eprintln!(
                    "[thrall] TCP accept from {}, upgrading to WebSocket...",
                    addr
                );
                match accept_async(stream).await {
                    Ok(ws_stream) => {
                        eprintln!(
                            "[thrall] WebSocket upgrade complete, starting roam handshake..."
                        );
                        let transport = WsTransport::new(ws_stream);
                        match ws_accept(transport, HandshakeConfig::default(), NoDispatcher).await {
                            Ok((handle, _incoming, driver)) => {
                                eprintln!("[thrall] Roam handshake complete");
                                tokio::spawn(async move {
                                    if let Err(e) = driver.run().await {
                                        eprintln!("[thrall] Driver error: {:?}", e);
                                    }
                                });
                                let _ = conn_tx.send(Some(handle));
                                eprintln!("[thrall] Ready to process requests");
                            }
                            Err(e) => eprintln!("[thrall] Roam handshake failed: {:?}", e),
                        }
                    }
                    Err(e) => eprintln!("[thrall] WebSocket upgrade failed: {:?}", e),
                }
            }
            Err(e) => eprintln!("[thrall] Accept failed: {:?}", e),
        }
    }
}

// Store Chrome PID for cleanup
static CHROME_PID: OnceLock<u32> = OnceLock::new();
static PANIC_HOOK_SET: OnceLock<()> = OnceLock::new();

fn kill_chrome() {
    if let Some(&pid) = CHROME_PID.get() {
        eprintln!("[thrall] Killing Chrome (pid {})", pid);
        unsafe {
            libc::kill(pid as i32, libc::SIGKILL);
        }
    }
}

extern "C" fn atexit_handler() {
    kill_chrome();
}

fn setup_cleanup_hooks() {
    PANIC_HOOK_SET.get_or_init(|| {
        let original = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            kill_chrome();
            original(info);
        }));
        unsafe {
            libc::signal(libc::SIGINT, signal_handler as *const () as usize);
            libc::signal(libc::SIGTERM, signal_handler as *const () as usize);
            libc::atexit(atexit_handler);
        }
    });
}

extern "C" fn signal_handler(sig: libc::c_int) {
    kill_chrome();
    unsafe {
        libc::signal(sig, libc::SIG_DFL);
        libc::raise(sig);
    }
}
