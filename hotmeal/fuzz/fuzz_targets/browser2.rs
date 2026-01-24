#![no_main]

//! Browser-side roundtrip fuzzer.
//!
//! This target sends old + new HTML to the browser, which:
//! 1. Parses both with DOMParser (browser normalization)
//! 2. Computes diff using hotmeal-wasm
//! 3. Applies patches
//! 4. Compares result with expected
//!
//! Since parsing and patching use the same browser, there should be no parser mismatches.

use std::sync::OnceLock;

use arbitrary::Arbitrary;
use browser_proto::{BrowserFuzzerClient, RoundtripResult};
use chromiumoxide::{Browser, BrowserConfig, cdp::browser_protocol::network::EnableParams};
use futures_util::StreamExt;
use libfuzzer_sys::fuzz_target;
use roam_stream::{ConnectionHandle, HandshakeConfig, NoDispatcher};
use roam_websocket::{WsTransport, ws_accept};
use similar::{ChangeTag, TextDiff};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot, watch};
use tokio_tungstenite::accept_async;

// Store Chrome PID for cleanup
static CHROME_PID: OnceLock<u32> = OnceLock::new();
static PANIC_HOOK_SET: OnceLock<()> = OnceLock::new();

fn kill_chrome() {
    if let Some(&pid) = CHROME_PID.get() {
        eprintln!("[browser2-fuzz] Killing Chrome (pid {})", pid);
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

#[derive(Debug, Clone, Arbitrary)]
struct Input {
    html_a: String,
    html_b: String,
}

struct FuzzRequest {
    old_html: String,
    new_html: String,
    response_tx: oneshot::Sender<RoundtripResult>,
}

static CHANNEL: OnceLock<mpsc::UnboundedSender<FuzzRequest>> = OnceLock::new();

fn get_channel() -> &'static mpsc::UnboundedSender<FuzzRequest> {
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

async fn run_browser_worker(mut rx: mpsc::UnboundedReceiver<FuzzRequest>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    eprintln!(
        "[browser2-fuzz] WebSocket server listening on port {}",
        port
    );

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
            eprintln!("[browser2-fuzz] Chrome launched (pid {})", pid);
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
    eprintln!("[browser2-fuzz] Loading bundle from: {}", bundle_url);

    let page = browser.new_page(&bundle_url).await.unwrap();

    if headed {
        page.execute(EnableParams::default()).await.ok();
        eprintln!("[browser2-fuzz] Network logging enabled");
    }

    eprintln!("[browser2-fuzz] Page loaded, WASM will connect automatically");

    let (conn_tx, conn_rx) = watch::channel::<Option<ConnectionHandle>>(None);
    let listener_handle = tokio::spawn(accept_connections(listener, conn_tx));

    while let Some(req) = rx.recv().await {
        let handle = loop {
            let current = conn_rx.borrow().clone();
            if let Some(h) = current {
                break h;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        };

        let client = BrowserFuzzerClient::new(handle);
        match client.test_roundtrip(req.old_html, req.new_html).await {
            Ok(result) => {
                let _ = req.response_tx.send(result);
            }
            Err(e) => {
                eprintln!("[browser2-fuzz] RPC error: {:?}", e);
                eprintln!("[browser2-fuzz] Browser connection lost, exiting");
                std::process::exit(0);
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
            "[browser2-fuzz] Waiting for browser to connect on {:?}...",
            listener.local_addr()
        );
        match listener.accept().await {
            Ok((stream, addr)) => {
                eprintln!(
                    "[browser2-fuzz] TCP accept from {}, upgrading to WebSocket...",
                    addr
                );
                match accept_async(stream).await {
                    Ok(ws_stream) => {
                        eprintln!(
                            "[browser2-fuzz] WebSocket upgrade complete, starting roam handshake..."
                        );
                        let transport = WsTransport::new(ws_stream);
                        match ws_accept(transport, HandshakeConfig::default(), NoDispatcher).await {
                            Ok((handle, _incoming, driver)) => {
                                eprintln!("[browser2-fuzz] Roam handshake complete");
                                tokio::spawn(async move {
                                    if let Err(e) = driver.run().await {
                                        eprintln!("[browser2-fuzz] Driver error: {:?}", e);
                                    }
                                });
                                let _ = conn_tx.send(Some(handle));
                                eprintln!("[browser2-fuzz] Ready to process fuzz requests");
                            }
                            Err(e) => {
                                eprintln!("[browser2-fuzz] Roam handshake failed: {:?}", e);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("[browser2-fuzz] WebSocket upgrade failed: {:?}", e);
                    }
                }
            }
            Err(e) => {
                eprintln!("[browser2-fuzz] Accept failed: {:?}", e);
            }
        }
    }
}

fuzz_target!(|input: Input| {
    // Skip empty inputs
    if input.html_a.is_empty() || input.html_b.is_empty() {
        return;
    }

    // Send to browser worker
    let (response_tx, response_rx) = oneshot::channel();
    get_channel()
        .send(FuzzRequest {
            old_html: input.html_a.clone(),
            new_html: input.html_b.clone(),
            response_tx,
        })
        .unwrap();

    // Wait for result
    let result = response_rx.blocking_recv().unwrap();

    // Skip cases where both normalize to empty (garbage input)
    if result.normalized_old.trim().is_empty() && result.normalized_new.trim().is_empty() {
        return;
    }

    if !result.success {
        print_failure(&input, &result);
        panic!("Browser roundtrip failed: {:?}", result.error);
    }
});

fn print_failure(input: &Input, result: &RoundtripResult) {
    eprintln!("\n========== FUZZ FAILURE ==========");
    eprintln!("Input A (raw): {:?}", input.html_a);
    eprintln!("Input B (raw): {:?}", input.html_b);
    eprintln!("\n--- Normalized old ---");
    eprintln!("{:?}", result.normalized_old);
    eprintln!("\n--- Patch trace ({} patches) ---", result.patch_count);
    for step in &result.patch_trace {
        eprintln!("  After patch {}: {:?}", step.index, step.html_after);
    }
    eprintln!("\n--- Diff (expected vs result) ---");
    print_char_diff(&result.normalized_new, &result.result_html);
    eprintln!("\nError: {:?}", result.error);
    eprintln!("==================================\n");
}

fn print_char_diff(expected: &str, actual: &str) {
    let diff = TextDiff::from_chars(expected, actual);
    for change in diff.iter_all_changes() {
        let (sign, color) = match change.tag() {
            ChangeTag::Delete => ("-", "\x1b[31m"), // red
            ChangeTag::Insert => ("+", "\x1b[32m"), // green
            ChangeTag::Equal => (" ", ""),
        };
        if change.tag() != ChangeTag::Equal {
            eprint!("{}{}{:?}\x1b[0m", color, sign, change.value());
        }
    }
    eprintln!();
}
