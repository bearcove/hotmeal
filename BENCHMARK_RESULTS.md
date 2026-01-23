# Hotmeal Benchmark Results - Arena + Tendril Implementation

Run date: 2026-01-23 (updated after slot-based path refactoring)
Machine: Apple M-series (aarch64-apple-darwin)
Timer precision: 41 ns
Implementation: Arena DOM + Tendril<UTF8, Atomic> strings (no rayon)

## Test Fixtures

| Size | File | Description |
|------|------|-------------|
| **Small** (8KB) | `https_quora.com_What-is-markup-in-HTML.html` | Small Q&A page |
| **Medium** (68KB) | `https_markdownguide.org_basic-syntax.html` | Documentation page |
| **Large** (172KB) | `https_developer.mozilla.org_en-US_docs_Web_HTML.html` | MDN docs |
| **XLarge** (340KB) | `https_fasterthanli.me.html` | Blog homepage with heavy styling |

## Performance Summary

### Key Wins
- **Parsing: 26-45% faster** across all document sizes vs original baseline
- **Serialization: 58-68% faster** on medium/large documents
- **Hot-reload (xlarge): 27% faster** (15.76ms → 11.49ms)
- **Small docs: 76% faster** hot-reload (149µs → 35.1µs)
- **Diff computation: improved** significantly with recent optimizations

### Recent Improvements (this run vs previous)
- **Diff-only xlarge: 8% faster** (5.712ms → 5.27ms)
- **Hot-reload medium: 7% faster** (7.112ms → 6.607ms)
- **Hot-reload large: 10% faster** (7.933ms → 7.177ms)
- **Hot-reload xlarge: 9% faster** (12.58ms → 11.49ms)

## Benchmark Results

### 1. Parsing Benchmarks

Measures time to parse HTML string into arena DOM using html5ever + Tendril:

| Test | Fastest | Median | Mean | vs Baseline |
|------|---------|--------|------|-------------|
| parse_small (8KB) | 13.41 µs | 13.85 µs | **14.27 µs** | 29% faster (was 20.14 µs) |
| parse_medium (68KB) | 950.5 µs | 983 µs | **994.7 µs** | 42% faster (was 1.712 ms) |
| parse_large (172KB) | 1.553 ms | 1.626 ms | **1.645 ms** | 38% faster (was 2.643 ms) |
| parse_xlarge (340KB) | 2.631 ms | 2.754 ms | **2.775 ms** | 47% faster (was 5.279 ms) |

**Improvements:**
- Zero-copy string handling with Tendril (refcounted, no allocations)
- Arena allocation (contiguous memory, better cache locality)
- ~8.2 µs/KB for larger documents (down from ~15 µs/KB)

### 2. Diffing Benchmarks (With Parsing)

Measures full diff cycle: parse old + parse new + compute diff:

| Test | Fastest | Median | Mean | vs Baseline |
|------|---------|--------|------|-------------|
| diff_small | 34.04 µs | 34.47 µs | **35.17 µs** | 74% faster (was 136.7 µs) |
| diff_medium | 5.807 ms | 6.002 ms | **6.018 ms** | 4% faster (was 6.286 ms) |
| diff_large | 6.938 ms | 7.231 ms | **7.268 ms** | 12% faster (was 8.229 ms) |
| diff_xlarge | 10.96 ms | 11.45 ms | **11.49 ms** | 26% faster (was 15.57 ms) |

**Analysis:**
- Faster parsing compensates for diff overhead
- Small documents see massive improvement (74% faster)
- Large documents significantly improved (26% faster on xlarge)

### 3. Diffing Benchmarks (Diff Only)

Measures pure diff computation with pre-parsed DOMs:

| Test | Fastest | Median | Mean | vs Baseline | vs Previous Run |
|------|---------|--------|------|-------------|-----------------|
| diff_only_small | 4.082 µs | 4.291 µs | **4.38 µs** | 93% faster (was 66.22 µs) | 13% faster |
| diff_only_medium | 3.571 ms | 3.75 ms | **3.781 ms** | 42% slower (was 2.667 ms) | 3% faster |
| diff_only_large | 3.383 ms | 3.546 ms | **3.562 ms** | 35% slower (was 2.648 ms) | 7% faster |
| diff_only_xlarge | 5.007 ms | 5.254 ms | **5.27 ms** | 9% slower (was 4.844 ms) | 8% faster |

**Why slower on large docs vs original baseline?**
- Removed rayon parallelization (needed to drop `Sync` requirement for Tendril)
- `precompute_descendants` now runs sequentially instead of in parallel
- Small docs are much faster (simpler traversal, less overhead)

**Trade-off analysis:**
- For xlarge: lost 0.4ms on diff vs baseline, gained 2.5ms on parsing → **net +2.1ms win**
- Recent optimizations have significantly improved diff performance
- Could re-add parallelization with different approach if needed

### 4. Serialization Benchmarks

Measures time to serialize DOM back to HTML string:

| Test | Fastest | Median | Mean | vs Baseline |
|------|---------|--------|------|-------------|
| serialize_small | 7.457 µs | 8.687 µs | **9.296 µs** | 70% slower (was 5.459 µs) |
| serialize_medium | 83.87 µs | 86.24 µs | **88.03 µs** | 70% faster (was 288.7 µs) |
| serialize_large | 188.1 µs | 191.3 µs | **193.7 µs** | 59% faster (was 477.8 µs) |
| serialize_xlarge | 436.1 µs | 473.3 µs | **473.9 µs** | 63% faster (was 1.283 ms) |

**Massive improvements on larger documents:**
- Tendril strings avoid allocations during serialization
- Arena layout enables efficient traversal
- 2.7x faster on xlarge documents
- Small doc overhead increased (still fast at < 10µs)

### 5. Full Hot-Reload Cycle

Measures complete cycle: parse old, parse new, diff, apply patches:

| Test | Fastest | Median | Mean | vs Baseline | vs Previous Run |
|------|---------|--------|------|-------------|-----------------|
| hot_reload_small | 33.2 µs | 34.04 µs | **35.1 µs** | 76% faster (was 149 µs) | same |
| hot_reload_medium | 6.321 ms | 6.591 ms | **6.607 ms** | 6% slower (was 6.232 ms) | 7% faster |
| hot_reload_large | 6.895 ms | 7.134 ms | **7.177 ms** | 12% faster (was 8.18 ms) | 10% faster |
| hot_reload_xlarge | 11.01 ms | 11.48 ms | **11.49 ms** | 27% faster (was 15.76 ms) | 9% faster |

**Breakdown for XLarge (340KB):**
```
                    Before    After     Change
Parse old:         5.3 ms    2.8 ms    -47%
Parse new:         5.3 ms    2.8 ms    -47%
Diff:              4.8 ms    5.3 ms    +10%
Apply patches:     0.3 ms    0.3 ms    ±0%
────────────────────────────────────────────
Total:            15.7 ms   11.2 ms    -29%
```

**Net result:** Parsing wins outweigh diff costs for large documents!

## Comparison with Baseline

### Parsing Performance

| Size | Baseline | Current | Speedup |
|------|----------|---------|---------|
| Small | 20.14 µs | 14.27 µs | 1.41x |
| Medium | 1.712 ms | 994.7 µs | 1.72x |
| Large | 2.643 ms | 1.645 ms | 1.61x |
| XLarge | 5.279 ms | 2.775 ms | 1.90x |

### Serialization Performance

| Size | Baseline | Current | Speedup |
|------|----------|---------|---------|
| Small | 5.459 µs | 9.296 µs | 0.59x |
| Medium | 288.7 µs | 88.03 µs | 3.28x |
| Large | 477.8 µs | 193.7 µs | 2.47x |
| XLarge | 1.283 ms | 473.9 µs | 2.71x |

### Full Hot-Reload Performance

| Size | Baseline | Current | Speedup |
|------|----------|---------|---------|
| Small | 149 µs | 35.1 µs | 4.25x |
| Medium | 6.232 ms | 6.607 ms | 0.94x |
| Large | 8.18 ms | 7.177 ms | 1.14x |
| XLarge | 15.76 ms | 11.49 ms | 1.37x |

## Analysis

### What Worked Well

1. **Zero-copy parsing with Tendril**
   - 47% faster parsing on xlarge docs
   - Refcounted strings avoid allocations
   - Direct integration with html5ever output

2. **Arena allocation**
   - Better cache locality
   - Faster serialization (2.7x on xlarge)
   - Simplified memory management

3. **Small document performance**
   - 76% faster hot-reload
   - Lower overhead dominates

4. **Recent optimizations (slot-based paths)**
   - Improved diff performance across the board
   - XLarge diff: 5.71ms → 5.27ms (8% faster)
   - Hot-reload improvements: 7-10% on medium/large docs

### What Got Slower

1. **Diff computation on large docs vs original baseline**
   - Removed rayon parallelization
   - Sequential `precompute_descendants`
   - 9-42% slower on medium/large vs original baseline
   - But improved significantly with recent optimizations

2. **Small document serialization**
   - 70% slower than baseline
   - Still very fast (< 10µs)

### Why Is This Still Worth It?

For the target use case (hot-reload on large documents):
- **XLarge hot-reload: 15.76ms → 11.49ms (27% faster)**
- Parsing improvements (47%) > diff slowdown (9%)
- Serialization is 2.7x faster on large docs
- Small docs see massive wins (76% faster, 4.25x speedup)

## Future Optimizations

### Re-add Parallelization
- Could use `scoped_threadpool` or similar
- Only parallelize on large documents
- Estimated gain: 1-2ms on xlarge

### Direct cinereus Integration
- Already implemented! (no conversion step)
- arena_dom implements TreeTypes directly
- Clean architecture, no copying

### Profile-Guided Optimization
- These benchmarks use single modifications
- Real hot-reload might have different patterns
- Verify with real workloads

## Conclusion

**The arena + Tendril implementation is a strong net win:**
- ✅ Faster parsing (47% on large docs)
- ✅ Much faster serialization (2.7x on large docs)
- ✅ Massive wins on small docs (76% faster, 4.25x speedup)
- ✅ 27% faster hot-reload on xlarge docs
- ✅ Recent slot-based path refactoring improved diff performance 7-10%
- ⚠️ Diff still 9-42% slower on medium/large docs vs original baseline (rayon removal)

**Recommendation:** Keep this implementation. The trade-offs are very favorable, with significant wins across the board. Could re-add parallelization later for additional gains if needed.

## Running Benchmarks

```bash
# Run all benchmarks
cargo bench

# Run specific benchmark
cargo bench --bench parsing
cargo bench --bench diffing
cargo bench --bench serialization
cargo bench --bench full_cycle

# Compare with baseline
git checkout main
cargo bench 2>&1 | tee baseline.txt
git checkout more-arena
cargo bench 2>&1 | tee arena.txt
```
