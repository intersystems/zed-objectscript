use crate::common::{IndexingIssue, PreparedFile, get_paths, prepare_document};
use objectscript_core::common::ts_range_to_lsp_range;
use objectscript_core::parse_structures::{FileType, IndexParsers};
use objectscript_core::workspace::{BulkIndexDocument, ProjectState};
use parking_lot::RwLock;
use rayon::iter::Either;
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tower_lsp::Client;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, MessageType, Url};
use tree_sitter::Range;

/// Arc-wrapped backend providing the LSP language server implementation.
pub struct BackendWrapper(pub(crate) Arc<Backend>);
impl BackendWrapper {
    /// Create a reference-counted backend wrapper around a new `Backend`.
    pub fn new(client: Client) -> Self {
        Self(Arc::new(Backend::new(client)))
    }
}

pub(crate) struct Backend {
    /// LSP Client.
    pub(crate) client: Client,
    /// Stores Url -> ProjectState for each Workspace.
    pub(crate) projects: Arc<RwLock<HashMap<Url, Arc<ProjectState>>>>,
    pub(crate) diagnostic_refresh_supported: AtomicBool,
    pub(crate) configuration_supported: AtomicBool,
}

impl Backend {
    /// Construct a new backend with an empty projects map.
    pub(crate) fn new(client: Client) -> Self {
        Self {
            client,
            projects: Arc::new(RwLock::new(HashMap::new())),
            diagnostic_refresh_supported: AtomicBool::new(false),
            configuration_supported: AtomicBool::new(false),
        }
    }

    pub(crate) fn set_diagnostic_refresh_supported(&self, supported: bool) {
        self.diagnostic_refresh_supported
            .store(supported, Ordering::Relaxed);
    }

    pub(crate) fn set_configuration_supported(&self, supported: bool) {
        self.configuration_supported
            .store(supported, Ordering::Relaxed);
    }

    pub(crate) async fn refresh_workspace_diagnostics_if_supported(&self) {
        if self.diagnostic_refresh_supported.load(Ordering::Relaxed) {
            let _ = self.client.workspace_diagnostic_refresh().await;
        }
    }

    /// Register a workspace (project) and its initial `ProjectState` by workspace URI.
    pub(crate) fn add_project(&self, uri: Url, state: ProjectState) {
        self.projects.write().insert(uri, Arc::new(state));
    }

    /// Fetch a project by its workspace URI.
    ///
    /// Returns a cloned `Arc` to the project state, or `None` if the workspace is not registered.
    pub fn get_project(&self, uri: &Url) -> Option<Arc<ProjectState>> {
        let result = self.projects.read().get(uri).cloned();
        result
    }

    /// Find the workspace URI that most specifically contains the given document URI.
    ///
    /// Converts the document URI to a file path and selects the registered workspace whose path is
    /// the longest prefix of that document path (i.e., the deepest matching workspace).
    fn find_parent_workspace(&self, uri: Url) -> Option<Url> {
        let doc_path: PathBuf = uri.to_file_path().ok()?;

        // find longest prefix
        let projects = self.projects.read();

        let parent = projects
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
            .map(|(_, ws_uri)| ws_uri);
        parent
    }

    /// Resolve the `ProjectState` associated with a document URI.
    ///
    /// This first finds the containing workspace (if any), then returns that project's state.
    pub(crate) fn get_project_from_document_url(&self, uri: &Url) -> Option<Arc<ProjectState>> {
        let project_url = self.find_parent_workspace(uri.clone())?;
        let result = self.get_project(&project_url);
        result
    }

    /// Handle an LSP "didOpen" for a document by forwarding it to the owning project.
    ///
    /// If no workspace contains `uri`, this is a no-op.
    pub fn handle_did_open(&self, uri: Url, text: String, file_type: FileType, version: i32) {
        let Some(project) = self.get_project_from_document_url(&uri) else {
            return;
        };
        project.handle_document_opened(uri, text, file_type, version);
    }

    /// Index all supported ObjectScript and XML files under the workspace root containing `uri`.
    ///
    /// This runs filesystem walking and parsing on Tokio's blocking thread pool. Each file is read,
    /// parsed with the appropriate Tree-sitter grammar, and inserted into the project's document
    /// store if absent. After the scan, inheritance and variable information is built once.
    pub(crate) async fn index_workspace(&self, uri: &Url) {
        let Some(project_uri) = self.find_parent_workspace(uri.clone()) else {
            eprintln!(
                "Failed to get project from document with url: {:?}",
                uri.path()
            );
            return;
        };
        let Some(project) = self.get_project(&project_uri) else {
            return;
        };
        let Some(root) = project.root_path() else {
            self.client
                .log_message(MessageType::ERROR, "project root path doesn't exist")
                .await;
            return;
        };
        let root = root.to_path_buf();
        self.index_workspace_root(&project_uri, root).await;
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
            let index_started = std::time::Instant::now();
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
            let lock_started = std::time::Instant::now();
            eprintln!("[index] waiting for project write lock");
            let mut data = project.data.write();
            eprintln!(
                "[index] acquired project write lock in {:.3?}",
                lock_started.elapsed()
            );
            let mut duplicate_class_diagnostics = Vec::new();
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
            eprintln!(
                "[index] finished {} in {:.3?}",
                root.display(),
                index_started.elapsed()
            );
        });
        // Wait for completion (and handle join errors)
        if let Err(join_err) = handle.await {
            eprintln!("Error: index_workspace_scope spawn_blocking failed: {join_err:?}");
        }
        self.refresh_workspace_diagnostics_if_supported().await;
    }
}
