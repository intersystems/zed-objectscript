use objectscript_core::common::get_member_name_and_range_from_root;
use objectscript_core::config::Config;
use objectscript_core::dependency_tracker::{DependencyGraph, Dependents};
use objectscript_core::global_semantic::GlobalSemanticModel;
use objectscript_core::override_index::OverrideIndex;
use objectscript_core::parse_structures::{ClassId, FileType};
use objectscript_core::workspace::{
    ProjectData, full_update_document_call_count, reset_full_update_document_call_count,
};
use std::collections::HashMap;
use std::env;
use std::hint::black_box;
use std::time::{Duration, Instant};
use tower_lsp::lsp_types::Url;
use tree_sitter::{InputEdit, Parser, Point, Range, Tree};
use tree_sitter_objectscript::LANGUAGE_OBJECTSCRIPT_UDL;

const CLASS_NAME: &str = "Bench.Big";

#[derive(Clone)]
struct PreparedEdit {
    old_content: String,
    new_content: String,
    old_tree: Tree,
    new_tree: Tree,
    changed_ranges: Vec<Range>,
    url: Url,
    new_class_range: Range,
    new_class_name: String,
}

struct BenchConfig {
    methods: usize,
    body_lines: usize,
    iterations: usize,
    warmup: usize,
}

#[derive(Debug)]
struct Stats {
    min: Duration,
    median: Duration,
    mean: Duration,
    max: Duration,
}

fn main() {
    if !cfg!(feature = "update-bench") {
        eprintln!(
            "Run with: cargo run -p objectscript-core --release --features update-bench --example update_bench"
        );
        std::process::exit(2);
    }

    let config = BenchConfig {
        methods: env_usize("BENCH_METHODS", 250),
        body_lines: env_usize("BENCH_BODY_LINES", 12),
        iterations: env_usize("BENCH_ITERS", 5),
        warmup: env_usize("BENCH_WARMUP", 2),
    };

    let prepared = prepare_edit(&config);

    println!(
        "fixture: methods={} body_lines={} bytes={} changed_ranges={}",
        config.methods,
        config.body_lines,
        prepared.old_content.len(),
        prepared.changed_ranges.len()
    );

    let full = collect_samples(&prepared, &config, UpdateKind::Full);
    let incremental = collect_samples(&prepared, &config, UpdateKind::Incremental);

    let full_stats = stats(&full);
    let incremental_stats = stats(&incremental);
    let speedup = full_stats.mean.as_nanos() as f64 / incremental_stats.mean.as_nanos() as f64;

    println!("update_document benchmark");
    println!("  setup excluded: parsing, changed_ranges, initial add_document");
    println!("  samples: {} warmup: {}", config.iterations, config.warmup);
    println!();
    println!(
        "{:<18} {:>12} {:>12} {:>12} {:>12}",
        "strategy", "min", "median", "mean", "max"
    );
    println!(
        "{:<18} {:>12} {:>12} {:>12} {:>12}",
        "full",
        format_duration(full_stats.min),
        format_duration(full_stats.median),
        format_duration(full_stats.mean),
        format_duration(full_stats.max)
    );
    println!(
        "{:<18} {:>12} {:>12} {:>12} {:>12}",
        "incremental",
        format_duration(incremental_stats.min),
        format_duration(incremental_stats.median),
        format_duration(incremental_stats.mean),
        format_duration(incremental_stats.max)
    );
    println!();
    println!("mean speedup: {:.2}x", speedup);
}

#[derive(Clone, Copy)]
enum UpdateKind {
    Full,
    Incremental,
}

fn collect_samples(
    prepared: &PreparedEdit,
    config: &BenchConfig,
    update_kind: UpdateKind,
) -> Vec<Duration> {
    let mut samples = Vec::with_capacity(config.iterations);
    let total_runs = config.warmup + config.iterations;

    for run_idx in 0..total_runs {
        let mut data = build_project_data(prepared);
        let class_id = *data
            .classes
            .get(CLASS_NAME)
            .expect("benchmark class should be indexed before update");

        reset_full_update_document_call_count();
        let start = Instant::now();
        match update_kind {
            UpdateKind::Full => {
                data.full_update_document(
                    black_box(prepared.url.clone()),
                    black_box(&prepared.new_content),
                    black_box(&prepared.new_tree),
                    black_box(FileType::Cls),
                    black_box(class_id),
                    black_box(prepared.new_class_name.clone()),
                    black_box(Some(2)),
                    black_box(prepared.new_class_range),
                );
            }
            UpdateKind::Incremental => {
                data.incremental_update_document(
                    black_box(prepared.url.clone()),
                    black_box(&prepared.new_tree),
                    black_box(FileType::Cls),
                    black_box(2),
                    black_box(&prepared.new_content),
                    black_box(prepared.changed_ranges.clone()),
                    black_box(prepared.new_class_name.clone()),
                    black_box(prepared.new_class_range),
                );
                let fallback_calls = full_update_document_call_count();
                assert_eq!(
                    fallback_calls, 0,
                    "incremental_update_document fell back to full_update_document"
                );
            }
        }
        let elapsed = start.elapsed();
        black_box(data.method_defs.len());

        if run_idx >= config.warmup {
            samples.push(elapsed);
        }
    }

    samples
}

fn build_project_data(prepared: &PreparedEdit) -> ProjectData {
    let mut data = ProjectData {
        config: Config::default(),
        documents: HashMap::new(),
        global_semantic_model: GlobalSemanticModel::new(),
        classes: HashMap::new(),
        method_defs: HashMap::new(),
        property_defs: HashMap::new(),
        parameter_defs: HashMap::new(),
        pub_var_defs: HashMap::new(),
        override_index: OverrideIndex::new(),
        dependent_class_index: Dependents::new(),
        dependency_graph: DependencyGraph::new(),
        unresolved_inheritance_references: HashMap::new(),
        unresolved_method_references: HashMap::new(),
    };

    let (class_range, class_name) = get_member_name_and_range_from_root(
        &prepared.old_content,
        prepared.old_tree.root_node(),
        false,
    )
    .expect("old generated class should have a class name");
    let class_id = ClassId(data.global_semantic_model.next_id());

    data.add_document(
        prepared.url.clone(),
        &prepared.old_content,
        &prepared.old_tree,
        FileType::Cls,
        Some(class_id),
        class_name,
        Some(1),
        class_range,
    );

    data
}

fn prepare_edit(config: &BenchConfig) -> PreparedEdit {
    assert!(config.methods > 1, "BENCH_METHODS must be greater than 1");

    let old_content = make_large_class(config.methods, config.body_lines);
    let target_method = config.methods / 2;
    let old_marker = format!("    Set methodNumber = {target_method}\n");
    let new_marker = format!("    Set methodNumber = {target_method}\n    Write methodNumber\n");

    let marker_start = old_content
        .find(&old_marker)
        .expect("generated class should contain target edit marker");
    let marker_end = marker_start + old_marker.len();
    let new_content = replace_range(&old_content, marker_start, marker_end, &new_marker);

    let mut parser = Parser::new();
    parser
        .set_language(&LANGUAGE_OBJECTSCRIPT_UDL.into())
        .expect("failed to load ObjectScript UDL grammar");

    let old_tree = parser
        .parse(&old_content, None)
        .expect("old generated class should parse");
    assert!(
        !old_tree.root_node().has_error(),
        "old generated class parsed with syntax errors"
    );

    let edit = InputEdit {
        start_byte: marker_start,
        old_end_byte: marker_end,
        new_end_byte: marker_start + new_marker.len(),
        start_position: point_for_byte(&old_content, marker_start),
        old_end_position: point_for_byte(&old_content, marker_end),
        new_end_position: point_for_byte(&new_content, marker_start + new_marker.len()),
    };
    let mut edited_old_tree = old_tree.clone();
    edited_old_tree.edit(&edit);

    let new_tree = parser
        .parse(&new_content, Some(&edited_old_tree))
        .expect("new generated class should parse");
    assert!(
        !new_tree.root_node().has_error(),
        "new generated class parsed with syntax errors"
    );

    let changed_ranges = vec![Range {
        start_byte: marker_start,
        end_byte: marker_start + new_marker.len(),
        start_point: point_for_byte(&new_content, marker_start),
        end_point: point_for_byte(&new_content, marker_start + new_marker.len()),
    }];

    let (new_class_range, new_class_name) =
        get_member_name_and_range_from_root(&new_content, new_tree.root_node(), false)
            .expect("new generated class should have a class name");

    let url = Url::from_file_path(env::temp_dir().join("objectscript-update-bench/Bench.Big.cls"))
        .expect("benchmark URL should be a valid file URL");

    PreparedEdit {
        old_content,
        new_content,
        old_tree,
        new_tree,
        changed_ranges,
        url,
        new_class_range,
        new_class_name,
    }
}

fn make_large_class(methods: usize, body_lines: usize) -> String {
    let mut content = String::new();
    content.push_str("Class Bench.Big\n{\n");

    for method_idx in 0..methods {
        content.push_str(&format!("Method Method{method_idx}() As %Status\n{{\n"));
        content.push_str(&format!("    Set methodNumber = {method_idx}\n"));
        content.push_str("    Set total = 0\n");
        for line_idx in 0..body_lines {
            content.push_str(&format!("    Set total = total + {line_idx}\n"));
        }
        content.push_str("    Write total\n");
        content.push_str("    Quit total\n");
        content.push_str("}\n");
    }

    content.push_str("}\n");
    content
}

fn replace_range(input: &str, start: usize, end: usize, replacement: &str) -> String {
    let mut output = String::with_capacity(input.len() - (end - start) + replacement.len());
    output.push_str(&input[..start]);
    output.push_str(replacement);
    output.push_str(&input[end..]);
    output
}

fn point_for_byte(text: &str, byte_index: usize) -> Point {
    let mut row = 0;
    let mut column = 0;

    for byte in text.as_bytes().iter().take(byte_index) {
        if *byte == b'\n' {
            row += 1;
            column = 0;
        } else {
            column += 1;
        }
    }

    Point { row, column }
}

fn stats(samples: &[Duration]) -> Stats {
    assert!(!samples.is_empty(), "no benchmark samples collected");

    let mut sorted = samples.to_vec();
    sorted.sort();
    let total_nanos: u128 = sorted.iter().map(Duration::as_nanos).sum();
    let mean_nanos = total_nanos / sorted.len() as u128;

    Stats {
        min: sorted[0],
        median: sorted[sorted.len() / 2],
        mean: Duration::from_nanos(mean_nanos as u64),
        max: sorted[sorted.len() - 1],
    }
}

fn format_duration(duration: Duration) -> String {
    let nanos = duration.as_nanos();
    if nanos >= 1_000_000 {
        format!("{:.3} ms", nanos as f64 / 1_000_000.0)
    } else if nanos >= 1_000 {
        format!("{:.3} µs", nanos as f64 / 1_000.0)
    } else {
        format!("{nanos} ns")
    }
}

fn env_usize(name: &str, default: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}
