#![deny(unsafe_code)]

use facet::Facet;
use roam::service;

// Re-export Patch from hotmeal for use in protocol
pub use hotmeal::Patch;

/// Wrapper for owned patches that can be sent over roam.
#[derive(Debug, Clone, Facet)]
pub struct OwnedPatches(pub Vec<Patch<'static>>);

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
    /// 2. Apply the patches
    /// 3. Return the resulting document.body.innerHTML
    async fn test_patch(&self, old_html: String, patches: OwnedPatches) -> TestPatchResult;

    /// Full roundtrip in browser: parse both HTMLs with DOMParser, diff, apply, compare.
    ///
    /// The browser will:
    /// 1. Parse `old_html` with DOMParser, serialize back (normalized_old)
    /// 2. Parse `new_html` with DOMParser, serialize back (normalized_new)
    /// 3. Compute diff using hotmeal-wasm
    /// 4. Apply patches to old DOM
    /// 5. Compare result with normalized_new
    async fn test_roundtrip(&self, old_html: String, new_html: String) -> RoundtripResult;
}

/// Result of a full roundtrip test in the browser.
#[derive(Debug, Clone, Facet)]
pub struct RoundtripResult {
    /// Whether the roundtrip succeeded (patched HTML matches expected).
    pub success: bool,
    /// Error message if something failed.
    pub error: Option<String>,
    /// The old HTML after browser normalization.
    pub normalized_old: String,
    /// The new HTML after browser normalization (expected result).
    pub normalized_new: String,
    /// The actual result after applying patches.
    pub result_html: String,
    /// Number of patches applied.
    pub patch_count: u32,
    /// Step-by-step trace of patch application.
    pub patch_trace: Vec<PatchStep>,
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
    /// Normalized old HTML after browser parsing (innerHTML readback).
    pub normalized_old_html: String,
    /// Step-by-step trace of patch application.
    /// Each entry is the innerHTML after applying that patch.
    pub patch_trace: Vec<PatchStep>,
}

/// One step in the patch application trace.
#[derive(Debug, Clone, Facet)]
pub struct PatchStep {
    /// Index of the patch that was applied.
    pub index: u32,
    /// Debug representation of the patch being applied.
    pub patch_debug: String,
    /// The innerHTML after applying this patch.
    pub html_after: String,
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
