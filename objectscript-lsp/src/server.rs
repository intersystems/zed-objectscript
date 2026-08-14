use objectscript_core::common::get_member_name_and_range_from_root;
use objectscript_core::parse_structures::FileType;
use objectscript_core::workspace::ProjectState;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tower_lsp::Client;
use tower_lsp::lsp_types::{MessageType, Url};
use tree_sitter::Parser;
use tree_sitter_objectscript::LANGUAGE_OBJECTSCRIPT_UDL;
use tree_sitter_objectscript_routine::LANGUAGE_OBJECTSCRIPT_ROUTINE;
use walkdir::WalkDir;

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
}

impl Backend {
    /// Construct a new backend with an empty projects map.
    pub(crate) fn new(client: Client) -> Self {
        Self {
            client,
            projects: Arc::new(RwLock::new(HashMap::new())),
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

    /// Index all `.cls` and `.inc` files under the workspace root containing `uri`.
    ///
    /// This runs filesystem walking and parsing on Tokio's blocking thread pool. Each file is read,
    /// parsed with the appropriate Tree-sitter grammar, and inserted into the project's document
    /// store if absent. After the scan, inheritance and variable information is built once.
    pub(crate) async fn index_workspace(&self, uri: &Url) {
        let Some(project) = self.get_project_from_document_url(&uri) else {
            eprintln!(
                "Failed to get project from document with url: {:?}",
                uri.path()
            );
            return;
        };
        let Some(root) = project.root_path() else {
            self.client
                .log_message(MessageType::ERROR, "project root path doesn't exist")
                .await;
            return;
        };
        let root = root.to_path_buf();
        // Run indexing on Tokio's blocking thread pool
        let handle = tokio::task::spawn_blocking(move || {
            let mut cls_parser = Parser::new();
            if cls_parser
                .set_language(&LANGUAGE_OBJECTSCRIPT_UDL.into())
                .is_err()
            {
                eprintln!("Error: Failed to load ObjectScript UDL grammar");
                return;
            }

            let mut routine_parser = Parser::new();
            if routine_parser
                .set_language(&LANGUAGE_OBJECTSCRIPT_ROUTINE.into())
                .is_err()
            {
                eprintln!("Error: Failed to load ObjectScript routine grammar");
                return;
            }

            let mut documents_already_existing = Vec::new();
            for entry in WalkDir::new(&root).into_iter().filter_map(|e| e.ok()) {
                let path = entry.path();

                let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
                    continue;
                };

                let (filetype, is_rtn) = match ext {
                    "cls" => (FileType::Cls, false),
                    "inc" => (FileType::Routine, true),
                    "rtn" => (FileType::Routine, true),
                    "mac" => (FileType::Routine, true),
                    "int" => (FileType::Routine, true),
                    "xml" => (FileType::Xml, false),
                    _ => continue,
                };

                let content = match std::fs::read_to_string(path) {
                    Ok(s) => s,
                    Err(_) => {
                        eprintln!("Error: Failed to read file contents: {}", path.display());
                        continue;
                    }
                };

                let url = match Url::from_file_path(path) {
                    Ok(u) => u,
                    Err(_) => {
                        eprintln!("Error: Failed to convert path to Url: {}", path.display());
                        continue;
                    }
                };

                let tree = if is_rtn {
                    match routine_parser.parse(&content, None) {
                        Some(t) => t,
                        None => {
                            eprintln!("Failed to parse file for: {:?}", path.display());
                            continue;
                        }
                    }
                } else {
                    match cls_parser.parse(&content, None) {
                        Some(t) => t,
                        None => {
                            eprintln!("Failed to parse file for: {:?}", path.display());
                            continue;
                        }
                    }
                };

                let (class_range, class_name) = if filetype == FileType::Xml {
                    (tree.root_node().range(), "XML".to_string())
                } else {
                    if let Some((class_range, class_name)) =
                        get_member_name_and_range_from_root(&content, tree.root_node(), is_rtn)
                    {
                        (class_range, class_name)
                    } else {
                        eprintln!(
                            "Error: Failed to get name from root node for file url: {:?}",
                            url.path()
                        );
                        continue;
                    }
                };

                // Commit inside the ProjectData lock
                {
                    let mut data = project.data.write();
                    let already_exists = data.add_document_if_absent(
                        url.clone(),
                        content,
                        &tree,
                        filetype,
                        class_name,
                        class_range,
                        None,
                    );
                    if already_exists {
                        documents_already_existing.push(url);
                    }
                }
            }
        });
        // Wait for completion (and handle join errors)
        if let Err(join_err) = handle.await {
            eprintln!("Error: index_workspace_scope spawn_blocking failed: {join_err:?}");
        }
    }
}
