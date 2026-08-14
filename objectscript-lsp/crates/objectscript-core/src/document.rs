use crate::parse_structures::{ClassId, FileType};
use crate::scope_tree::*;
use tree_sitter::Tree;

/// Holds the current text (`content`), its parsed Tree-sitter syntax tree (`tree`),
/// derived semantic and scope state (scope tree + loca). `version` is `None` until the
/// document has been synced with the client.
#[derive(Clone, Debug)]
pub struct Document {
    /// Full file contents.
    pub content: String,
    /// Latest Tree-Sitter tree for this file.
    pub tree: Tree,
    /// LSP document version, `None` until document is opened.
    pub version: Option<i32>,
    /// Type of ObjectScript File: `.cls`, `.inc`, or `.mac`
    pub file_type: FileType,
    /// Keeps track of symbols (locations of class, private methods, variables) for the given file.
    pub scope_tree: ScopeTree,
    /// An ID that maps the the corresponding class for this file, if this is a `.cls` file.
    pub class_id: Option<ClassId>,
    /// Name of class for `.cls` files.
    pub class_name: String,
}

impl Document {
    /// Creates a new `Document` from parsed source state.
    ///
    /// Initializes the document text (`content`), syntax tree (`tree`), file metadata, and the
    /// initial `ScopeTree`. Semantic ids (`local_semantic_model_id`, `class_id`) are set to `None`
    /// and filled in during indexing/build steps.
    pub fn new(
        content: String,
        tree: Tree,
        file_type: FileType,
        class_name: String,
        class_id: Option<ClassId>,
        scope_tree: ScopeTree,
        version: Option<i32>,
    ) -> Self {
        Self {
            content,
            tree,
            version,
            file_type,
            scope_tree,
            class_id: class_id,
            class_name,
        }
    }
}
