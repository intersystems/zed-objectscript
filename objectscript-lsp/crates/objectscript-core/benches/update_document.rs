use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
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
use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tower_lsp::lsp_types::Url;
use tree_sitter::{InputEdit, Parser, Point, Range, Tree};
use tree_sitter_objectscript::LANGUAGE_OBJECTSCRIPT_UDL;
use tree_sitter_objectscript_routine::LANGUAGE_OBJECTSCRIPT_ROUTINE;

const LARGE_DOTTED_STATEMENTS_FIXTURE: &str =
    "objectscript-tests/local/test-large-dotted-statements-full.mac";

#[derive(Clone)]
struct PreparedEdit {
    old_content: String,
    new_content: String,
    input_edit: InputEdit,
    old_tree: Tree,
    new_tree: Tree,
    changed_ranges: Vec<Range>,
    url: Url,
    file_type: FileType,
    is_rtn: bool,
    new_class_range: Range,
    new_class_name: String,
    new_class_name_def: Range,
}

struct BenchConfig {
    methods: usize,
    body_lines: usize,
}

fn bench_update_document(c: &mut Criterion) {
    let inputs = prepared_inputs_from_env();
    let sample_size = env_usize("BENCH_SAMPLE_SIZE", 10).max(10);
    let warmup_secs = env_usize("BENCH_WARMUP_SECS", 3).max(1);
    let measurement_secs = env_usize("BENCH_MEASUREMENT_SECS", 10).max(1);

    let mut group = c.benchmark_group("update_document");
    group.sample_size(sample_size);
    group.warm_up_time(Duration::from_secs(warmup_secs as u64));
    group.measurement_time(Duration::from_secs(measurement_secs as u64));

    for (input_label, prepared) in inputs {
        group.bench_function(
            BenchmarkId::new("full_update_document", &input_label),
            |bencher| {
                bencher.iter_batched(
                    || {
                        let data = build_project_data(&prepared);
                        let class_id = *data
                            .classes
                            .get(&prepared.new_class_name)
                            .expect("benchmark document should be indexed before update");
                        (data, class_id)
                    },
                    |(mut data, class_id)| {
                        data.full_update_document(
                            black_box(prepared.url.clone()),
                            black_box(&prepared.new_content),
                            black_box(&prepared.new_tree),
                            black_box(prepared.file_type),
                            black_box(class_id),
                            black_box(prepared.new_class_name.clone()),
                            black_box(Some(2)),
                            black_box(prepared.new_class_range),
                        );
                        black_box(data.method_defs.len());
                        data
                    },
                    BatchSize::PerIteration,
                );
            },
        );

        group.bench_function(
            BenchmarkId::new("incremental_update_document", &input_label),
            |bencher| {
                bencher.iter_batched(
                    || build_project_data(&prepared),
                    |mut data| {
                        reset_full_update_document_call_count();
                        data.incremental_update_document(
                            black_box(prepared.url.clone()),
                            black_box(&prepared.new_tree),
                            black_box(prepared.file_type),
                            black_box(2),
                            black_box(&prepared.new_content),
                            black_box(prepared.changed_ranges.clone()),
                            black_box(prepared.new_class_name.clone()),
                            black_box(prepared.new_class_range),
                            black_box(prepared.new_class_name_def),
                        );
                        assert_eq!(
                            full_update_document_call_count(),
                            0,
                            "incremental_update_document fell back to full_update_document"
                        );
                        black_box(data.method_defs.len());
                        data
                    },
                    BatchSize::PerIteration,
                );
            },
        );
    }

    group.finish();
}

fn bench_parse_document(c: &mut Criterion) {
    let inputs = prepared_inputs_from_env();
    let sample_size = env_usize("BENCH_SAMPLE_SIZE", 10).max(10);
    let warmup_secs = env_usize("BENCH_WARMUP_SECS", 3).max(1);
    let measurement_secs = env_usize("BENCH_MEASUREMENT_SECS", 10).max(1);

    let mut group = c.benchmark_group("parse_document");
    group.sample_size(sample_size);
    group.warm_up_time(Duration::from_secs(warmup_secs as u64));
    group.measurement_time(Duration::from_secs(measurement_secs as u64));

    for (input_label, prepared) in inputs {
        group.bench_function(
            BenchmarkId::new("full_parse_document", &input_label),
            |bencher| {
                bencher.iter_batched(
                    || new_parser(prepared.file_type),
                    |mut parser| {
                        let tree = parser
                            .parse(black_box(prepared.new_content.as_str()), None)
                            .expect("new generated class should parse from scratch");
                        assert!(
                            !tree.root_node().has_error(),
                            "full parse produced syntax errors"
                        );
                        black_box(tree.root_node().end_byte());
                        tree
                    },
                    BatchSize::PerIteration,
                );
            },
        );

        group.bench_function(
            BenchmarkId::new("incremental_parse_document", &input_label),
            |bencher| {
                bencher.iter_batched(
                    || {
                        let mut edited_old_tree = prepared.old_tree.clone();
                        edited_old_tree.edit(&prepared.input_edit);
                        (new_parser(prepared.file_type), edited_old_tree)
                    },
                    |(mut parser, edited_old_tree)| {
                        let tree = parser
                            .parse(
                                black_box(prepared.new_content.as_str()),
                                Some(black_box(&edited_old_tree)),
                            )
                            .expect("new generated class should parse incrementally");
                        assert!(
                            !tree.root_node().has_error(),
                            "incremental parse produced syntax errors"
                        );
                        black_box(tree.root_node().end_byte());
                        tree
                    },
                    BatchSize::PerIteration,
                );
            },
        );
    }

    group.finish();
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
        inheritance_diagonstics: HashMap::new(),
        method_reference_diagnostics: HashMap::new(),
        other_class_diagnostics: HashMap::new(),
    };

    let (class_range, class_name, _class_name_def) = get_member_name_and_range_from_root(
        &prepared.old_content,
        prepared.old_tree.root_node(),
        prepared.is_rtn,
    )
    .expect("old generated class should have a class name");
    let class_id = ClassId(data.global_semantic_model.next_id());

    data.add_document(
        prepared.url.clone(),
        &prepared.old_content,
        &prepared.old_tree,
        prepared.file_type,
        Some(class_id),
        class_name,
        Some(1),
        class_range,
    );

    data
}

fn prepared_inputs_from_env() -> Vec<(String, PreparedEdit)> {
    if let Ok(path) = env::var("BENCH_INPUT_FILE") {
        let path = PathBuf::from(path);
        let prepared = prepare_file_edit(&path);
        let label = format!(
            "file_{}_bytes_{}",
            sanitize_label(
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("input")
            ),
            prepared.old_content.len()
        );
        return vec![(label, prepared)];
    }

    if let Ok(preset) = env::var("BENCH_INPUT_PRESET") {
        let path = input_preset_path(&preset);
        let prepared = prepare_file_edit(&path);
        let label = format!(
            "preset_{}_bytes_{}",
            sanitize_label(&preset),
            prepared.old_content.len()
        );
        return vec![(label, prepared)];
    }

    bench_configs_from_env()
        .into_iter()
        .map(|config| {
            let prepared = prepare_synthetic_edit(&config);
            let label = format!(
                "methods_{}_body_lines_{}_bytes_{}",
                config.methods,
                config.body_lines,
                prepared.old_content.len()
            );
            (label, prepared)
        })
        .collect()
}

fn input_preset_path(preset: &str) -> PathBuf {
    match preset {
        "large_dotted_statements" | "large-dotted-statements" | "large_dotted" => {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join(LARGE_DOTTED_STATEMENTS_FIXTURE)
        }
        other => {
            panic!("unsupported BENCH_INPUT_PRESET {other:?}; expected \"large_dotted_statements\"")
        }
    }
}

fn prepare_synthetic_edit(config: &BenchConfig) -> PreparedEdit {
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

    let file_type = FileType::Cls;
    let is_rtn = false;
    let mut parser = new_parser(file_type);

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

    // The benchmark intentionally measures update-document work, not Tree-sitter's
    // changed-range computation. Use the precise range of the synthetic statement
    // insertion so incremental_update_document receives a small changed scope.
    let changed_ranges = vec![Range {
        start_byte: marker_start,
        end_byte: marker_start + new_marker.len(),
        start_point: point_for_byte(&new_content, marker_start),
        end_point: point_for_byte(&new_content, marker_start + new_marker.len()),
    }];

    let (new_class_range, new_class_name, new_class_name_def) =
        get_member_name_and_range_from_root(&new_content, new_tree.root_node(), is_rtn)
            .expect("new generated class should have a class name");

    let url = Url::from_file_path(env::temp_dir().join("objectscript-update-bench/Bench.Big.cls"))
        .expect("benchmark URL should be a valid file URL");

    PreparedEdit {
        old_content,
        new_content,
        input_edit: edit,
        old_tree,
        new_tree,
        changed_ranges,
        url,
        file_type,
        is_rtn,
        new_class_range,
        new_class_name,
        new_class_name_def,
    }
}

fn prepare_file_edit(path: &Path) -> PreparedEdit {
    let file_type = infer_file_type(path);
    let is_rtn = file_type == FileType::Routine;
    let old_content = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read benchmark input file {path:?}: {error}"));
    let (insert_at, insertion) = synthetic_file_edit(&old_content, file_type);
    let new_content = replace_range(&old_content, insert_at, insert_at, &insertion);

    let mut parser = new_parser(file_type);
    let old_tree = parser
        .parse(&old_content, None)
        .unwrap_or_else(|| panic!("old benchmark input file {path:?} should parse"));
    assert!(
        !old_tree.root_node().has_error(),
        "old benchmark input file {path:?} parsed with syntax errors"
    );

    let edit = InputEdit {
        start_byte: insert_at,
        old_end_byte: insert_at,
        new_end_byte: insert_at + insertion.len(),
        start_position: point_for_byte(&old_content, insert_at),
        old_end_position: point_for_byte(&old_content, insert_at),
        new_end_position: point_for_byte(&new_content, insert_at + insertion.len()),
    };
    let mut edited_old_tree = old_tree.clone();
    edited_old_tree.edit(&edit);

    let new_tree = parser
        .parse(&new_content, Some(&edited_old_tree))
        .unwrap_or_else(|| panic!("new benchmark input file {path:?} should parse"));
    assert!(
        !new_tree.root_node().has_error(),
        "new benchmark input file {path:?} parsed with syntax errors"
    );

    let changed_ranges = vec![Range {
        start_byte: insert_at,
        end_byte: insert_at + insertion.len(),
        start_point: point_for_byte(&new_content, insert_at),
        end_point: point_for_byte(&new_content, insert_at + insertion.len()),
    }];

    let (new_class_range, new_class_name, new_class_name_def) =
        get_member_name_and_range_from_root(&new_content, new_tree.root_node(), is_rtn)
            .unwrap_or_else(|| panic!("benchmark input file {path:?} should have a member name"));

    let url = Url::from_file_path(path)
        .unwrap_or_else(|_| panic!("benchmark input path {path:?} should be a valid file URL"));

    PreparedEdit {
        old_content,
        new_content,
        input_edit: edit,
        old_tree,
        new_tree,
        changed_ranges,
        url,
        file_type,
        is_rtn,
        new_class_range,
        new_class_name,
        new_class_name_def,
    }
}

fn infer_file_type(path: &Path) -> FileType {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("cls") => FileType::Cls,
        Some("mac") | Some("inc") | Some("rtn") | Some("int") => FileType::Routine,
        other => panic!(
            "unsupported BENCH_INPUT_FILE extension {other:?}; expected .cls or routine file"
        ),
    }
}

fn synthetic_file_edit(content: &str, file_type: FileType) -> (usize, String) {
    match file_type {
        FileType::Routine => routine_file_edit(content),
        FileType::Cls => class_file_edit(content),
        FileType::Xml => panic!("XML benchmark inputs are not supported"),
    }
}

fn routine_file_edit(content: &str) -> (usize, String) {
    let line_start = content[..content.len() / 2]
        .rfind('\n')
        .map_or(0, |idx| idx + 1);
    let line_end = content[line_start..]
        .find('\n')
        .map_or(content.len(), |idx| line_start + idx + 1);
    let line = &content[line_start..line_end];
    let prefix: String = line
        .chars()
        .take_while(|ch| matches!(ch, ' ' | '\t' | '.'))
        .collect();
    let prefix = if prefix.is_empty() {
        "\t".to_string()
    } else {
        prefix
    };
    (line_end, format!("{prefix}; criterion benchmark edit\n"))
}

fn class_file_edit(content: &str) -> (usize, String) {
    let line_start = content[..content.len() / 2]
        .rfind('\n')
        .map_or(0, |idx| idx + 1);
    let line_end = content[line_start..]
        .find('\n')
        .map_or(content.len(), |idx| line_start + idx + 1);
    (line_end, "    // criterion benchmark edit\n".to_string())
}

fn new_parser(file_type: FileType) -> Parser {
    let mut parser = Parser::new();
    match file_type {
        FileType::Cls => parser
            .set_language(&LANGUAGE_OBJECTSCRIPT_UDL.into())
            .expect("failed to load ObjectScript UDL grammar"),
        FileType::Routine => parser
            .set_language(&LANGUAGE_OBJECTSCRIPT_ROUTINE.into())
            .expect("failed to load ObjectScript routine grammar"),
        FileType::Xml => panic!("XML benchmark inputs are not supported"),
    }
    parser
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

fn bench_configs_from_env() -> Vec<BenchConfig> {
    let body_lines = env_usize("BENCH_BODY_LINES", 12);
    let methods = env::var("BENCH_METHODS_LIST")
        .ok()
        .map(|value| {
            value
                .split(',')
                .filter_map(|part| part.trim().parse::<usize>().ok())
                .collect::<Vec<_>>()
        })
        .filter(|values| !values.is_empty())
        .unwrap_or_else(|| vec![env_usize("BENCH_METHODS", 100)]);

    methods
        .into_iter()
        .map(|methods| BenchConfig {
            methods,
            body_lines,
        })
        .collect()
}

fn env_usize(name: &str, default: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn sanitize_label(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

criterion_group! {
    name = benches;
    config = Criterion::default().configure_from_args();
    targets = bench_update_document, bench_parse_document
}
criterion_main!(benches);
