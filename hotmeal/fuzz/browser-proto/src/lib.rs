#![deny(unsafe_code)]

use facet::Facet;
use roam::service;

/// Browser fuzzer service - implemented by the browser, called by the fuzzer.
///
/// The browser receives old HTML and patches, applies them to the DOM,
/// and returns the resulting HTML.
#[service]
pub trait BrowserFuzzer {
    /// Apply patches to HTML in the browser.
    ///
    /// The browser will:
    /// 1. Set document.body.innerHTML to `old_html`
    /// 2. Apply the patches from `patches_json`
    /// 3. Return the resulting document.body.innerHTML
    async fn test_patch(&self, old_html: String, patches_json: String) -> TestPatchResult;
}

/// Result of applying patches in the browser.
#[derive(Debug, Clone, Facet)]
pub struct TestPatchResult {
    /// Whether the patch application succeeded.
    pub success: bool,
    /// Error message if the patch failed.
    pub error: Option<String>,
    /// The resulting HTML after applying patches.
    pub result_html: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn print_method_id() {
        let id = browser_fuzzer_method_id::test_patch();
        println!("\n\ntest_patch method ID: 0x{:016x}n\n\n", id);
    }
}
