use browser_proto::{
    BrowserFuzzer, BrowserFuzzerDispatcher, OwnedPatches, Patch, PatchStep, RoundtripResult,
    TestPatchResult,
};
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
        patches: OwnedPatches,
    ) -> TestPatchResult {
        run_test(&old_html, patches.0)
    }

    async fn test_roundtrip(
        &self,
        _cx: &Context,
        old_html: String,
        new_html: String,
    ) -> RoundtripResult {
        run_roundtrip(&old_html, &new_html)
    }
}

fn log(msg: &str) {
    web_sys::console::log_1(&wasm_bindgen::JsValue::from_str(msg));
}

fn run_roundtrip(old_html: &str, new_html: &str) -> RoundtripResult {
    use web_sys::{DomParser, SupportedType};

    log(&format!(
        "[browser-wasm] roundtrip: old={:?} new={:?}",
        old_html, new_html
    ));

    // Create DOMParser
    let parser = match DomParser::new() {
        Ok(p) => p,
        Err(e) => {
            return RoundtripResult {
                success: false,
                error: Some(format!("DOMParser::new failed: {:?}", e)),
                normalized_old: String::new(),
                normalized_new: String::new(),
                result_html: String::new(),
                patch_count: 0,
                patch_trace: vec![],
            };
        }
    };

    // Wrap inputs in full HTML documents for parsing
    let old_doc_html = format!("<html><body>{}</body></html>", old_html);
    let new_doc_html = format!("<html><body>{}</body></html>", new_html);

    // Parse old HTML
    let old_doc = match parser.parse_from_string(&old_doc_html, SupportedType::TextHtml) {
        Ok(doc) => doc,
        Err(e) => {
            return RoundtripResult {
                success: false,
                error: Some(format!("parse old_html failed: {:?}", e)),
                normalized_old: String::new(),
                normalized_new: String::new(),
                result_html: String::new(),
                patch_count: 0,
                patch_trace: vec![],
            };
        }
    };

    // Parse new HTML
    let new_doc = match parser.parse_from_string(&new_doc_html, SupportedType::TextHtml) {
        Ok(doc) => doc,
        Err(e) => {
            return RoundtripResult {
                success: false,
                error: Some(format!("parse new_html failed: {:?}", e)),
                normalized_old: String::new(),
                normalized_new: String::new(),
                result_html: String::new(),
                patch_count: 0,
                patch_trace: vec![],
            };
        }
    };

    // Get normalized HTML from both
    let normalized_old = old_doc.body().map(|b| b.inner_html()).unwrap_or_default();
    let normalized_new = new_doc.body().map(|b| b.inner_html()).unwrap_or_default();

    log(&format!(
        "[browser-wasm] normalized: old={:?} new={:?}",
        normalized_old, normalized_new
    ));

    // Skip if both normalize to empty
    if normalized_old.trim().is_empty() && normalized_new.trim().is_empty() {
        return RoundtripResult {
            success: true,
            error: None,
            normalized_old,
            normalized_new,
            result_html: String::new(),
            patch_count: 0,
            patch_trace: vec![],
        };
    }

    // Compute diff using hotmeal-wasm (on normalized HTML)
    let patches = match hotmeal_wasm::diff_html_patches(
        &format!("<html><body>{}</body></html>", normalized_old),
        &format!("<html><body>{}</body></html>", normalized_new),
    ) {
        Ok(p) => p,
        Err(e) => {
            return RoundtripResult {
                success: false,
                error: Some(format!("diff_html failed: {:?}", e)),
                normalized_old,
                normalized_new,
                result_html: String::new(),
                patch_count: 0,
                patch_trace: vec![],
            };
        }
    };

    log(&format!("[browser-wasm] {} patches", patches.len()));

    // Set the document body to the old content so we can patch it
    let window = web_sys::window().unwrap();
    let document = window.document().unwrap();
    let body = document.body().unwrap();
    body.set_inner_html(&normalized_old);

    // Apply patches one at a time, capturing state after each
    // Use persistent slots across all patches
    let mut slots = hotmeal_wasm::Slots::new();
    let mut patch_trace = Vec::with_capacity(patches.len());
    for (i, patch) in patches.iter().enumerate() {
        log(&format!("[browser-wasm] applying patch {}: {:?}", i, patch));
        if let Err(e) = hotmeal_wasm::apply_patches_with_slots(&[patch.clone()], &mut slots) {
            return RoundtripResult {
                success: false,
                error: Some(format!("patch {} failed: {:?}", i, e)),
                normalized_old,
                normalized_new,
                result_html: body.inner_html(),
                patch_count: i as u32,
                patch_trace,
            };
        }
        patch_trace.push(PatchStep {
            index: i as u32,
            html_after: body.inner_html(),
        });
    }

    let result_html = body.inner_html();
    let patch_count = patches.len() as u32;

    log(&format!(
        "[browser-wasm] result: {:?}, expected: {:?}",
        result_html, normalized_new
    ));

    // Compare result with expected
    let success = result_html == normalized_new;

    RoundtripResult {
        success,
        error: if success {
            None
        } else {
            Some("result doesn't match expected".to_string())
        },
        normalized_old,
        normalized_new,
        result_html,
        patch_count,
        patch_trace,
    }
}

fn run_test(old_html: &str, patches: Vec<Patch<'static>>) -> TestPatchResult {
    log(&format!(
        "[browser-wasm] run_test starting, old_html={:?}",
        old_html
    ));

    // Set the initial HTML
    if let Err(e) = hotmeal_wasm::set_body_inner_html(old_html) {
        log(&format!(
            "[browser-wasm] set_body_inner_html failed: {:?}",
            e
        ));
        return TestPatchResult {
            success: false,
            error: Some(format!("set_body_inner_html failed: {:?}", e)),
            result_html: String::new(),
            normalized_old_html: String::new(),
            patch_trace: vec![],
        };
    }

    // Read back the normalized HTML
    let normalized_old_html = match hotmeal_wasm::get_body_inner_html() {
        Ok(html) => {
            log(&format!("[browser-wasm] normalized_old_html={:?}", html));
            html
        }
        Err(e) => {
            log(&format!(
                "[browser-wasm] get_body_inner_html failed: {:?}",
                e
            ));
            return TestPatchResult {
                success: false,
                error: Some(format!("get_body_inner_html failed: {:?}", e)),
                result_html: String::new(),
                normalized_old_html: String::new(),
                patch_trace: vec![],
            };
        }
    };

    log(&format!(
        "[browser-wasm] applying {} patches",
        patches.len()
    ));

    // Apply patches one at a time, capturing HTML after each
    let mut patch_trace = Vec::with_capacity(patches.len());

    for (i, patch) in patches.iter().enumerate() {
        log(&format!("[browser-wasm] applying patch {}: {:?}", i, patch));
        if let Err(e) = hotmeal_wasm::apply_patches(&[patch.clone()]) {
            log(&format!("[browser-wasm] patch {} failed: {:?}", i, e));
            return TestPatchResult {
                success: false,
                error: Some(format!("patch {}: {:?}", i, e)),
                result_html: String::new(),
                normalized_old_html,
                patch_trace,
            };
        }

        let html_after = match hotmeal_wasm::get_body_inner_html() {
            Ok(html) => {
                log(&format!("[browser-wasm] after patch {}: {:?}", i, html));
                html
            }
            Err(e) => {
                log(&format!(
                    "[browser-wasm] get_body_inner_html after patch {} failed: {:?}",
                    i, e
                ));
                return TestPatchResult {
                    success: false,
                    error: Some(format!(
                        "get_body_inner_html after patch {} failed: {:?}",
                        i, e
                    )),
                    result_html: String::new(),
                    normalized_old_html,
                    patch_trace,
                };
            }
        };

        patch_trace.push(PatchStep {
            index: i as u32,
            html_after,
        });
    }

    let result_html = patch_trace
        .last()
        .map(|s| s.html_after.clone())
        .unwrap_or_else(|| normalized_old_html.clone());

    log(&format!("[browser-wasm] run_test complete, success=true"));
    TestPatchResult {
        success: true,
        error: None,
        result_html,
        normalized_old_html,
        patch_trace,
    }
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
