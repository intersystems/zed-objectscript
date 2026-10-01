//! CH-009 cold-workspace construction benchmark.
//!
//! This is deliberately an example rather than production instrumentation: all
//! clocks and input generation disappear from normal library builds.

use objectscript_core::common::{get_member_name_and_range_from_root, initial_build_scope_tree};
use objectscript_core::config::Config;
use objectscript_core::dependency_tracker::{DependencyGraph, Dependents};
use objectscript_core::document::Document;
use objectscript_core::global_semantic::GlobalSemanticModel;
use objectscript_core::override_index::OverrideIndex;
use objectscript_core::parse_structures::{ClassId, FileType};
use objectscript_core::scope_tree::ScopeTree;
use objectscript_core::workspace::{BulkIndexDocument, ProjectData};
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tower_lsp::lsp_types::Url;
use tree_sitter::{Parser, Range, Tree};
use tree_sitter_objectscript::LANGUAGE_OBJECTSCRIPT_UDL;
use tree_sitter_objectscript_routine::LANGUAGE_OBJECTSCRIPT_ROUTINE;
use walkdir::WalkDir;

#[derive(Clone)]
struct Source {
    path: PathBuf,
    content: String,
    file_type: FileType,
}

#[derive(Clone)]
struct Parsed {
    url: Url,
    content: String,
    tree: Tree,
    file_type: FileType,
    class_name: String,
    class_range: Range,
}

#[derive(Default)]
struct Timings {
    parse: Duration,
    declaration: Duration,
    finalization: Duration,
    total: Duration,
}

fn main() {
    let sources = match env::args_os().nth(1) {
        Some(path) => load_workspace(Path::new(&path)),
        None => generated_workspace(),
    };
    assert!(
        !sources.is_empty(),
        "workspace contains no supported source files"
    );

    println!(
        "CH-009 cold workspace benchmark: {} documents, {} bytes",
        sources.len(),
        sources
            .iter()
            .map(|source| source.content.len())
            .sum::<usize>()
    );
    let parse_started = Instant::now();
    let parsed = parse_sources(&sources);
    let parse = parse_started.elapsed();
    print_timings("sequential add_document", run_sequential(&parsed, parse));
    print_timings("BulkWorkspaceIndex", run_bulk(&parsed, parse));
}

fn run_sequential(parsed: &[Parsed], parse: Duration) -> Timings {
    let declaration = Instant::now();
    let mut data = empty_project_data();
    for input in parsed {
        let class_id = (input.file_type != FileType::Xml)
            .then(|| ClassId(data.global_semantic_model.next_id()));
        data.add_document(
            input.url.clone(),
            &input.content,
            &input.tree,
            input.file_type,
            class_id,
            input.class_name.clone(),
            None,
            input.class_range,
        );
    }
    let declaration = declaration.elapsed();
    Timings {
        parse,
        declaration,
        finalization: Duration::ZERO,
        total: parse + declaration,
    }
}

fn run_bulk(parsed: &[Parsed], parse: Duration) -> Timings {
    let declaration = Instant::now();
    let mut data = empty_project_data();
    let mut bulk = data.begin_bulk_index();
    for input in parsed {
        let scope_tree = if input.file_type == FileType::Xml {
            ScopeTree::new()
        } else {
            initial_build_scope_tree(
                &input.tree,
                &input.content,
                input.file_type == FileType::Routine,
            )
        };
        bulk.register(BulkIndexDocument {
            url: input.url.clone(),
            class_range: input.class_range,
            document: Document::new(
                input.content.clone(),
                input.tree.clone(),
                input.file_type,
                input.class_name.clone(),
                None,
                scope_tree,
                None,
            ),
        });
    }
    let declaration = declaration.elapsed();
    let finalization = Instant::now();
    bulk.finalize();
    let finalization = finalization.elapsed();
    Timings {
        parse,
        declaration,
        finalization,
        total: parse + declaration + finalization,
    }
}

fn parse_sources(sources: &[Source]) -> Vec<Parsed> {
    let mut cls_parser = parser(FileType::Cls);
    let mut routine_parser = parser(FileType::Routine);
    sources
        .iter()
        .map(|source| {
            let parser = match source.file_type {
                FileType::Cls => &mut cls_parser,
                FileType::Routine => &mut routine_parser,
                FileType::Xml => unreachable!("XML is not included in this benchmark"),
            };
            let tree = parser
                .parse(&source.content, None)
                .unwrap_or_else(|| panic!("failed to parse {:?}", source.path));
            let is_routine = source.file_type == FileType::Routine;
            let (class_range, class_name, _) =
                get_member_name_and_range_from_root(&source.content, tree.root_node(), is_routine)
                    .unwrap_or_else(|| {
                        panic!("source has no class/routine name: {:?}", source.path)
                    });
            let absolute = source
                .path
                .canonicalize()
                .unwrap_or_else(|_| source.path.clone());
            let url = Url::from_file_path(&absolute)
                .unwrap_or_else(|_| panic!("cannot create file URL for {:?}", source.path));
            Parsed {
                url,
                content: source.content.clone(),
                tree,
                file_type: source.file_type,
                class_name,
                class_range,
            }
        })
        .collect()
}

fn parser(file_type: FileType) -> Parser {
    let mut parser = Parser::new();
    let language = match file_type {
        FileType::Cls => LANGUAGE_OBJECTSCRIPT_UDL,
        FileType::Routine => LANGUAGE_OBJECTSCRIPT_ROUTINE,
        FileType::Xml => unreachable!(),
    };
    parser.set_language(&language.into()).expect("load grammar");
    parser
}

fn generated_workspace() -> Vec<Source> {
    let mut sources = Vec::new();
    // Reverse inheritance order intentionally makes the old incremental path
    // exercise its unresolved-reference repair work.
    for number in (0..40).rev() {
        let parent = if number == 0 {
            String::new()
        } else {
            format!(" Extends Bench.Class{}", number - 1)
        };
        let content = format!(
            "Class Bench.Class{number}{parent}\n{{\nProperty Value As %Integer;\nMethod Work(arg As %Integer) As %Integer\n{{\n Set local = arg + {number}\n Quit local\n}}\n}}\n"
        );
        sources.push(Source {
            path: env::temp_dir().join(format!("ch009/Bench.Class{number}.cls")),
            content,
            file_type: FileType::Cls,
        });
    }
    for number in 0..10 {
        sources.push(Source {
            path: env::temp_dir().join(format!("ch009/BenchRoutine{number}.mac")),
            content: format!(
                "ROUTINE BenchRoutine{number}\n\nMain(arg)\n Set local=arg+{number}\n Quit local\n\nSecond\n Do Main(1)\n Quit\n"
            ),
            file_type: FileType::Routine,
        });
    }
    sources
}

fn load_workspace(root: &Path) -> Vec<Source> {
    WalkDir::new(root)
        .sort_by_file_name()
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| {
            let extension = entry.path().extension()?.to_str()?.to_ascii_lowercase();
            let file_type = match extension.as_str() {
                "cls" => FileType::Cls,
                "mac" | "int" | "inc" | "rtn" => FileType::Routine,
                _ => return None,
            };
            Some(Source {
                path: entry.path().to_owned(),
                content: fs::read_to_string(entry.path())
                    .unwrap_or_else(|error| panic!("failed to read {:?}: {error}", entry.path())),
                file_type,
            })
        })
        .collect()
}

fn print_timings(label: &str, timings: Timings) {
    println!("\n{label}");
    println!("  parse:        {:>10.3} ms", millis(timings.parse));
    println!("  declaration:  {:>10.3} ms", millis(timings.declaration));
    println!("  finalization: {:>10.3} ms", millis(timings.finalization));
    println!("  total:        {:>10.3} ms", millis(timings.total));
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn empty_project_data() -> ProjectData {
    ProjectData {
        config: Config::default(),
        documents: HashMap::new(),
        global_semantic_model: GlobalSemanticModel::new(),
        classes: HashMap::new(),
        method_defs: HashMap::new(),
        pub_var_defs: HashMap::new(),
        parameter_defs: HashMap::new(),
        property_defs: HashMap::new(),
        relationship_defs: HashMap::new(),
        foreignkey_defs: HashMap::new(),
        query_defs: HashMap::new(),
        index_defs: HashMap::new(),
        trigger_defs: HashMap::new(),
        xdata_defs: HashMap::new(),
        projection_defs: HashMap::new(),
        storage_defs: HashMap::new(),
        override_index: OverrideIndex::new(),
        dependent_class_index: Dependents::new(),
        dependency_graph: DependencyGraph::new(),
        unresolved_inheritance_references: HashMap::new(),
        unresolved_method_references: HashMap::new(),
        inheritance_diagonstics: HashMap::new(),
        method_reference_diagnostics: HashMap::new(),
        other_class_diagnostics: HashMap::new(),
        sys_classes: HashSet::new(),
        sys_classes_overwritten: HashSet::new(),
    }
}
