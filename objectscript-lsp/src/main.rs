use crate::server::BackendWrapper;
use objectscript_core::workspace::ProjectState;
use objectscript_core::workspace_diff::{
    ClassComparison, compare_classes_parallel, compare_workspaces_parallel,
};
use std::collections::BTreeSet;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use tower_lsp::{LspService, Server};
// #[cfg(test)]
mod backend_testing;
use crate::backend_testing::BackendTester;
use tower_lsp::lsp_types::Url;
mod common;
mod lsp;
mod server;
use rayon::ThreadPoolBuilder;
use std::str::FromStr;
#[cfg(test)]
mod test;

async fn setup_backend_and_workspace(project_root: PathBuf) -> (BackendTester, Url) {
    let state = ProjectState::new();
    if state
        .project_root_path
        .set(Some(project_root.clone()))
        .is_err()
    {
        eprintln!("failed to set the root path");
    }
    let backend = BackendTester::new();
    let uri = Url::from_file_path(&project_root).unwrap();
    backend.add_project(uri.clone(), state);
    let _ = backend.index_workspace_root(&uri, project_root).await;
    (backend, uri)
}

fn write_comparison_report(
    output_path: &PathBuf,
    baseline_root: &PathBuf,
    target_root: &PathBuf,
    customer_root: &PathBuf,
    comparison_scope: &str,
    direct_dependencies: &BTreeSet<String>,
    transitive_dependencies: &BTreeSet<String>,
    comparisons: &[ClassComparison],
) -> std::io::Result<()> {
    let file = std::fs::File::create(output_path)?;
    let mut output = BufWriter::new(file);
    let mut unchanged = 0;
    let mut added = 0;
    let mut removed = 0;
    let mut changed = 0;
    let mut unavailable = 0;
    for comparison in comparisons {
        match comparison {
            ClassComparison::Unchanged { .. } => unchanged += 1,
            ClassComparison::Added { .. } => added += 1,
            ClassComparison::Removed { .. } => removed += 1,
            ClassComparison::Changed(_) => changed += 1,
            ClassComparison::SnapshotUnavailable { .. } => unavailable += 1,
        }
    }

    writeln!(output, "ObjectScript SYS dependency comparison")?;
    writeln!(output, "baseline SYS: {}", baseline_root.display())?;
    writeln!(output, "target SYS: {}", target_root.display())?;
    writeln!(output, "customer: {}", customer_root.display())?;
    writeln!(output, "scope: {comparison_scope}")?;
    writeln!(output)?;
    writeln!(output, "direct dependencies: {}", direct_dependencies.len())?;
    for class_name in direct_dependencies {
        writeln!(output, "  {class_name}")?;
    }
    writeln!(
        output,
        "transitive-only dependencies: {}",
        transitive_dependencies.len()
    )?;
    for class_name in transitive_dependencies {
        writeln!(output, "  {class_name}")?;
    }
    writeln!(output)?;
    writeln!(output, "comparison summary")?;
    writeln!(output, "  unchanged: {unchanged}")?;
    writeln!(output, "  changed: {changed}")?;
    writeln!(output, "  added: {added}")?;
    writeln!(output, "  removed: {removed}")?;
    writeln!(output, "  snapshot unavailable: {unavailable}")?;
    writeln!(output)?;
    writeln!(output, "differences")?;
    for comparison in comparisons {
        if !matches!(comparison, ClassComparison::Unchanged { .. }) {
            writeln!(output, "{comparison:#?}")?;
        }
    }
    output.flush()
}

#[tokio::main]
async fn main() {
    let worker_count = std::env::var("OBJECTSCRIPT_INDEX_WORKERS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|count| *count > 0)
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|count| count.get())
                .unwrap_or(4)
                .min(8)
        });
    ThreadPoolBuilder::new()
        .num_threads(worker_count)
        .build_global()
        .expect("Rayon pool must be configured once");
    if std::env::args().any(|arg| arg == "--server") {
        let stdin = tokio::io::stdin();
        let stdout = tokio::io::stdout();
        let (service, socket) = LspService::build(|client| BackendWrapper::new(client)).finish();
        Server::new(stdin, stdout, socket).serve(service).await;
        return;
    }

    let iris_current_version_project_root = std::env::var("OBJECTSCRIPT_BASELINE_SYS_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from_str("/Users/hkimura/objectscript-dependencies/IRIS/202502/").unwrap()
        });
    let iris_new_version_project_root = std::env::var("OBJECTSCRIPT_TARGET_SYS_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from_str("/Users/hkimura/objectscript-dependencies/IRIS/202602/").unwrap()
        });
    let customer_project_root = std::env::var("OBJECTSCRIPT_CUSTOMER_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/Users/hkimura/objectscript-dependencies/customer/"));
    let comparison_output = std::env::var("OBJECTSCRIPT_COMPARISON_OUTPUT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from("/Users/hkimura/objectscript-dependencies/class-comparison-report.txt")
        });
    let compare_all_classes = std::env::var("OBJECTSCRIPT_COMPARE_ALL")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false);

    let baseline_sys_root = iris_current_version_project_root.clone();
    let baseline_customer_root = customer_project_root.clone();
    let baseline_compare_all = compare_all_classes;
    let baseline_index = async {
        let (backend, uri) = setup_backend_and_workspace(baseline_sys_root).await;
        if !baseline_compare_all {
            let project = backend
                .get_project(&uri)
                .expect("missing baseline IRIS project");
            project.data.write().mark_current_classes_as_sys();
            backend
                .index_workspace_root(&uri, baseline_customer_root)
                .await;
        }
        (backend, uri)
    };

    // Index the baseline SYS + customer project and target SYS project concurrently.
    let ((old_iris_backend, old_iris_uri), (new_iris_backend, new_iris_uri)) = tokio::join!(
        baseline_index,
        setup_backend_and_workspace(iris_new_version_project_root.clone()),
    );

    let old_iris_workspace = old_iris_backend
        .get_project(&old_iris_uri)
        .expect("missing project for old iris version");

    let new_iris_workspace = new_iris_backend
        .get_project(&new_iris_uri)
        .expect("missing project for new iris version");

    let old_data = old_iris_workspace.data.read();
    let new_data = new_iris_workspace.data.read();
    let (direct, transitive) = old_data.get_sys_dependencies();
    let direct: BTreeSet<String> = direct.into_iter().collect();
    let transitive: BTreeSet<String> = transitive.into_iter().collect();
    let dependency_names: Vec<String> = direct.iter().chain(transitive.iter()).cloned().collect();
    let (comparison_scope, mut comparisons) = if compare_all_classes {
        (
            "all classes in either workspace",
            compare_workspaces_parallel(&old_data, &new_data),
        )
    } else {
        (
            "customer SYS dependencies",
            compare_classes_parallel(&dependency_names, &old_data, &new_data),
        )
    };
    comparisons.sort_by(|left, right| left.class_name().cmp(right.class_name()));
    write_comparison_report(
        &comparison_output,
        &iris_current_version_project_root,
        &iris_new_version_project_root,
        &customer_project_root,
        comparison_scope,
        &direct,
        &transitive,
        &comparisons,
    )
    .unwrap_or_else(|error| {
        panic!(
            "failed to write comparison report {}: {error}",
            comparison_output.display()
        )
    });
    eprintln!(
        "[compare] wrote {} class comparisons to {}",
        comparisons.len(),
        comparison_output.display()
    );
}
