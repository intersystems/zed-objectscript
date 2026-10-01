use crate::common::{IndexingIssue, PreparedFile, get_paths, prepare_document};
use objectscript_core::common::ts_range_to_lsp_range;

use objectscript_core::parse_structures::IndexParsers;
use objectscript_core::workspace::{BulkIndexDocument, ProjectState};
use parking_lot::RwLock;
use rayon::iter::Either;
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Url};
use tree_sitter::Range;
/// Test harness that mirrors the real Backend for integration testing without a live LSP client.
#[derive(Debug)]
pub(crate) struct BackendTester {
    pub(crate) projects: Arc<RwLock<HashMap<Url, Arc<ProjectState>>>>,
}

impl BackendTester {
    /// Create a new empty BackendTester with no registered projects.
    pub(crate) fn new() -> Self {
        Self {
            projects: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a workspace project by URI.
    pub(crate) fn add_project(&self, uri: Url, state: ProjectState) {
        self.projects.write().insert(uri, Arc::new(state));
    }

    /// Retrieve a project by its workspace URI.
    pub fn get_project(&self, uri: &Url) -> Option<Arc<ProjectState>> {
        self.projects.read().get(uri).cloned()
    }

    #[cfg(test)]
    fn find_parent_workspace(&self, uri: Url) -> Option<Url> {
        let doc_path: PathBuf = uri.to_file_path().ok()?;

        // find longest prefix
        let projects = self.projects.read();

        projects
            .keys()
            .filter_map(|ws_uri| {
                let ws_path = ws_uri.to_file_path().ok()?;
                if doc_path.starts_with(&ws_path) {
                    Some((ws_path.components().count(), ws_uri.clone()))
                } else {
                    None
                }
            })
            .max_by_key(|(depth, _)| *depth)
            .map(|(_, ws_uri)| ws_uri)
    }

    /// Resolve the project that contains the given document URI.
    #[cfg(test)]
    pub(crate) fn get_project_from_document_url(&self, uri: &Url) -> Option<Arc<ProjectState>> {
        let project_url = self.find_parent_workspace(uri.clone())?;
        self.get_project(&project_url)
    }

    /// Parse and index all ObjectScript files under the workspace containing `uri`.
    #[cfg(test)]
    pub(crate) async fn index_workspace(&self, uri: &Url) {
        let Some(project) = self.get_project_from_document_url(&uri) else {
            return;
        };
        let Some(root) = project.root_path() else {
            eprintln!("Couldn't get root");
            return;
        };
        let root = root.to_path_buf();
        self.index_root_into_project(project, root).await;
    }

    /// Append all supported files under `root` to an existing project's index.
    pub(crate) async fn index_workspace_root(&self, project_uri: &Url, root: PathBuf) {
        let Some(project) = self.get_project(project_uri) else {
            return;
        };
        self.index_root_into_project(project, root).await;
    }

    async fn index_root_into_project(&self, project: Arc<ProjectState>, root: PathBuf) {
        let paths = get_paths(&root);
        // Run indexing on Tokio's blocking thread pool
        let handle = tokio::task::spawn_blocking(move || {
            eprintln!("[index] scanning {}", root.display());
            let (prepared, issues): (Vec<(BulkIndexDocument, Range)>, Vec<IndexingIssue>) = paths
                .into_par_iter()
                .map_init(IndexParsers::new, |parsers, (path, file_type)| {
                    prepare_document(path, file_type, parsers)
                })
                .partition_map(|outcome| match outcome {
                    PreparedFile::Ready(document, class_name_range) => {
                        Either::Left((document, class_name_range))
                    }
                    PreparedFile::Failed(issue) => Either::Right(issue),
                });
            for issue in issues {
                eprintln!("[index] skipped file: {issue}");
            }
            let mut duplicate_class_diagnostics = Vec::new();
            let lock_started = std::time::Instant::now();
            eprintln!("[index] waiting for project write lock");
            let mut data = project.data.write();
            eprintln!(
                "[index] acquired project write lock in {:.3?}",
                lock_started.elapsed()
            );
            {
                let mut bulk = data.begin_bulk_index();
                for (document, class_name_def_range) in prepared {
                    let url = document.url.clone();
                    let class_name = document.document.class_name.clone();
                    let lsp_range = ts_range_to_lsp_range(
                        document.document.content.as_str(),
                        class_name_def_range,
                    );
                    let registration = bulk.register(document);
                    if !registration.duplicate_document && registration.duplicate_class {
                        let diagnostic = Diagnostic {
                            range: lsp_range,
                            severity: Some(DiagnosticSeverity::ERROR),
                            code: None,
                            code_description: None,
                            source: Some("ObjectScript".to_string()),
                            message: format!(
                                "A Class named {:?} already exists in this workspace.",
                                &class_name
                            ),
                            related_information: None,
                            tags: None,
                            data: None,
                        };
                        duplicate_class_diagnostics.push((url, diagnostic));
                    }
                }
                bulk.finalize();
            }
            for (url, diagnostic) in duplicate_class_diagnostics {
                data.other_class_diagnostics
                    .entry(url)
                    .or_insert_with(Vec::new)
                    .push(diagnostic);
            }
        });

        // Wait for completion (and handle join errors)
        if let Err(join_err) = handle.await {
            eprintln!("index_workspace_scope spawn_blocking failed: {join_err:?}");
        }
    }
}
