#![no_main]

//! Browser-side patch application fuzzer.
//!
//! This target computes diff server-side using hotmeal, then sends patches
//! to the browser to apply. This tests that patches computed by hotmeal
//! can be correctly applied by hotmeal-wasm in the browser DOM.

use hotmeal::StrTendril;
use libfuzzer_sys::fuzz_target;
use similar::{ChangeTag, TextDiff};

mod common;

fuzz_target!(|data: &[u8]| target(data));

fn target(data: &[u8]) {
    let Some((full_a, full_b)) = common::prepare_html_inputs(data) else {
        return;
    };

    // Compute patches using hotmeal
    let full_a = StrTendril::from(full_a.clone());
    let full_b = StrTendril::from(full_b.clone());

    let patches = match hotmeal::diff_html(&full_a, &full_b) {
        Ok(p) => p,
        Err(err) => panic!("hotmeal failed to diff {err}"),
    };

    // Parse old HTML with html5ever and get DOM tree
    let old_doc = hotmeal::parse(&full_a);
    let html5ever_tree = common::document_body_to_dom_node(&old_doc);

    // Send to browser worker
    let Some(result) = common::test_roundtrip(full_a.to_string(), full_b.to_string()) else {
        return;
    };

    // Skip cases where browser parsed to empty
    if result.normalized_old.trim().is_empty() {
        return;
    }

    // Apply patches natively with hotmeal and collect traces
    let mut native_trace = Vec::with_capacity(patches.len());
    let normalized_old_tendril = StrTendril::from(result.normalized_old.clone());
    let mut native_doc_for_apply = hotmeal::parse(&normalized_old_tendril);
    let mut native_slots = native_doc_for_apply.init_patch_slots();

    for patch in patches.iter() {
        native_doc_for_apply
            .apply_patch_with_slots(patch.clone(), &mut native_slots)
            .unwrap();
        let html_after = native_doc_for_apply.to_body_html();
        native_trace.push(html_after);
    }

    // Compare html5ever tree with browser tree - detect parser mismatches
    if html5ever_tree != result.initial_dom_tree {
        eprintln!("\n========== PARSER MISMATCH ==========");
        eprintln!("Input: {:?}", full_a);
        eprintln!("\n--- html5ever tree ---");
        eprintln!("{}", common::pretty_print_dom(&html5ever_tree, 0));
        eprintln!("--- browser tree ---");
        eprintln!("{}", common::pretty_print_dom(&result.initial_dom_tree, 0));
        eprintln!("\n--- diff ---");
        let h5e_str = common::pretty_print_dom(&html5ever_tree, 0);
        let browser_str = common::pretty_print_dom(&result.initial_dom_tree, 0);
        print_diff(&h5e_str, &browser_str);
        eprintln!("=====================================\n");
        panic!("Parser mismatch detected! Fix html5ever to match browser.");
    }

    // Compare patch traces in lockstep
    if result.patch_trace.len() != native_trace.len() {
        eprintln!("\n========== PATCH TRACE LENGTH MISMATCH ==========");
        eprintln!("Browser trace length: {}", result.patch_trace.len());
        eprintln!("Native trace length: {}", native_trace.len());
        eprintln!("=================================================\n");
        panic!("Patch trace length mismatch!");
    }

    for (i, (browser_step, native_html)) in result
        .patch_trace
        .iter()
        .zip(native_trace.iter())
        .enumerate()
    {
        if browser_step.html_after != *native_html {
            eprintln!("\n========== PATCH STEP MISMATCH (step {}) ==========", i);
            eprintln!("Patch: {:?}", browser_step.patch_debug);
            eprintln!("\n--- Native (hotmeal) ---");
            eprintln!("{}", native_html);
            eprintln!("\n--- Browser (wasm) ---");
            eprintln!("{}", browser_step.html_after);
            eprintln!("\n--- diff ---");
            print_diff(native_html, &browser_step.html_after);
            eprintln!("\n===================================================\n");
            panic!("Patch application mismatch at step {}!", i);
        }
    }

    // Final comparison: result HTML should match normalized new HTML
    if result.result_html != result.normalized_new {
        eprintln!("\n========== FINAL RESULT MISMATCH ==========");
        eprintln!("Input A: {:?}", full_a);
        eprintln!("Input B: {:?}", full_b);
        eprintln!("\n--- Expected (normalized B) ---");
        eprintln!("{}", result.normalized_new);
        eprintln!("\n--- Actual (patched result) ---");
        eprintln!("{}", result.result_html);
        eprintln!("\n--- diff ---");
        print_diff(&result.normalized_new, &result.result_html);
        eprintln!("\n==============================================\n");
        panic!("Patch application mismatch! Patches did not produce expected result.");
    }
}

fn print_diff(expected: &str, actual: &str) {
    let diff = TextDiff::from_lines(expected, actual);
    for change in diff.iter_all_changes() {
        let (sign, color) = match change.tag() {
            ChangeTag::Delete => ("-", "\x1b[31m"),
            ChangeTag::Insert => ("+", "\x1b[32m"),
            ChangeTag::Equal => (" ", ""),
        };
        eprint!("{}{}{}\x1b[0m", color, sign, change.value());
    }
}
