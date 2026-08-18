# Update Document Benchmarks

This benchmark compares the old full document update path against the new incremental document update path. It also measures Tree-sitter parsing time for the same generated file and edit.

The benchmark target is:

```text
crates/objectscript-core/benches/update_document.rs
```

It is a Criterion benchmark, so it reports statistically sampled timing ranges and writes an HTML report that can be used for presentations.

## Quick Smoke Run

Use this when you only want to verify that the benchmark compiles and runs.

To compile the benchmark without collecting measurements:

```bash
cargo bench -p objectscript-core --features update-bench --bench update_document --no-run
```

```bash
BENCH_METHODS=10 \
BENCH_BODY_LINES=1 \
BENCH_SAMPLE_SIZE=10 \
BENCH_WARMUP_SECS=1 \
BENCH_MEASUREMENT_SECS=1 \
cargo bench -p objectscript-core --features update-bench --bench update_document -- --quiet
```

The smoke run is intentionally small. Do not use it as the final presentation number.

## Presentation Run

Use a larger synthetic class and longer measurement window for numbers that are more stable.

```bash
BENCH_METHODS=100 \
BENCH_BODY_LINES=12 \
BENCH_SAMPLE_SIZE=10 \
BENCH_WARMUP_SECS=3 \
BENCH_MEASUREMENT_SECS=10 \
cargo bench -p objectscript-core --features update-bench --bench update_document
```

Criterion writes the aggregate report to:

```text
target/criterion/report/index.html
```

It also writes per-benchmark reports under:

```text
target/criterion/update_document/
target/criterion/parse_document/
```

If the benchmark is run from inside `crates/objectscript-core`, Criterion may write those reports under `crates/objectscript-core/target/criterion/` instead.

If `gnuplot` is not installed, Criterion falls back to the Rust `plotters` backend. That is fine; the benchmark still runs and still generates HTML reports.

## Running Only One Benchmark Group

The same Criterion target contains two groups:

- `update_document`
- `parse_document`

To run only the update benchmarks:

```bash
cargo bench -p objectscript-core --features update-bench --bench update_document -- update_document
```

To run only the parser benchmarks:

```bash
cargo bench -p objectscript-core --features update-bench --bench update_document -- parse_document
```

## Running Against a Real File

Set `BENCH_INPUT_FILE` to benchmark one existing source file instead of a generated synthetic class.

For example, to run the update and parser benchmarks against the large dotted-statement routine fixture:

```bash
BENCH_INPUT_FILE=/Users/hkimura/zed-objectscript/objectscript-lsp/objectscript-tests/local/test-large-dotted-statements-full.mac \
BENCH_SAMPLE_SIZE=10 \
BENCH_WARMUP_SECS=3 \
BENCH_MEASUREMENT_SECS=10 \
cargo bench -p objectscript-core --features update-bench --bench update_document
```

To run only the parser benchmarks for that file:

```bash
BENCH_INPUT_FILE=/Users/hkimura/zed-objectscript/objectscript-lsp/objectscript-tests/local/test-large-dotted-statements-full.mac \
BENCH_SAMPLE_SIZE=10 \
BENCH_WARMUP_SECS=1 \
BENCH_MEASUREMENT_SECS=1 \
cargo bench -p objectscript-core --features update-bench --bench update_document -- parse_document --quiet
```

To run only the update benchmarks for that file:

```bash
BENCH_INPUT_FILE=/Users/hkimura/zed-objectscript/objectscript-lsp/objectscript-tests/local/test-large-dotted-statements-full.mac \
BENCH_SAMPLE_SIZE=10 \
BENCH_WARMUP_SECS=1 \
BENCH_MEASUREMENT_SECS=1 \
cargo bench -p objectscript-core --features update-bench --bench update_document -- update_document --quiet
```

The real-file mode supports `.cls`, `.mac`, `.inc`, `.rtn`, and `.int` files. When `BENCH_INPUT_FILE` is set, it overrides `BENCH_METHODS`, `BENCH_METHODS_LIST`, and `BENCH_BODY_LINES`.

The large dotted-statement fixture is currently about 87 KB:

```text
1182 lines
87462 bytes
```

The real-file benchmark makes one small insertion near the middle of the file. For routine files, it inserts an ObjectScript comment line and preserves the existing leading whitespace or dotted indentation. For class files, it inserts a comment line.

Some routine benchmarks currently print semantic-analysis warnings to stderr, such as unsupported set-target cases. Those warnings are not Criterion output. If they make the terminal hard to read, redirect stderr:

```bash
BENCH_INPUT_FILE=/Users/hkimura/zed-objectscript/objectscript-lsp/objectscript-tests/local/test-large-dotted-statements-full.mac \
cargo bench -p objectscript-core --features update-bench --bench update_document -- update_document --quiet \
2>/tmp/objectscript-update-document-bench.stderr
```

## Scaling Run

To show how the two paths scale as document size grows, run several method counts in one benchmark invocation:

```bash
BENCH_METHODS_LIST=20,50,100 \
BENCH_BODY_LINES=12 \
BENCH_SAMPLE_SIZE=10 \
BENCH_WARMUP_SECS=3 \
BENCH_MEASUREMENT_SECS=10 \
cargo bench -p objectscript-core --features update-bench --bench update_document
```

This produces separate `full_update_document` and `incremental_update_document` measurements for each generated class size.

It also produces `full_parse_document` and `incremental_parse_document` measurements for the same sizes.

## Environment Variables

| Variable | Default | Meaning |
|---|---:|---|
| `BENCH_METHODS` | `100` | Number of methods generated in the synthetic `Bench.Big` class. Ignored when `BENCH_METHODS_LIST` is set. |
| `BENCH_METHODS_LIST` | unset | Comma-separated method counts used to benchmark multiple document sizes in one run. Example: `20,50,100`. |
| `BENCH_BODY_LINES` | `12` | Number of repeated body lines generated inside each method. |
| `BENCH_INPUT_FILE` | unset | Path to a real `.cls`, `.mac`, `.inc`, `.rtn`, or `.int` file to benchmark instead of generating a synthetic class. |
| `BENCH_SAMPLE_SIZE` | `10` | Criterion sample count. The benchmark enforces Criterion's minimum of 10. |
| `BENCH_WARMUP_SECS` | `3` | Criterion warmup duration in seconds. |
| `BENCH_MEASUREMENT_SECS` | `10` | Criterion measurement duration in seconds. |

## What the Benchmark Builds

The benchmark creates a synthetic ObjectScript class named `Bench.Big`.

The generated class shape is:

```objectscript
Class Bench.Big
{
Method Method0() As %Status
{
    Set methodNumber = 0
    Set total = 0
    Set total = total + 0
    ...
    Write total
    Quit total
}

Method Method1() As %Status
{
    ...
}
}
```

The class size is controlled by `BENCH_METHODS` or `BENCH_METHODS_LIST` plus `BENCH_BODY_LINES`.

The benchmark then makes one small edit in the middle method:

```objectscript
Set methodNumber = N
Write methodNumber
```

That simulates the common LSP case: a large document is already open and indexed, and the user makes a small edit inside one method body.

When `BENCH_INPUT_FILE` is set, the benchmark uses the file contents instead of generating `Bench.Big`. It still follows the same setup: parse and index the old document, prepare one small edit, parse the new document, then time the update functions against that prepared state.

## What Is Timed

### Update Benchmarks

Each Criterion iteration starts with an already-indexed old document. The timed section compares:

```text
ProjectData::full_update_document(...)
```

against:

```text
ProjectData::incremental_update_document(...)
```

The benchmark uses `BatchSize::PerIteration`, so each measurement gets fresh project state and does not reuse a mutated `ProjectData` from the previous iteration.

### Parser Benchmarks

The parser group compares:

```text
full_parse_document
```

against:

```text
incremental_parse_document
```

`full_parse_document` measures parsing the updated document text from scratch:

```text
Parser::parse(new_content, None)
```

`incremental_parse_document` measures Tree-sitter incremental parsing using the edited old tree:

```text
Parser::parse(new_content, Some(edited_old_tree))
```

Both parser benchmarks exclude parser construction and language setup. The incremental parser benchmark also excludes cloning the old tree and applying `Tree::edit`; those happen in Criterion setup so the measured number is the parser call itself.

## Measurement Boundaries

The benchmark has two separate timing boundaries:

- `update_document` measures semantic update work after parsing has already produced the new tree.
- `parse_document` measures the Tree-sitter parser call itself.

### Update Benchmark Boundary

The update benchmark intentionally does not include parser time. It receives an already-parsed `new_tree`, matching the direct function boundary of `full_update_document` and `incremental_update_document`.

Excluded from `update_document` timing:

- generating the synthetic class text
- reading a real input file when `BENCH_INPUT_FILE` is set
- parsing the old Tree-sitter tree before setup
- parsing the new Tree-sitter tree before setup
- preparing the synthetic changed range
- constructing the initial `ProjectData`
- adding the original document before the update
- looking up the original `class_id`

Included in `update_document` timing:

- the call to `full_update_document` in the full-update benchmark
- the call to `incremental_update_document` in the incremental benchmark
- small benchmark harness overhead, such as `black_box` calls, cheap argument clones, and the fallback assertion

### Parser Benchmark Boundary

The parser benchmark is where parser time is measured.

Excluded from `parse_document` timing:

- generating the synthetic class text
- reading a real input file when `BENCH_INPUT_FILE` is set
- constructing the parser
- setting the parser language
- cloning the old tree
- applying the `InputEdit` to the old tree
- computing changed ranges

Included in `parse_document` timing:

- `Parser::parse(new_content, None)` for full parse
- `Parser::parse(new_content, Some(edited_old_tree))` for incremental parse
- small benchmark harness overhead, such as `black_box` calls and parse-result assertions

Tree-sitter query objects are cached in static `OnceLock<Query>` values. Criterion warmup usually initializes those caches before measurement, so the reported numbers represent steady-state update cost after query compilation has already happened.

This means the benchmark is function-focused. It is not measuring end-to-end LSP latency from text edit receipt through parsing and semantic update together.

The presentation claim should be phrased as:

```text
Given an already-indexed document and a parsed new tree, incremental update is X times faster than rebuilding the document index.
```

For parser numbers, the presentation claim should be phrased as:

```text
For the same edit, Tree-sitter incremental parsing is X times faster than parsing the whole document from scratch.
```

## Incremental Fallback Guard

The benchmark enables the `update-bench` feature. That feature adds a test-only counter around `full_update_document`.

The incremental benchmark does this on every iteration:

1. resets the full-update call counter
2. calls `incremental_update_document`
3. asserts that the full-update counter is still zero

If `incremental_update_document` falls back to `full_update_document`, the benchmark fails instead of reporting a misleading incremental number.

## Interpreting Results

Criterion prints a timing interval like:

```text
time: [9.5029 ms 9.5355 ms 9.5633 ms]
```

Criterion may print times using different units depending on how long the benchmark takes:

| Unit | Meaning | Equivalent |
|---|---|---:|
| `s` | seconds | `1 s = 1,000 ms` |
| `ms` | milliseconds | `1 ms = 0.001 s` |
| `µs` | microseconds | `1 µs = 0.001 ms` |

For example:

```text
110 µs = 0.110 ms
9.54 ms = 0.00954 s
```

The middle value is the estimate to use for a simple comparison. For example, if the full update reports `96.03 ms` and the incremental update reports `9.54 ms`, the speedup is:

```text
96.03 / 9.54 = 10.07x faster
```

For slides, use the same machine, the same benchmark settings, and the Criterion HTML report from a non-smoke run.

For the 42 KB synthetic class used by `BENCH_METHODS=100` and `BENCH_BODY_LINES=12`, a short local run produced:

```text
full_update_document:           [95.066 ms 95.261 ms 95.467 ms]
incremental_update_document:    [9.5192 ms 9.5379 ms 9.5559 ms]
full_parse_document:            [8.8201 ms 8.8478 ms 8.8850 ms]
incremental_parse_document:     [110.22 µs 112.99 µs 119.25 µs]
```

Use your own presentation run as the source of truth, but this gives the expected order of magnitude.

For the 87 KB `test-large-dotted-statements-full.mac` fixture, a short local parser-only run produced:

```text
full_parse_document:            [33.946 ms 34.029 ms 34.112 ms]
incremental_parse_document:     [2.1627 ms 2.1652 ms 2.1690 ms]
```

For the same 87 KB fixture, a short local update-only run produced:

```text
full_update_document:           [2.8057 s 2.8153 s 2.8291 s]
incremental_update_document:    [1.1869 s 1.2009 s 1.2205 s]
```

The real-file update result is still guarded against fallback: if `incremental_update_document` calls `full_update_document`, the benchmark fails instead of reporting the number.
