#![no_main]

use std::sync::OnceLock;

use arbitrary::Arbitrary;
use browser_proto::{BrowserFuzzerClient, TestPatchResult};
use chromiumoxide::{Browser, BrowserConfig, cdp::browser_protocol::network::EnableParams};
use futures_util::StreamExt;
use hotmeal::StrTendril;
use libfuzzer_sys::fuzz_target;
use roam_stream::{ConnectionHandle, HandshakeConfig, NoDispatcher};
use roam_websocket::{WsTransport, ws_accept};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot, watch};
use tokio_tungstenite::accept_async;

#[derive(Debug, Clone, Arbitrary)]
struct Input {
    html_a: String,
    html_b: String,
}

struct FuzzRequest {
    old_html: String,
    patches_json: String,
    response_tx: oneshot::Sender<TestPatchResult>,
}

static CHANNEL: OnceLock<mpsc::UnboundedSender<FuzzRequest>> = OnceLock::new();

fn get_channel() -> &'static mpsc::UnboundedSender<FuzzRequest> {
    CHANNEL.get_or_init(|| {
        let (tx, rx) = mpsc::unbounded_channel();
        std::thread::spawn(move || {
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(run_browser_worker(rx))
        });
        tx
    })
}

async fn run_browser_worker(mut rx: mpsc::UnboundedReceiver<FuzzRequest>) {
    // 1. Start WebSocket server on random port
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    eprintln!("[browser-fuzz] WebSocket server listening on port {}", port);

    // 2. Launch Chrome with chromiumoxide
    let headed = std::env::var("BROWSER_HEAD").is_ok();
    let mut config = BrowserConfig::builder()
        .arg("--allow-file-access-from-files")
        .arg("--disable-web-security");
    if headed {
        config = config.with_head().arg("--auto-open-devtools-for-tabs");
    }
    let (browser, mut handler) = Browser::launch(config.build().unwrap()).await.unwrap();

    // Spawn browser event handler
    tokio::spawn(async move {
        while let Some(event) = handler.next().await {
            let _ = event;
        }
    });

    // 3. Navigate to the bundle HTML with port in fragment
    let bundle_path = std::env::current_dir()
        .unwrap()
        .join("browser-bundle")
        .join("dist")
        .join("index.html");
    let bundle_url = format!("file://{}#{}", bundle_path.display(), port);
    eprintln!("[browser-fuzz] Loading bundle from: {}", bundle_url);

    let page = browser.new_page(&bundle_url).await.unwrap();

    // Enable network logging if headed
    if headed {
        page.execute(EnableParams::default()).await.ok();
        eprintln!("[browser-fuzz] Network logging enabled");
    }

    eprintln!("[browser-fuzz] Page loaded, WASM will connect automatically");

    // Channel to broadcast current connection handle
    let (conn_tx, conn_rx) = watch::channel::<Option<ConnectionHandle>>(None);

    // Spawn connection acceptor loop
    let listener_handle = tokio::spawn(accept_connections(listener, conn_tx));

    // Process fuzz requests, getting fresh handle for each
    while let Some(req) = rx.recv().await {
        // Wait for a valid connection
        let handle = loop {
            let current = conn_rx.borrow().clone();
            if let Some(h) = current {
                break h;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        };

        let client = BrowserFuzzerClient::new(handle);
        match client.test_patch(req.old_html, req.patches_json).await {
            Ok(result) => {
                let _ = req.response_tx.send(result);
            }
            Err(e) => {
                eprintln!("[browser-fuzz] RPC error: {:?}", e);
                let _ = req.response_tx.send(TestPatchResult {
                    success: false,
                    error: Some(format!("RPC error: {:?}", e)),
                    result_html: String::new(),
                });
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
                                // Spawn driver
                                tokio::spawn(async move {
                                    if let Err(e) = driver.run().await {
                                        eprintln!("[browser-fuzz] Driver error: {:?}", e);
                                    }
                                });
                                // Broadcast new handle
                                let _ = conn_tx.send(Some(handle));
                                eprintln!("[browser-fuzz] Ready to process fuzz requests");
                            }
                            Err(e) => {
                                eprintln!("[browser-fuzz] Roam handshake failed: {:?}", e);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("[browser-fuzz] WebSocket upgrade failed: {:?}", e);
                    }
                }
            }
            Err(e) => {
                eprintln!("[browser-fuzz] Accept failed: {:?}", e);
            }
        }
    }
}

fuzz_target!(|input: Input| {
    // Skip empty inputs
    if input.html_a.is_empty() || input.html_b.is_empty() {
        return;
    }

    // Wrap in full HTML document
    let full_a = format!("<html><body>{}</body></html>", input.html_a);
    let full_b = format!("<html><body>{}</body></html>", input.html_b);

    // Compute patches using hotmeal
    let old_tendril = StrTendril::from(full_a);
    let new_tendril = StrTendril::from(full_b);

    let patches = match hotmeal::diff_html(&old_tendril, &new_tendril) {
        Ok(p) => p,
        Err(_) => return, // Skip inputs that fail to diff
    };

    // Serialize patches to JSON using facet_json
    let patches_json = match facet_json::to_string(&patches) {
        Ok(j) => j,
        Err(_) => return,
    };

    // Send to browser worker
    let (response_tx, response_rx) = oneshot::channel();
    get_channel()
        .send(FuzzRequest {
            old_html: input.html_a.clone(),
            patches_json,
            response_tx,
        })
        .unwrap();

    // Wait for result
    let result = response_rx.blocking_recv().unwrap();

    if !result.success {
        panic!("Browser patch failed: {:?}", result.error);
    }
});
