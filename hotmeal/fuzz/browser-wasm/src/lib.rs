use browser_proto::{BrowserFuzzer, BrowserFuzzerDispatcher, TestPatchResult};
use roam::Context;
use roam_session::initiate_framed;
use roam_websocket::WsTransport;
use wasm_bindgen::prelude::*;

#[derive(Clone)]
struct Handler;

impl BrowserFuzzer for Handler {
    async fn test_patch(
        &self,
        _cx: &Context,
        old_html: String,
        patches_json: String,
    ) -> TestPatchResult {
        match run_test(&old_html, &patches_json) {
            Ok(result_html) => TestPatchResult {
                success: true,
                error: None,
                result_html,
            },
            Err(e) => TestPatchResult {
                success: false,
                error: Some(e),
                result_html: String::new(),
            },
        }
    }
}

fn run_test(old_html: &str, patches_json: &str) -> Result<String, String> {
    hotmeal_wasm::set_body_inner_html(old_html).map_err(|e| format!("{:?}", e))?;
    hotmeal_wasm::apply_patches_json(patches_json).map_err(|e| format!("{:?}", e))?;
    hotmeal_wasm::get_body_inner_html().map_err(|e| format!("{:?}", e))
}

#[wasm_bindgen]
pub async fn connect(port: u32) -> Result<(), JsValue> {
    let url = format!("ws://127.0.0.1:{}", port);
    web_sys::console::log_1(&format!("[browser-wasm] connecting to {}", url).into());

    let transport = WsTransport::connect(&url)
        .await
        .map_err(|e| format!("connect failed: {}", e))?;

    web_sys::console::log_1(&"[browser-wasm] connected, starting handshake".into());

    let dispatcher = BrowserFuzzerDispatcher::new(Handler);
    let (_handle, _incoming, driver) = initiate_framed(transport, Default::default(), dispatcher)
        .await
        .map_err(|e| format!("handshake failed: {:?}", e))?;

    web_sys::console::log_1(&"[browser-wasm] handshake complete, running driver".into());

    driver
        .run()
        .await
        .map_err(|e| format!("driver error: {:?}", e))?;

    Ok(())
}
