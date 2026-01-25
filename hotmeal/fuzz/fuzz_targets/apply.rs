#![no_main]

//! Native diff+apply fuzzer.
//!
//! Tests that hotmeal can diff two DOMs and apply patches correctly.

use arbitrary::Arbitrary;
use hotmeal::StrTendril;
use libfuzzer_sys::fuzz_target;

mod common;

#[derive(Arbitrary, Debug)]
struct Input {
    html_a: Vec<u8>,
    html_b: Vec<u8>,
}

fuzz_target!(|input: Input| {
    let a = String::from_utf8_lossy(&input.html_a);
    let b = String::from_utf8_lossy(&input.html_b);

    let a_tendril = StrTendril::from(a.as_ref());
    let b_tendril = StrTendril::from(b.as_ref());

    let doc_a = hotmeal::parse(&a_tendril);
    let doc_b = hotmeal::parse(&b_tendril);

    let patches = hotmeal::diff(&doc_a, &doc_b).expect("diff must always succeed");

    // Capture the full trace
    let mut patched = doc_a.clone();
    let trace = common::PatchTrace::capture(&mut patched, &patches);

    // Check for failures
    if !trace.all_succeeded() {
        eprintln!("Patch application failed!\n");
        eprintln!("Input A: {:?}", a);
        eprintln!("Input B: {:?}", b);
        eprintln!("\n{}", trace);
        panic!("apply_patches must always succeed");
    }

    // Compare body contents
    let patched_body = patched.to_body_html();
    let expected_body = doc_b.to_body_html();

    if patched_body != expected_body {
        eprintln!("Roundtrip mismatch!\n");
        eprintln!("Input A: {:?}", a);
        eprintln!("Input B: {:?}", b);
        eprintln!("\nExpected body: {:?}", expected_body);
        eprintln!("Got body: {:?}", patched_body);
        eprintln!("\nFull trace:\n{}", trace);
        panic!("Patched body content should match target body content");
    }
});
