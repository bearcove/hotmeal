#![no_main]

//! Apply parity fuzzer.
//!
//! Compares native hotmeal patch application against browser hotmeal-wasm.
//! Both compute the same diff, then apply patches step-by-step.
//! After each patch, we compare the full DOM trees to catch any divergence.

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

    // Send to browser worker for roundtrip
    let Some(result) = common::test_roundtrip(full_a.to_string(), full_b.to_string()) else {
        return;
    };

    // Skip cases where browser parsed to empty
    if result.normalized_old.trim().is_empty() {
        return;
    }

    // Apply patches natively with hotmeal and collect DOM trees after each step
    let normalized_old_tendril = StrTendril::from(result.normalized_old.clone());
    let mut native_doc = hotmeal::parse(&normalized_old_tendril);
    let mut native_slots = native_doc.init_patch_slots();

    // Compare patch traces in lockstep
    if result.patch_trace.len() != patches.len() {
        eprintln!("\n========== PATCH COUNT MISMATCH ==========");
        eprintln!("Browser trace length: {}", result.patch_trace.len());
        eprintln!("Native patches count: {}", patches.len());
        eprintln!("===========================================\n");
        panic!("Patch count mismatch!");
    }

    for (i, (patch, browser_step)) in patches.iter().zip(result.patch_trace.iter()).enumerate() {
        // Apply patch natively
        native_doc
            .apply_patch_with_slots(patch.clone(), &mut native_slots)
            .unwrap();

        // Get native DOM tree after this patch
        let native_tree = common::document_body_to_dom_node(&native_doc);

        // Compare DOM trees
        if native_tree != browser_step.dom_tree {
            eprintln!("\n========== APPLY MISMATCH (step {}) ==========", i);
            eprintln!("Patch: {:?}", browser_step.patch_debug);
            eprintln!("\n--- Native DOM tree ---");
            eprintln!("{}", native_tree);
            eprintln!("\n--- Browser DOM tree ---");
            eprintln!("{}", browser_step.dom_tree);
            eprintln!("\n--- diff ---");
            print_diff(&native_tree.to_string(), &browser_step.dom_tree.to_string());
            eprintln!("\n===============================================\n");
            panic!("Apply mismatch at step {}!", i);
        }
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
