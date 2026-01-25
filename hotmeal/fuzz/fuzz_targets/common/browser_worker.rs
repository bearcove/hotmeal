use std::sync::OnceLock;

use browser_proto::{BrowserFuzzerClient, DomNode, OwnedPatches, RoundtripResult, TestPatchResult};
use chromiumoxide::{
    Browser, BrowserConfig,
    cdp::browser_protocol::network::EnableParams,
    cdp::js_protocol::runtime::{EnableParams as RuntimeEnableParams, EventConsoleApiCalled},
};
use futures_util::StreamExt;
use roam_stream::{ConnectionHandle, HandshakeConfig, NoDispatcher};
use roam_websocket::{WsTransport, ws_accept};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot, watch};
use tokio_tungstenite::accept_async;

static CHANNEL: OnceLock<mpsc::UnboundedSender<BrowserRequest>> = OnceLock::new();

/// Internal request enum covering all BrowserFuzzer service methods.
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
    eprintln!("[browser-fuzz] WebSocket server listening on port {}", port);

    let headed = std::env::var("BROWSER_HEAD").is_ok();
    let mut config = BrowserConfig::builder()
        .arg("--allow-file-access-from-files")
        .arg("--disable-web-security");
    if headed {
        config = config.with_head().arg("--auto-open-devtools-for-tabs");
    }
    let (mut browser, mut handler) = Browser::launch(config.build().unwrap()).await.unwrap();

    if let Some(child) = browser.get_mut_child() {
        if let Some(pid) = child.inner.id() {
            let _ = CHROME_PID.set(pid);
            eprintln!("[browser-fuzz] Chrome launched (pid {})", pid);
        }
    }

    tokio::spawn(async move {
        while let Some(event) = handler.next().await {
            let _ = event;
        }
    });

    let bundle_path = std::env::current_dir()
        .unwrap()
        .join("browser-bundle")
        .join("dist")
        .join("index.html");
    let bundle_url = format!("file://{}#{}", bundle_path.display(), port);
    eprintln!("[browser-fuzz] Loading bundle from: {}", bundle_url);

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
        eprintln!("[browser-fuzz] Network logging enabled");
    }

    eprintln!("[browser-fuzz] Page loaded, WASM will connect automatically");

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

        let client = BrowserFuzzerClient::new(handle);

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
                    eprintln!("[browser-fuzz] test_patch error: {:?}", e);
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
                    eprintln!("[browser-fuzz] test_roundtrip error: {:?}", e);
                    let _ = response_tx.send(None);
                }
            },
            BrowserRequest::ParseToDom { html, response_tx } => {
                match client.parse_to_dom(html).await {
                    Ok(result) => {
                        let _ = response_tx.send(Some(result));
                    }
                    Err(e) => {
                        eprintln!("[browser-fuzz] parse_to_dom error: {:?}", e);
                        let _ = response_tx.send(None);
                    }
                }
            }
        }
    }

    listener_handle.abort();
}

async fn accept_connections(
    listener: TcpListener,
    conn_tx: watch::Sender<Option<ConnectionHandle>>,
) {
    loop {
        eprintln!(
            "[browser-fuzz] Waiting for browser to connect on {:?}...",
            listener.local_addr()
        );
        match listener.accept().await {
            Ok((stream, addr)) => {
                eprintln!(
                    "[browser-fuzz] TCP accept from {}, upgrading to WebSocket...",
                    addr
                );
                match accept_async(stream).await {
                    Ok(ws_stream) => {
                        eprintln!(
                            "[browser-fuzz] WebSocket upgrade complete, starting roam handshake..."
                        );
                        let transport = WsTransport::new(ws_stream);
                        match ws_accept(transport, HandshakeConfig::default(), NoDispatcher).await {
                            Ok((handle, _incoming, driver)) => {
                                eprintln!("[browser-fuzz] Roam handshake complete");
                                tokio::spawn(async move {
                                    if let Err(e) = driver.run().await {
                                        eprintln!("[browser-fuzz] Driver error: {:?}", e);
                                    }
                                });
                                let _ = conn_tx.send(Some(handle));
                                eprintln!("[browser-fuzz] Ready to process fuzz requests");
                            }
                            Err(e) => eprintln!("[browser-fuzz] Roam handshake failed: {:?}", e),
                        }
                    }
                    Err(e) => eprintln!("[browser-fuzz] WebSocket upgrade failed: {:?}", e),
                }
            }
            Err(e) => eprintln!("[browser-fuzz] Accept failed: {:?}", e),
        }
    }
}

// Store Chrome PID for cleanup
static CHROME_PID: OnceLock<u32> = OnceLock::new();
static PANIC_HOOK_SET: OnceLock<()> = OnceLock::new();

fn kill_chrome() {
    if let Some(&pid) = CHROME_PID.get() {
        eprintln!("[browser-fuzz] Killing Chrome (pid {})", pid);
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
