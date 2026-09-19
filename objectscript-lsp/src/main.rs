use crate::server::BackendWrapper;
use objectscript_core::workspace::ProjectState;
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

    let iris_current_version_project_root =
        PathBuf::from_str("/Users/hkimura/objectscript-dependencies/IRIS/202502/").unwrap();
    let iris_new_version_project_root =
        PathBuf::from_str("/Users/hkimura/objectscript-dependencies/IRIS/202602/").unwrap();

    // index both IRIS versions concurrently
    let ((old_iris_backend, old_iris_uri), (new_iris_backend, new_iris_uri)) = tokio::join!(
        setup_backend_and_workspace(iris_current_version_project_root),
        setup_backend_and_workspace(iris_new_version_project_root),
    );

    let _old_iris_workspace = old_iris_backend
        .get_project(&old_iris_uri)
        .expect("missing project for old iris version");

    let _new_iris_workspace = new_iris_backend
        .get_project(&new_iris_uri)
        .expect("missing project for new iris version");
}
