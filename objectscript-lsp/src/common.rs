use objectscript_core::common::{
    get_member_name_and_range_from_root, get_node_children, initial_build_scope_tree,
};
use objectscript_core::document::Document;
use objectscript_core::parse_structures::{FileType, IndexParsers};
use objectscript_core::scope_tree::ScopeTree;
use objectscript_core::workspace::BulkIndexDocument;
use std::path::{Path, PathBuf};
use tower_lsp::lsp_types::Url;
use tree_sitter::{Node, Range};
use walkdir::WalkDir;

pub enum PreparedFile {
    Ready(BulkIndexDocument, Range),
    Failed(IndexingIssue),
}

#[derive(Debug)]
pub enum IndexingIssue {
    Read {
        path: PathBuf,
        kind: std::io::ErrorKind,
    },
    InvalidUrl {
        path: PathBuf,
    },
    ParseFailed {
        path: PathBuf,
        file_type: FileType,
    },
    MissingDeclaration {
        path: PathBuf,
        url: Url,
    },
}

impl std::fmt::Display for IndexingIssue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read { path, kind } => {
                write!(formatter, "could not read {}: {kind:?}", path.display())
            }
            Self::InvalidUrl { path } => {
                write!(
                    formatter,
                    "could not convert {} to a file URL",
                    path.display()
                )
            }
            Self::ParseFailed { path, file_type } => {
                write!(
                    formatter,
                    "parser returned no tree for {file_type:?} {}",
                    path.display()
                )
            }
            Self::MissingDeclaration { path, url } => {
                write!(
                    formatter,
                    "could not find a class/routine declaration in {} ({url})",
                    path.display()
                )
            }
        }
    }
}

pub fn get_paths(root: &Path) -> Vec<(PathBuf, FileType)> {
    WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.into_path();
            if !path.is_file() {
                return None;
            }
            let file_type = match path.extension().and_then(|ext| ext.to_str()) {
                Some("cls") => FileType::Cls,
                Some("inc" | "rtn" | "mac" | "int") => FileType::Routine,
                Some("xml") => FileType::Xml,
                _ => return None,
            };
            Some((path, file_type))
        })
        .collect()
}

pub fn prepare_document(
    path: PathBuf,
    file_type: FileType,
    parsers: &mut IndexParsers,
) -> PreparedFile {
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) => {
            return PreparedFile::Failed(IndexingIssue::Read {
                path,
                kind: error.kind(),
            });
        }
    };
    let tree = match file_type {
        FileType::Routine => parsers.routine.parse(&content, None),
        FileType::Cls => parsers.cls.parse(&content, None),
        FileType::Xml => parsers.xml.parse(&content, None),
    };
    let Some(tree) = tree else {
        return PreparedFile::Failed(IndexingIssue::ParseFailed { path, file_type });
    };
    let url = match Url::from_file_path(&path) {
        Ok(url) => url,
        Err(_) => return PreparedFile::Failed(IndexingIssue::InvalidUrl { path }),
    };
    let is_routine = file_type == FileType::Routine;
    let declaration = if file_type == FileType::Xml {
        Some((
            tree.root_node().range(),
            "XML".to_string(),
            tree.root_node().range(),
        ))
    } else {
        get_member_name_and_range_from_root(&content, tree.root_node(), is_routine)
    };
    let Some((class_range, class_name, class_name_def_range)) = declaration else {
        return PreparedFile::Failed(IndexingIssue::MissingDeclaration { path, url });
    };
    let scope_tree = if file_type == FileType::Xml {
        ScopeTree::new()
    } else {
        initial_build_scope_tree(&tree, &content, is_routine)
    };
    let document = Document::new(content, tree, file_type, class_name, None, scope_tree, None);
    PreparedFile::Ready(
        BulkIndexDocument {
            url,
            class_range,
            document,
        },
        class_name_def_range,
    )
}

/// Generate a human-readable diagnostic message for a syntax error node.
pub fn diagnostic_message(node: Node, error_text: &str) -> Option<String> {
    if let Some(sibling_node) = node.prev_named_sibling() {
        match sibling_node.kind() {
            "statement" => {
                let child = sibling_node.named_child(0);
                if let Some(child) = child {
                    match child.kind() {
                        "command_set" => {
                            let children = get_node_children(child);
                            if let Some(last_child) = children.last() {
                                match last_child.kind() {
                                    "keyword_set" => {
                                        let Some(_) = node.parent() else {
                                            return Some(format!(
                                                "Syntax Error: Invalid set command {}",
                                                error_text
                                            ));
                                        };
                                        return Some(format!(
                                            "Syntax Error: Expected a variable name, got {}",
                                            error_text
                                        ));
                                    }
                                    "set_argument" => {
                                        let set_arg_children =
                                            get_node_children(last_child.clone());
                                        if let Some(child) = set_arg_children.last() {
                                            match child.kind() {
                                                "set_target" | "set_target_list" => {
                                                    if let Some(next_sib) = child.next_sibling() {
                                                        if next_sib.kind() == "=" {
                                                            return Some(format!(
                                                                "Syntax Error: Expected an expression, {} is not a valid expression.",
                                                                error_text
                                                            ));
                                                        }
                                                    };
                                                    return Some(format!(
                                                        "Syntax Error: Expected '=' or another variable name separated with by a comma and contained within parenthesis, got {}",
                                                        error_text
                                                    ));
                                                }
                                                "expression" => {
                                                    return Some(format!(
                                                        "Syntax Error: Unexpected, {} after an expression. Expected a binary operator or end of SET command",
                                                        error_text
                                                    ));
                                                }

                                                _ => return None,
                                            }
                                        }
                                        return None;
                                    }
                                    _ => {
                                        return None;
                                    }
                                }
                            }
                        }
                        _ => {
                            return None;
                        }
                    }
                }
            }
            _ => {
                return None;
            }
        }
    }
    None
}
