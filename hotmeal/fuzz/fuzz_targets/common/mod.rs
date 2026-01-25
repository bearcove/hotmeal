mod browser_worker;
pub use browser_worker::*;

mod dom_generator;
pub use dom_generator::*;

mod dom_node;
pub use dom_node::*;

mod patch_trace;
pub use patch_trace::*;

/// Split raw bytes at 0xFF delimiter into two HTML strings.
fn split_input(data: &[u8]) -> Option<(String, String)> {
    let pos = data.iter().position(|&b| b == 0xFF)?;
    let html_a = std::str::from_utf8(&data[..pos]).ok()?.to_owned();
    let html_b = std::str::from_utf8(&data[pos + 1..]).ok()?.to_owned();
    Some((html_a, html_b))
}

/// Splits the input data into two sanitized HTML documents wrapped in a standard template.
pub fn prepare_html_inputs(data: &[u8]) -> Option<(String, String)> {
    // Split at 0xFF delimiter
    let Some((html_a, html_b)) = split_input(data) else {
        return None;
    };

    // Skip empty inputs
    if html_a.is_empty() || html_b.is_empty() {
        return None;
    }

    // Skip inputs with null bytes
    if html_a.contains('\0') || html_b.contains('\0') {
        return None;
    }

    // Wrap in full HTML document with DOCTYPE for no-quirks mode
    let full_a = format!("<!DOCTYPE html><html><body>{}</body></html>", html_a);
    let full_b = format!("<!DOCTYPE html><html><body>{}</body></html>", html_b);

    Some((full_a, full_b))
}
