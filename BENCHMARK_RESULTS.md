# Hotmeal Benchmark Results - Arena + Stem Implementation

Run date: 2026-01-23 (after Stem<'a> for Send+Sync Document)
Machine: Apple M-series (aarch64-apple-darwin)
Timer precision: 41 ns
Implementation: Arena DOM + Stem<'a> strings (Send+Sync, zero-copy where possible)

## Test Fixtures

| Size | File | Description |
|------|------|-------------|
| **Small** (8KB) | `https_quora.com_What-is-markup-in-HTML.html` | Small Q&A page |
| **Medium** (68KB) | `https_markdownguide.org_basic-syntax.html` | Documentation page |
| **Large** (172KB) | `https_developer.mozilla.org_en-US_docs_Web_HTML.html` | MDN docs |
| **XLarge** (340KB) | `https_fasterthanli.me.html` | Blog homepage with heavy styling |
| **XXLarge** (492KB) | `xxl.html` | Extra large document |

## Performance Summary

### Key Changes in This Version

- **Document is now Send+Sync** via `Stem<'a>` type (closes #21)
- **Zero-copy parsing** when content points into the original StrTendril
- **Breaking API change**: `parse()` now takes `&StrTendril` instead of `&str`

### Trade-offs

The Stem conversion adds some overhead on larger documents, but enables:
- Thread-safe Document handling
- Zero-copy parsing where possible
- Consistent API with explicit ownership

## Benchmark Results

### 1. Parsing Benchmarks

Measures time to parse HTML string into arena DOM:

| Test | Fastest | Median | Mean | vs Previous |
|------|---------|--------|------|-------------|
| parse_small (8KB) | 13.87 µs | 13.99 µs | **14.34 µs** | similar |
| parse_medium (68KB) | 925 µs | 967.1 µs | **976 µs** | 6% faster |
| parse_large (172KB) | 1.482 ms | 1.542 ms | **1.558 ms** | 8% faster |
| parse_xlarge (340KB) | 4.626 ms | 4.748 ms | **4.767 ms** | 67% slower* |

*XLarge regression due to Stem conversion overhead on documents with many nodes.

### 2. Serialization Benchmarks

Measures time to serialize DOM back to HTML string:

| Test | Fastest | Median | Mean | vs Previous |
|------|---------|--------|------|-------------|
| serialize_small | 8.374 µs | 8.457 µs | **8.556 µs** | similar |
| serialize_medium | 72.58 µs | 78.14 µs | **78.73 µs** | 10% faster |
| serialize_large | 170.5 µs | 181.7 µs | **184.1 µs** | 8% faster |
| serialize_xlarge | 406.4 µs | 419.4 µs | **425.1 µs** | 13% faster |

### 3. Diffing Benchmarks (With Parsing)

Measures full diff cycle: parse old + parse new + compute diff:

| Test | Fastest | Median | Mean |
|------|---------|--------|------|
| diff_small | 32.91 µs | 33.74 µs | **34.5 µs** |
| diff_medium | 3.429 ms | 3.748 ms | **3.872 ms** |
| diff_large | 4.475 ms | 4.938 ms | **4.998 ms** |
| diff_xlarge | 11.61 ms | 12.24 ms | **12.81 ms** |

### 4. Diffing Benchmarks (Diff Only)

Measures pure diff computation with pre-parsed DOMs:

| Test | Fastest | Median | Mean |
|------|---------|--------|------|
| diff_only_small | 3.749 µs | 3.874 µs | **3.988 µs** |
| diff_only_medium | 1.572 ms | 1.611 ms | **1.712 ms** |
| diff_only_large | 1.395 ms | 1.492 ms | **1.521 ms** |
| diff_only_xlarge | 1.899 ms | 2.078 ms | **2.146 ms** |

### 5. Full Hot-Reload Cycle

Measures complete cycle: parse old, parse new, diff, apply patches:

| Test | Fastest | Median | Mean |
|------|---------|--------|------|
| hot_reload_small | 30.91 µs | 31.2 µs | **32.12 µs** |
| hot_reload_medium | 3.355 ms | 3.527 ms | **3.534 ms** |
| hot_reload_large | 4.442 ms | 4.771 ms | **4.818 ms** |
| hot_reload_xlarge | 11.58 ms | 12.09 ms | **12.1 ms** |
| hot_reload_xxlarge | 13.83 ms | 14.5 ms | **14.53 ms** |

## Comparison with Previous (Tendril-only)

| Metric | Previous | Current | Change |
|--------|----------|---------|--------|
| parse_small | 15.12 µs | 14.34 µs | 5% faster |
| parse_medium | 1.04 ms | 976 µs | 6% faster |
| parse_large | 1.69 ms | 1.558 ms | 8% faster |
| parse_xlarge | 2.85 ms | 4.767 ms | 67% slower |
| serialize_medium | 87.84 µs | 78.73 µs | 10% faster |
| serialize_large | 199 µs | 184.1 µs | 8% faster |
| serialize_xlarge | 486.6 µs | 425.1 µs | 13% faster |
| hot_reload_small | 32.6 µs | 32.12 µs | similar |
| hot_reload_medium | 3.48 ms | 3.534 ms | similar |
| hot_reload_large | 4.87 ms | 4.818 ms | similar |
| hot_reload_xlarge | 7.54 ms | 12.1 ms | 60% slower |
| hot_reload_xxlarge | 7.46 ms | 14.53 ms | 95% slower |

## Analysis

### What This Version Provides

1. **Send+Sync Document** - Can now be shared across threads
2. **Zero-copy parsing** - Content that comes from the input tendril borrows directly
3. **Improved serialization** - 8-13% faster across medium/large docs
4. **Improved small/medium parsing** - 5-8% faster

### Trade-offs

1. **XLarge document regression** - 60-95% slower on very large documents
   - This is due to the Stem conversion checking every string
   - The zero-copy check (`tendril_to_stem_with_input`) runs for every node
   - On documents with thousands of nodes, this adds up

### Potential Optimizations

1. **Batch Stem conversion** - Convert strings in batches to reduce overhead
2. **Skip zero-copy check for small strings** - Short strings might be faster to just copy
3. **Use a different strategy for large documents** - Profile to find the hot path

## Running Benchmarks

```bash
# Run all benchmarks
cargo bench

# Run specific benchmark
cargo bench --bench parsing
cargo bench --bench diffing
cargo bench --bench serialization
cargo bench --bench full_cycle
```
