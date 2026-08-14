use objectscript_core::common::get_member_name_and_range_from_root;
use objectscript_core::parse_structures::FileType;
use objectscript_core::workspace::ProjectState;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tower_lsp::lsp_types::Url;
use tree_sitter::Parser;
use tree_sitter_objectscript::LANGUAGE_OBJECTSCRIPT_UDL;
use tree_sitter_objectscript_routine::LANGUAGE_OBJECTSCRIPT_ROUTINE;
use walkdir::WalkDir;

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
    pub(crate) fn get_project_from_document_url(&self, uri: &Url) -> Option<Arc<ProjectState>> {
        let project_url = self.find_parent_workspace(uri.clone())?;
        self.get_project(&project_url)
    }

    /// Parse and index all ObjectScript files under the workspace containing `uri`.
    pub(crate) async fn index_workspace(&self, uri: &Url) {
        let Some(project) = self.get_project_from_document_url(&uri) else {
            return;
        };
        let Some(root) = project.root_path() else {
            eprintln!("Couldn't get root");
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
                eprintln!("Failed to load ObjectScript UDL grammar");
                return;
            }

            let mut routine_parser = Parser::new();
            if routine_parser
                .set_language(&LANGUAGE_OBJECTSCRIPT_ROUTINE.into())
                .is_err()
            {
                eprintln!("Failed to load ObjectScript routine grammar");
                return;
            }
            let mut documents_already_existing = Vec::new();
            for entry in WalkDir::new(&root).into_iter().filter_map(|e| e.ok()) {
                let path = entry.path();

                let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
                    continue;
                };

                let (filetype, use_routine) = match ext {
                    "cls" => (FileType::Cls, false),
                    "inc" => (FileType::Routine, true),
                    "rtn" => (FileType::Routine, true),
                    "mac" => (FileType::Routine, true),
                    "int" => (FileType::Routine, true),
                    _ => continue,
                };

                let code = match std::fs::read_to_string(path) {
                    Ok(s) => s,
                    Err(_) => continue,
                };

                let url = match Url::from_file_path(path) {
                    Ok(u) => u,
                    Err(_) => continue,
                };

                let tree = if use_routine {
                    match routine_parser.parse(&code, None) {
                        Some(t) => t,
                        None => {
                            eprintln!("Failed to parse file: {:?}", path);
                            continue;
                        }
                    }
                } else {
                    match cls_parser.parse(&code, None) {
                        Some(t) => t,
                        None => {
                            eprintln!("Failed to parse file: {:?}", path);
                            continue;
                        }
                    }
                };
                let is_rtn = if filetype == FileType::Routine {
                    true
                } else {
                    false
                };

                if let Some((member_range, member_name)) =
                    get_member_name_and_range_from_root(code.as_str(), tree.root_node(), is_rtn)
                {
                    let mut data = project.data.write();
                    let already_exists = data.add_document_if_absent(
                        url.clone(),
                        code,
                        &tree,
                        filetype,
                        member_name,
                        member_range,
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
            eprintln!("index_workspace_scope spawn_blocking failed: {join_err:?}");
        }
    }
}
