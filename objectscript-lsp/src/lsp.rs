use crate::common::diagnostic_message;
use crate::server::BackendWrapper;
use objectscript_core::common::{
    advance_point, generic_exit_statements, generic_skipping_statements, get_node_children,
    get_string_at_byte_range, parse_line_ref, point_to_byte, point_to_lsp_position,
    position_to_point, ts_range_to_lsp_range,
};
use objectscript_core::common::{
    collect_error_nodes, get_outer_type_from_identifier, xml_objectscript_implementation_ranges,
};
use objectscript_core::config::Config;
use objectscript_core::parse_structures::{ClassId, FileType, MemberType, RefactorLevel};
use objectscript_core::workspace::ProjectState;
use serde_json;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tower_lsp::LanguageServer;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::request::{GotoImplementationParams, GotoImplementationResponse};
use tower_lsp::lsp_types::{
    CodeAction, CodeActionKind, CodeActionOptions, CodeActionParams, CodeActionProviderCapability,
    CodeActionResponse, CodeActionTriggerKind, Command, Diagnostic, DiagnosticOptions,
    DiagnosticServerCapabilities, DiagnosticSeverity, DidChangeTextDocumentParams,
    DidChangeWatchedFilesParams, DidChangeWatchedFilesRegistrationOptions,
    DidOpenTextDocumentParams, DocumentDiagnosticParams, DocumentDiagnosticReport,
    DocumentDiagnosticReportResult, ExecuteCommandOptions, ExecuteCommandParams, FileChangeType,
    FileSystemWatcher, FullDocumentDiagnosticReport, GlobPattern, GotoDefinitionParams,
    GotoDefinitionResponse, ImplementationProviderCapability, InitializeParams, InitializeResult,
    InitializedParams, Location, MessageType, OneOf, Position, Range as LspRange, Range,
    Registration, RelatedFullDocumentDiagnosticReport, ServerCapabilities, ServerInfo,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit, WatchKind, WorkspaceEdit,
};
use tree_sitter::{InputEdit, Parser, Point, Tree};
use tree_sitter_objectscript_playground::LANGUAGE_OBJECTSCRIPT;

const REFACTOR_DOCUMENT_COMMAND: &str = "objectscript.refactorDocument";
const REFACTOR_WORKSPACE_COMMAND: &str = "objectscript.refactorWorkspace";
const LEGACY_REFACTOR_WORKSPACE_DO_COMMAND: &str = "objectscript.refactorWorkspaceDottedDo";
const USER_SELECTABLE_REFACTOR_LEVELS: [RefactorLevel; 4] = [
    RefactorLevel::DoCommands,
    RefactorLevel::Conditionals,
    RefactorLevel::ForCommands,
    RefactorLevel::All,
];

fn empty_diagnostic_report() -> DocumentDiagnosticReportResult {
    DocumentDiagnosticReport::Full(RelatedFullDocumentDiagnosticReport {
        related_documents: None,
        full_document_diagnostic_report: FullDocumentDiagnosticReport {
            result_id: None,
            items: vec![],
        },
    })
    .into()
}

fn file_type_from_path(path: &str) -> Option<FileType> {
    if path.ends_with(".cls") {
        Some(FileType::Cls)
    } else if path.ends_with(".inc")
        || path.ends_with(".rtn")
        || path.ends_with(".mac")
        || path.ends_with(".int")
    {
        Some(FileType::Routine)
    } else if path.ends_with(".xml") {
        Some(FileType::Xml)
    } else {
        None
    }
}

fn refactor_kind_requested(only: Option<&Vec<CodeActionKind>>) -> bool {
    only.map(|requested| {
        requested.iter().any(|kind| {
            CodeActionKind::REFACTOR_REWRITE
                .as_str()
                .starts_with(kind.as_str())
        })
    })
    .unwrap_or(true)
}

fn full_document_range(text: &str) -> Range {
    let line = text
        .as_bytes()
        .iter()
        .filter(|&&byte| byte == b'\n')
        .count() as u32;
    let last_line_start = text.rfind('\n').map(|idx| idx + 1).unwrap_or(0);
    let character = text[last_line_start..].encode_utf16().count() as u32;

    Range {
        start: Position::new(0, 0),
        end: Position::new(line, character),
    }
}

fn refactor_title(refactor_level: RefactorLevel, scope: &str) -> String {
    let target = match refactor_level {
        RefactorLevel::All => "All Code",
        RefactorLevel::DoCommands => "Legacy Do Commands",
        RefactorLevel::Conditionals => "Legacy If/Else Commands",
        RefactorLevel::ForCommands => "Legacy For Commands",
    };
    format!("Refactor {target} {scope}")
}

fn selectable_document_refactor_levels(file_type: FileType) -> &'static [RefactorLevel] {
    const ROUTINE_LEVELS: [RefactorLevel; 4] = [
        RefactorLevel::DoCommands,
        RefactorLevel::Conditionals,
        RefactorLevel::ForCommands,
        RefactorLevel::All,
    ];
    const CLASS_LEVELS: [RefactorLevel; 3] = [
        RefactorLevel::Conditionals,
        RefactorLevel::ForCommands,
        RefactorLevel::All,
    ];
    const XML_LEVELS: [RefactorLevel; 0] = [];

    match file_type {
        FileType::Routine => &ROUTINE_LEVELS,
        FileType::Cls => &CLASS_LEVELS,
        FileType::Xml => &XML_LEVELS,
    }
}

fn push_host_syntax_diagnostics(
    diagnostics: &mut Vec<Diagnostic>,
    content: &str,
    tree: &Tree,
    file_type: FileType,
) {
    let error_nodes = collect_error_nodes(tree.root_node());

    for node in error_nodes {
        let lsp_range = ts_range_to_lsp_range(content, node.range());
        let error_text =
            get_string_at_byte_range(content, node.byte_range()).unwrap_or_else(String::new);
        let message = if file_type == FileType::Xml {
            format!("XML syntax error: Unexpected {}", error_text)
        } else {
            diagnostic_message(node, error_text.as_str())
                .unwrap_or(format!("Syntax Error: Unexpected {}", error_text))
        };

        diagnostics.push(Diagnostic {
            range: lsp_range,
            severity: Some(DiagnosticSeverity::ERROR),
            code: None,
            code_description: None,
            source: None,
            message,
            related_information: None,
            tags: None,
            data: None,
        });
    }
}

fn push_xml_injected_objectscript_diagnostics(
    diagnostics: &mut Vec<Diagnostic>,
    content: &str,
    xml_tree: &Tree,
) {
    let ranges = xml_objectscript_implementation_ranges(xml_tree.root_node(), content);

    for range in ranges {
        let Some(text) = get_string_at_byte_range(content, range.start_byte..range.end_byte) else {
            continue;
        };
        if text.trim().is_empty() {
            continue;
        }

        let mut parser = Parser::new();
        if parser.set_language(&LANGUAGE_OBJECTSCRIPT.into()).is_err() {
            continue;
        }
        if parser.set_included_ranges(&[range]).is_err() {
            continue;
        }

        let Some(tree) = parser.parse(content, None) else {
            continue;
        };

        push_host_syntax_diagnostics(diagnostics, content, &tree, FileType::Routine);
    }
}

fn build_refactor_command(
    command: &str,
    uri: &tower_lsp::lsp_types::Url,
    refactor_level: RefactorLevel,
    scope: &str,
) -> Command {
    Command {
        title: refactor_title(refactor_level, scope),
        command: command.to_string(),
        arguments: Some(vec![
            Value::String(uri.to_string()),
            command_refactor_level_value(refactor_level),
        ]),
    }
}

fn build_document_refactor_edit(
    project: &ProjectState,
    uri: &tower_lsp::lsp_types::Url,
    refactor_level: RefactorLevel,
) -> Option<WorkspaceEdit> {
    let (_, content, _, _) = project.get_document_info(uri)?;
    let updated_content = project.refactor_document(uri, refactor_level)?;
    let mut changes = HashMap::new();
    changes.insert(
        uri.clone(),
        vec![TextEdit::new(
            full_document_range(content.as_str()),
            updated_content,
        )],
    );
    Some(WorkspaceEdit::new(changes))
}

fn collect_workspace_refactor_changes(
    project: &ProjectState,
    refactor_level: RefactorLevel,
) -> HashMap<tower_lsp::lsp_types::Url, Vec<TextEdit>> {
    let mut changes = HashMap::new();

    for (updated_content, url) in project.refactor(refactor_level) {
        let Some((_, content, _, _)) = project.get_document_info(&url) else {
            continue;
        };

        changes.insert(
            url.clone(),
            vec![TextEdit::new(
                full_document_range(content.as_str()),
                updated_content,
            )],
        );
    }

    changes
}

fn command_uri_argument(arguments: &[Value]) -> Option<tower_lsp::lsp_types::Url> {
    let first = arguments.first()?;
    let uri = serde_json::from_value::<String>(first.clone()).ok()?;
    tower_lsp::lsp_types::Url::parse(uri.as_str()).ok()
}

fn command_refactor_level_argument(command: &str, arguments: &[Value]) -> Option<RefactorLevel> {
    if command == LEGACY_REFACTOR_WORKSPACE_DO_COMMAND {
        return Some(RefactorLevel::DoCommands);
    }

    let level = arguments.get(1)?.as_str()?;
    match level {
        "all" => Some(RefactorLevel::All),
        "do" => Some(RefactorLevel::DoCommands),
        "conditionals" | "if" | "if/else" => Some(RefactorLevel::Conditionals),
        "for" => Some(RefactorLevel::ForCommands),
        _ => None,
    }
}

fn command_refactor_level_value(refactor_level: RefactorLevel) -> Value {
    let value = match refactor_level {
        RefactorLevel::All => "all",
        RefactorLevel::DoCommands => "do",
        RefactorLevel::Conditionals => "conditionals",
        RefactorLevel::ForCommands => "for",
    };
    Value::String(value.to_string())
}

fn build_caps(cfg: &Config) -> ServerCapabilities {
    ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(
            TextDocumentSyncKind::INCREMENTAL,
        )),
        definition_provider: Some(OneOf::Left(true)),
        implementation_provider: Some(ImplementationProviderCapability::Simple(true)),
        document_formatting_provider: cfg.enable_formatting.then_some(OneOf::Left(true)),
        code_action_provider: Some(CodeActionProviderCapability::Options(CodeActionOptions {
            code_action_kinds: Some(vec![CodeActionKind::REFACTOR_REWRITE]),
            resolve_provider: Some(false),
            work_done_progress_options: Default::default(),
        })),
        diagnostic_provider: Some(DiagnosticServerCapabilities::Options(DiagnosticOptions {
            identifier: None,
            inter_file_dependencies: false,
            workspace_diagnostics: false,
            work_done_progress_options: Default::default(),
        })),
        execute_command_provider: Some(ExecuteCommandOptions {
            commands: vec![
                REFACTOR_DOCUMENT_COMMAND.to_string(),
                REFACTOR_WORKSPACE_COMMAND.to_string(),
                LEGACY_REFACTOR_WORKSPACE_DO_COMMAND.to_string(),
            ],
            work_done_progress_options: Default::default(),
        }),
        experimental: Some(serde_json::json!({
            "objectscriptDependenciesProvider": true
        })),

        // TODO: need to do dotted statement formatting
        // document_formatting_provider: cfg.enable_formatting.then_some(OneOf::Left(true)),
        ..Default::default()
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for BackendWrapper {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        // negotiate w/ client to set config for formatting, lint, snippets
        let negotiations: Config = params
            .initialization_options
            .and_then(|v| serde_json::from_value::<Config>(v).ok())
            .unwrap_or_default();

        if let Some(folders) = params.workspace_folders {
            for folder in folders {
                let Ok(project_root) = folder.uri.to_file_path() else {
                    self.0
                        .client
                        .log_message(MessageType::ERROR, "Failed to get project root path")
                        .await;
                    continue;
                };
                // create projectState and set the projectRoot
                let state = ProjectState::new();
                if state.project_root_path.set(Some(project_root)).is_err() {
                    self.0
                        .client
                        .log_message(
                            MessageType::WARNING,
                            "project_root_path was already set; ignoring duplicate initialize",
                        )
                        .await;
                }

                // add projectState to projects
                self.0.add_project(folder.uri, state);
            }
        }
        Ok(InitializeResult {
            capabilities: build_caps(&negotiations),
            server_info: Some(ServerInfo {
                name: "objectscript-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        // register watchers for any .cls and .inc files in the workspace
        let globs = [
            "**/*.cls", "**/*.inc", "**/*.rtn", "**/*.mac", "**/*.int", "**/*.xml",
        ];
        let watchers = globs
            .into_iter()
            .map(|g| FileSystemWatcher {
                glob_pattern: GlobPattern::String(g.to_string()).into(),
                kind: Some(WatchKind::Create | WatchKind::Change | WatchKind::Delete),
            })
            .collect();
        let options = DidChangeWatchedFilesRegistrationOptions { watchers };

        let register_options = match serde_json::to_value(options) {
            Ok(v) => Some(v),
            Err(e) => {
                self.0
                    .client
                    .log_message(MessageType::ERROR, &e.to_string())
                    .await;
                None
            }
        };

        let registration = Registration {
            id: "ObjectScriptCacheWatcher".to_string(),
            method: "workspace/didChangeWatchedFiles".to_string(),
            register_options,
        };

        self.0
            .client
            .register_capability(vec![registration])
            .await
            .ok();

        if let Ok(Some(folders)) = self.0.client.workspace_folders().await {
            for workspace in folders {
                let backend = Arc::clone(&self.0);
                tokio::spawn(async move {
                    let _ = backend.index_workspace(&workspace.uri).await;
                });
            }
        }
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        if !refactor_kind_requested(params.context.only.as_ref()) {
            return Ok(None);
        }

        let uri = params.text_document.uri;
        let Some(project) = self.0.get_project_from_document_url(&uri) else {
            return Ok(None);
        };
        let Some((file_type, _, _, tree)) = project.get_document_info(&uri) else {
            return Ok(None);
        };
        let document_is_parseable = !tree.root_node().has_error();

        let mut actions = Vec::new();
        if document_is_parseable {
            for refactor_level in selectable_document_refactor_levels(file_type.clone()) {
                actions.push(
                    CodeAction {
                        title: refactor_title(*refactor_level, "in this document"),
                        kind: Some(CodeActionKind::REFACTOR_REWRITE),
                        diagnostics: None,
                        edit: None,
                        command: Some(build_refactor_command(
                            REFACTOR_DOCUMENT_COMMAND,
                            &uri,
                            *refactor_level,
                            "in this document",
                        )),
                        is_preferred: (*refactor_level == RefactorLevel::All).then_some(true),
                        disabled: None,
                        data: None,
                    }
                    .into(),
                );
            }
        }

        if file_type != FileType::Xml
            && params.context.trigger_kind != Some(CodeActionTriggerKind::AUTOMATIC)
        {
            for refactor_level in USER_SELECTABLE_REFACTOR_LEVELS {
                actions.push(
                    CodeAction {
                        title: refactor_title(refactor_level, "in workspace"),
                        kind: Some(CodeActionKind::REFACTOR_REWRITE),
                        diagnostics: None,
                        edit: None,
                        command: Some(build_refactor_command(
                            REFACTOR_WORKSPACE_COMMAND,
                            &uri,
                            refactor_level,
                            "in workspace",
                        )),
                        is_preferred: None,
                        disabled: None,
                        data: None,
                    }
                    .into(),
                );
            }
        }

        if actions.is_empty() {
            Ok(None)
        } else {
            Ok(Some(actions))
        }
    }

    async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        for change in params.changes {
            let Some(file_type) = file_type_from_path(change.uri.path()) else {
                continue;
            };
            if change.typ == FileChangeType::DELETED {
                continue;
            }

            let Some(project) = self.0.get_project_from_document_url(&change.uri) else {
                continue;
            };
            let Ok(path) = change.uri.to_file_path() else {
                continue;
            };
            let Ok(text) = std::fs::read_to_string(path) else {
                continue;
            };
            let version = project
                .get_document_info(&change.uri)
                .map(|(_, _, version, _)| version)
                .unwrap_or(0);

            project.handle_document_opened(change.uri, text, file_type, version);
        }
    }

    async fn execute_command(&self, params: ExecuteCommandParams) -> Result<Option<Value>> {
        if params.command != REFACTOR_DOCUMENT_COMMAND
            && params.command != REFACTOR_WORKSPACE_COMMAND
            && params.command != LEGACY_REFACTOR_WORKSPACE_DO_COMMAND
        {
            return Ok(None);
        }

        let Some(uri) = command_uri_argument(params.arguments.as_slice()) else {
            self.0
                .client
                .log_message(
                    MessageType::ERROR,
                    "Workspace refactor command is missing a document URI argument.",
                )
                .await;
            return Ok(None);
        };
        let Some(refactor_level) =
            command_refactor_level_argument(params.command.as_str(), params.arguments.as_slice())
        else {
            self.0
                .client
                .log_message(
                    MessageType::ERROR,
                    "Workspace refactor command is missing a valid refactor level argument.",
                )
                .await;
            return Ok(None);
        };
        let Some(project) = self.0.get_project_from_document_url(&uri) else {
            return Ok(None);
        };

        if params.command == REFACTOR_DOCUMENT_COMMAND {
            let Some(edit) = build_document_refactor_edit(project.as_ref(), &uri, refactor_level)
            else {
                return Ok(None);
            };

            let response = self.0.client.apply_edit(edit).await?;
            if !response.applied {
                let reason = response
                    .failure_reason
                    .unwrap_or_else(|| "Unknown failure".to_string());
                self.0
                    .client
                    .log_message(
                        MessageType::WARNING,
                        format!("Document refactor was not applied: {reason}",),
                    )
                    .await;
            }
        } else {
            let changes = collect_workspace_refactor_changes(project.as_ref(), refactor_level);
            if changes.is_empty() {
                return Ok(None);
            }

            let response = self
                .0
                .client
                .apply_edit(WorkspaceEdit::new(changes))
                .await?;
            if !response.applied {
                let reason = response
                    .failure_reason
                    .unwrap_or_else(|| "Unknown failure".to_string());
                self.0
                    .client
                    .log_message(
                        MessageType::WARNING,
                        format!("Workspace refactor was not applied: {reason}",),
                    )
                    .await;
            }
        }

        Ok(None)
    }

    async fn diagnostic(
        &self,
        params: DocumentDiagnosticParams,
    ) -> Result<DocumentDiagnosticReportResult> {
        let uri = params.text_document.uri;
        let mut diagnostics: Vec<Diagnostic> = Vec::new();
        let Some(project) = self.0.get_project_from_document_url(&uri) else {
            self.0
                .client
                .log_message(MessageType::ERROR, "Failed to get project from document")
                .await;
            generic_exit_statements("LSP", "diagnostic");
            return Ok(empty_diagnostic_report());
        };
        let doc_snapshot: Option<(FileType, String, Tree)> = {
            let data = project.data.read();
            data.documents
                .get(&uri)
                .map(|d| (d.file_type.clone(), d.content.clone(), d.tree.clone()))
        };

        let (file_type, content, tree) = match doc_snapshot {
            Some(v) => v,
            None => {
                self.0
                    .client
                    .log_message(MessageType::ERROR, "Failed to get document")
                    .await;
                return Ok(empty_diagnostic_report());
            }
        };
        let content = content.as_str();
        push_host_syntax_diagnostics(&mut diagnostics, content, &tree, file_type.clone());
        if file_type == FileType::Xml {
            let host_count = diagnostics.len();
            push_xml_injected_objectscript_diagnostics(&mut diagnostics, content, &tree);
            self.0
                .client
                .log_message(
                    MessageType::INFO,
                    format!(
                        "XML diagnostic for {} -> host errors: {}, total errors after injected ObjectScript pass: {}",
                        uri,
                        host_count,
                        diagnostics.len()
                    ),
                )
                .await;
        }
        Ok(
            DocumentDiagnosticReport::Full(RelatedFullDocumentDiagnosticReport {
                related_documents: None,
                full_document_diagnostic_report: FullDocumentDiagnosticReport {
                    result_id: None,
                    items: diagnostics,
                },
            })
            .into(),
        )
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let mut locations: Vec<Location> = Vec::new();
        let mut definitions = Vec::new();
        let Some(project) = self.0.get_project_from_document_url(&uri) else {
            self.0
                .client
                .log_message(
                    MessageType::ERROR,
                    "Error: Failed to get project from document, aborting goto_definition",
                )
                .await;
            return Ok(None);
        };
        let (content, tree, class_id, class_name): (String, Tree, ClassId, String) = {
            let data = project.data.read();
            let Some(document) = data.documents.get(&uri) else {
                eprintln!("Error: failed to get document for url {:?},", uri.path());
                return Ok(None);
            };
            let Some(class_id) = document.class_id.clone() else {
                return Ok(None);
            };

            let Some(class_name) = document.class_name.clone() else {
                return Ok(None);
            };
            (
                document.content.clone(),
                document.tree.clone(),
                class_id,
                class_name,
            )
        };
        let content = content.as_str();
        // find what node is at that position
        // convert position to point, and find smallest node that has the range of that Point
        let point = position_to_point(content, position);
        let Some(node) = tree
            .root_node()
            .named_descendant_for_point_range(point, point)
        else {
            eprintln!("Error: Failed to get node at point, exiting (LSP, goto_definition)",);
            return Ok(None);
        };
        if node.kind() == "identifier"
            || node.kind() == "objectscript_identifier"
            || node.kind() == "objectscript_identifier_special"
        {
            let Some(name) = node.parent() else {
                eprintln!("Warning: Identifier node does not have a parent");
                return Ok(None);
            };
            let Some(node_type) = get_outer_type_from_identifier(&name) else {
                return Ok(None);
            };
            let Some(name_string) = get_string_at_byte_range(content, node.byte_range()) else {
                self.0
                    .client
                    .log_message(
                        MessageType::ERROR,
                        "Error: failed to identifier string in goto_implementation.",
                    )
                    .await;
                return Ok(None);
            };
            match node_type {
                MemberType::ClassDef => {
                    definitions = {
                        let data = project.data.read();
                        data.get_class_superclasses(&class_id)
                    }
                }
                MemberType::MethodDef => {
                    definitions = {
                        let data = project.data.read();
                        data.get_method_superclass(name_string, &class_id)
                    }
                }
                MemberType::Class => {
                    definitions = {
                        let data = project.data.read();
                        data.get_class_definition(&name_string)
                    }
                }
                MemberType::RelativeMethodCall => {
                    definitions = {
                        let data = project.data.read();
                        let method_ref = if let Some(m_ref) = data
                            .method_defs
                            .get(&class_name)
                            .and_then(|methods| methods.get(&name_string))
                        {
                            m_ref
                        } else {
                            return Ok(None);
                        };
                        data.get_method_definition(method_ref, None)
                    }
                }
                MemberType::LocalVariable | MemberType::GlobalVariable => {
                    definitions = {
                        let data = project.data.read();
                        data.get_variable_definition(&uri, point, name_string)
                    }
                }
                MemberType::ClassMethodCall => {
                    let Some(method_name_parent) = name.parent() else {
                        eprintln!("Error: expected method_name node to have parent");
                        return Ok(None);
                    };
                    if let Some(class_ref) = method_name_parent.named_child(0)
                        && let Some(class_name_node_outer) = class_ref.named_child(1)
                    {
                        // this part will remove the strings and such (it grabs the actual $.identifier node)
                        if let Some(class_name_node) = class_name_node_outer.named_child(0) {
                            if let Some(class_name_str) =
                                get_string_at_byte_range(content, class_name_node.byte_range())
                            {
                                definitions = {
                                    let data = project.data.read();
                                    let method_ref = if let Some(m_ref) = data
                                        .method_defs
                                        .get(&class_name_str)
                                        .and_then(|methods| methods.get(&name_string))
                                    {
                                        m_ref
                                    } else {
                                        return Ok(None);
                                    };
                                    data.get_method_definition(method_ref, None)
                                };
                            }
                        }
                    }
                }
                MemberType::RoutineMethodCall => {
                    if name.kind() != "method_name" {
                        eprintln!(
                            "Error: Expected node with MemberType::RoutineMethodCall to be method_name, but got {:?}, aborting (goto_definition)",
                            name.kind()
                        );
                    }
                    if let Some(method_name_parent) = name.parent() {
                        match method_name_parent.kind() {
                            "routine_tag_call" | "print_argument" | "goto_argument" => {
                                definitions = {
                                    let data = project.data.read();
                                    let method_ref = if let Some(m_ref) = data
                                        .method_defs
                                        .get(&class_name)
                                        .and_then(|methods| methods.get(&name_string))
                                    {
                                        m_ref
                                    } else {
                                        return Ok(None);
                                    };
                                    data.get_method_definition(method_ref, None)
                                }
                            }
                            "extrinsic_function" | "line_ref" => {
                                definitions = {
                                    let data = project.data.read();
                                    let (routine_name, method_name, offset) = parse_line_ref(
                                        method_name_parent,
                                        content,
                                        class_name.clone(),
                                    );
                                    let method_ref = if let Some(m_ref) = data
                                        .method_defs
                                        .get(&routine_name)
                                        .and_then(|methods| methods.get(&method_name))
                                    {
                                        m_ref
                                    } else {
                                        return Ok(None);
                                    };
                                    data.get_method_definition(method_ref, offset)
                                }
                            }
                            _ => return Ok(None),
                        }
                    } else {
                        return Ok(None);
                    }
                }
                MemberType::OrefMethod => match name.kind() {
                    "method_name" => {
                        if let Some(oref_method) = name.parent()
                            && let Some(oref_method_parent) = oref_method.parent()
                        {
                            let oref_chain_expr = if oref_method_parent.kind()
                                == "oref_chain_segment"
                                && let Some(par) = oref_method_parent.parent()
                            {
                                par
                            } else if oref_method_parent.kind() == "do_parameter" {
                                oref_method_parent
                            } else {
                                eprintln!(
                                    "Error: Expected oref_chain_segment or do_parameter node, got {:?}, aborting goto_definition",
                                    oref_method_parent
                                );
                                return Ok(None);
                            };

                            if oref_chain_expr.named_child_count() > 2 {
                                eprintln!(
                                    "Error: Right now, analysis only supports 2 children for oref expression, aborting goto_definition"
                                );
                                return Ok(None);
                            }
                            // name_string = method_name
                            // variable_name =
                            if let Some(var_name_node) = oref_chain_expr.named_child(0) {
                                if var_name_node.kind() == "lvn"
                                    && let Some(var_name_str) = get_string_at_byte_range(
                                        content,
                                        var_name_node.byte_range(),
                                    )
                                {
                                    definitions = {
                                        let data = project.data.read();
                                        data.get_oref_definitions(
                                            &var_name_str,
                                            &name_string,
                                            &class_name,
                                            name.range(),
                                            true,
                                        )
                                    }
                                } else if var_name_node.kind() == "class_method_call" {
                                    if let Some(class_ref) = var_name_node.named_child(0)
                                        && let Some(method_name_node) = var_name_node.named_child(1)
                                        && let Some(class_name_node) = class_ref.named_child(1)
                                    {
                                        // this part will remove the strings and such (it grabs the actual $.identifier node)
                                        if let Some(method_name) = method_name_node.named_child(0)
                                            && let Some(curr_class_name) =
                                                class_name_node.named_child(0)
                                        {
                                            if let Some(method_name) = get_string_at_byte_range(
                                                content,
                                                method_name.byte_range(),
                                            ) {
                                                if method_name.eq_ignore_ascii_case("%new")
                                                    && let Some(curr_class) =
                                                        get_string_at_byte_range(
                                                            content,
                                                            curr_class_name.byte_range(),
                                                        )
                                                {
                                                    definitions = {
                                                        let data = project.data.read();
                                                        let method_ref = if let Some(m_ref) = data
                                                            .method_defs
                                                            .get(&curr_class)
                                                            .and_then(|methods| {
                                                                methods.get(&name_string)
                                                            }) {
                                                            m_ref
                                                        } else {
                                                            return Ok(None);
                                                        };
                                                        data.get_method_definition(method_ref, None)
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        } else {
                            return Ok(None);
                        }
                    }
                    "lvn" => {
                        if let Some(lvn_parent) = name.parent() {
                            match lvn_parent.kind() {
                                "class_ref" => {
                                    if let Some(class_ref_parent) = lvn_parent.parent() {
                                        match class_ref_parent.kind() {
                                            "class_method_call" => {
                                                if let Some(method_name_outer) =
                                                    class_ref_parent.named_child(1)
                                                    && let Some(method_name) =
                                                        method_name_outer.named_child(0)
                                                    && let Some(method_name_str) =
                                                        get_string_at_byte_range(
                                                            content,
                                                            method_name.byte_range(),
                                                        )
                                                {
                                                    definitions = {
                                                        let data = project.data.read();
                                                        data.get_oref_definitions(
                                                            &name_string,
                                                            &method_name_str,
                                                            &class_name,
                                                            name.range(),
                                                            false,
                                                        )
                                                    }
                                                }
                                            }
                                            "oref_chain_expr" => {
                                                if class_ref_parent.named_child_count() > 2 {
                                                    eprintln!(
                                                        "Error: Right now, analysis only supports 2 children for oref expression, aborting goto_definition"
                                                    );
                                                    return Ok(None);
                                                }
                                                if let Some(oref_chain_segment) =
                                                    class_ref_parent.named_child(1)
                                                    && let Some(oref_method) =
                                                        oref_chain_segment.named_child(0)
                                                    && let Some(method_name_node) =
                                                        oref_method.named_child(0)
                                                    && let Some(method_name_identifier) =
                                                        method_name_node.named_child(0)
                                                    && let Some(method_name_str) =
                                                        get_string_at_byte_range(
                                                            content,
                                                            method_name_identifier.byte_range(),
                                                        )
                                                {
                                                    definitions = {
                                                        let data = project.data.read();
                                                        data.get_oref_definitions(
                                                            &name_string,
                                                            &method_name_str,
                                                            &class_name,
                                                            name.range(),
                                                            false,
                                                        )
                                                    }
                                                }
                                            }
                                            _ => {
                                                eprintln!(
                                                    "Error: Unsupported node kind for class ref parent {:?}, aborting (goto_definition)",
                                                    class_ref_parent.kind()
                                                );
                                            }
                                        }
                                    }
                                }
                                "do_parameter" | "job_argument" => {
                                    // _method_call node
                                    if let Some(oref_method_node) = lvn_parent.named_child(1)
                                        && oref_method_node.kind() == "oref_method"
                                        && let Some(outer_method_name_node) =
                                            oref_method_node.named_child(0)
                                        && let Some(method_name_node) =
                                            outer_method_name_node.named_child(0)
                                        && let Some(method_name_str) = get_string_at_byte_range(
                                            content,
                                            method_name_node.byte_range(),
                                        )
                                    {
                                        definitions = {
                                            let data = project.data.read();
                                            data.get_oref_definitions(
                                                &name_string,
                                                &method_name_str,
                                                &class_name,
                                                name.range(),
                                                false,
                                            )
                                        }
                                    }
                                }
                                "oref_chain_expr" => {
                                    if lvn_parent.named_child_count() > 2 {
                                        eprintln!(
                                            "Error: Right now, analysis only supports 2 children for oref expression, aborting goto_definition"
                                        );
                                        return Ok(None);
                                    }
                                    if let Some(oref_chain_segment) = lvn_parent.named_child(1)
                                        && let Some(oref_method) = oref_chain_segment.named_child(0)
                                        && let Some(method_name_node) = oref_method.named_child(0)
                                        && let Some(method_name_identifier) =
                                            method_name_node.named_child(0)
                                        && let Some(method_name_str) = get_string_at_byte_range(
                                            content,
                                            method_name_identifier.byte_range(),
                                        )
                                    {
                                        definitions = {
                                            let data = project.data.read();
                                            data.get_oref_definitions(
                                                &name_string,
                                                &method_name_str,
                                                &class_name,
                                                name.range(),
                                                false,
                                            )
                                        }
                                    }
                                }
                                _ => {}
                            }
                        } else {
                            eprintln!(
                                "Error: Expected Lvn Node, got {:?}, and it did not have a parent node",
                                name.kind()
                            );
                            return Ok(None);
                        }
                    }
                    _ => {
                        eprintln!(
                            "Error: In MemberType::OrefMethod branch of goto_definition, unsupported node type: {:?}, returning None",
                            name.kind()
                        );
                    }
                },
                _ => {
                    definitions = {
                        let data = project.data.read();
                        data.get_variable_definition(&uri, point, name_string)
                    }
                }
            }
        } else if node.kind() == "routine_name" {
            let Some(routine_name) = get_string_at_byte_range(content, node.byte_range()) else {
                return Ok(None);
            };
            let Some(routine_ref) = node.parent() else {
                eprintln!("Error: routine_name node does not have a parent");
                return Ok(None);
            };
            if routine_ref.kind() != "routine_ref" {
                eprintln!("Error: expected routine_name node parent to be routine_ref");
                return Ok(None);
            }
            definitions = {
                let data = project.data.read();
                data.get_class_definition(&routine_name)
            }
        } else if node.kind() == "gvn" {
            let gvn_children = get_node_children(node);
            for gvn_child in gvn_children {
                if gvn_child.kind() == "identifier" {
                    if let Some(gvn_id) = get_string_at_byte_range(content, gvn_child.byte_range())
                    {
                        definitions = {
                            let data = project.data.read();
                            let point = position_to_point(content, position);
                            data.get_variable_definition(&uri, point, gvn_id)
                        }
                    }
                }
            }
        } else if node.kind() == "lvn" {
            // this is the case where a base regex is aliased as a lvn (so it shouldn't have any children)
            let Some(lvn_id_node) = node.named_child(0) else {
                return Ok(None);
            };
            if let Some(lvn_id) = get_string_at_byte_range(content, lvn_id_node.byte_range()) {
                definitions = {
                    let data = project.data.read();
                    let point = position_to_point(content, position);
                    data.get_variable_definition(&uri, point, lvn_id)
                }
            }
        } else if node.kind() == "numeric_literal" {
            if let Some(parent) = node.parent() {
                if parent.kind() == "routine_tag_call"
                    || parent.kind() == "print_argument"
                    || parent.kind() == "goto_argument"
                {
                    let Some(num_str) = get_string_at_byte_range(content, node.byte_range()) else {
                        return Ok(None);
                    };
                    match num_str.trim().parse::<usize>() {
                        Ok(n) => {
                            let new_point = Point {
                                row: point.row + n,
                                column: point.column,
                            };
                            let start = point_to_lsp_position(content, new_point);
                            let lsp_range = LspRange { start, end: start };
                            let location = Location {
                                uri: uri.clone(),
                                range: lsp_range,
                            };
                            return Ok(Some(GotoDefinitionResponse::Scalar(location)));
                        }
                        Err(_) => return Ok(None),
                    }
                }
            }
        }
        for (url, range) in definitions {
            let data = project.data.read();
            let Some(document) = data.documents.get(&url) else {
                eprintln!("Error: Couldn't get document content, skipping in goto_definition");
                continue;
            };
            let document_content = document.content.as_str();
            let lsp_range = ts_range_to_lsp_range(document_content, range);
            let location = Location {
                uri: url.clone(),
                range: lsp_range,
            };
            locations.push(location);
        }

        return if locations.is_empty() {
            eprintln!(
                "Error: Symbol {:?} is not defined in this workspace (goto_definition).",
                node.kind()
            );
            Ok(None)
        } else if locations.len() == 1 {
            Ok(Some(GotoDefinitionResponse::Scalar(locations[0].clone())))
        } else {
            Ok(Some(GotoDefinitionResponse::Array(locations)))
        };
    }

    async fn goto_implementation(
        &self,
        params: GotoImplementationParams,
    ) -> Result<Option<GotoImplementationResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let mut locations = Vec::new();
        let mut overrides = Vec::new();
        let Some(project) = self.0.get_project_from_document_url(&uri) else {
            self.0
                .client
                .log_message(
                    MessageType::ERROR,
                    "Error: Failed to get project from document aborting goto_implementation",
                )
                .await;

            return Ok(None);
        };
        let (content, tree, class_id): (String, Tree, ClassId) = {
            let data = project.data.read();
            let Some(document) = data.documents.get(&uri) else {
                eprintln!("Error: failed to get document for url {:?},", uri.path());
                return Ok(None);
            };
            let Some(class_id) = document.class_id else {
                eprintln!("Error: Class ID doesn't exist for url {:?}", uri.path());
                return Ok(None);
            };
            (document.content.clone(), document.tree.clone(), class_id)
        };
        let content = content.as_str();
        // find what node is at that position
        // convert position to point, and find smallest node that has the range of that Point
        let point = position_to_point(content, position);
        let Some(node) = tree
            .root_node()
            .named_descendant_for_point_range(point, point)
        else {
            self.0
                .client
                .log_message(
                    MessageType::ERROR,
                    format!(
                        "Error: Failed to get node at point, exiting (LSP, goto_implementation)",
                    ),
                )
                .await;
            return Ok(None);
        };
        if node.kind() == "identifier" || node.kind() == "objectscript_identifier" {
            let Some(name) = node.parent() else {
                eprintln!("Warning: Identifier node does not have a parent");
                return Ok(None);
            };
            let Some(node_type) = get_outer_type_from_identifier(&name) else {
                return Ok(None);
            };
            let Some(name_string) = get_string_at_byte_range(content, node.byte_range()) else {
                self.0
                    .client
                    .log_message(
                        MessageType::ERROR,
                        "Error: failed to identifier string in goto_implementation.",
                    )
                    .await;
                return Ok(None);
            };
            match node_type {
                MemberType::Class => {
                    let Some(class_name_parent) = name.parent() else {
                        eprintln!(
                            "Error: failed to get parent node for class name node, exiting goto_implementation"
                        );
                        return Ok(None);
                    };
                    match class_name_parent.kind() {
                        "class_definition" => {
                            overrides = {
                                let data = project.data.read();
                                data.get_class_implementations(&class_id)
                            };
                        }
                        _ => {
                            // TODO: Need to get class ID for the actual class this is referencing
                            let Some(class_name_str) =
                                get_string_at_byte_range(content, name.byte_range())
                            else {
                                eprintln!(
                                    "Error: Couldn't get class name string, exiting goto_implementation"
                                );
                                return Ok(None);
                            };
                            overrides = {
                                let data = project.data.read();
                                let Some(curr_class_id) = data.classes.get(&class_name_str) else {
                                    eprintln!(
                                        "Error: Class ID DNE for class {:?}, leaving goto_implementation",
                                        class_name_str
                                    );
                                    return Ok(None);
                                };
                                data.get_class_implementations(curr_class_id)
                            };
                        }
                    }
                }
                MemberType::ClassMethodCall => {
                    // In this case, the identifier (name_string) is the method name
                    let Some(class_method_call) = name.parent() else {
                        eprintln!("Error: expected method_name node to have parent");
                        return Ok(None);
                    };
                    if let Some(class_ref) = class_method_call.named_child(0)
                        && let Some(class_name_node) = class_ref.named_child(1)
                    {
                        // this part will remove the strings and such (it grabs the actual $.identifier node)
                        if let Some(class_name) = class_name_node.named_child(0)
                            && let Some(class_name_str) =
                                get_string_at_byte_range(content, class_name.byte_range())
                        {
                            overrides = {
                                let data = project.data.read();
                                if let Some(method_ref) = data
                                    .method_defs
                                    .get(&class_name_str)
                                    .and_then(|methods| methods.get(&name_string))
                                {
                                    data.get_method_overrides(method_ref)
                                } else {
                                    return Ok(None);
                                }
                            };
                        }
                    }
                }
                MemberType::MethodDef => {
                    overrides = {
                        let data = project.data.read();
                        if let Some(class) = data.global_semantic_model.get_class(&class_id)
                            && let Some(method_ref) = class.methods.get(&name_string)
                        {
                            data.get_method_overrides(&method_ref)
                        } else {
                            return Ok(None);
                        }
                    };
                }
                _ => return Ok(None),
            }
        }
        for (uri, range) in &overrides {
            let data = project.data.read();
            let Some(document_content) = data.documents.get(uri).map(|d| d.content.as_str()) else {
                eprintln!("Error: failed to get document of uri {}", uri.path());
                generic_skipping_statements(
                    "goto_implementation",
                    uri.path(),
                    "Url, failed to get document",
                );
                continue;
            };
            let lsp_range = ts_range_to_lsp_range(document_content, *range);
            let location = Location {
                uri: uri.clone(),
                range: lsp_range,
            };
            locations.push(location);
        }
        if locations.len() == 1 {
            Ok(Some(GotoImplementationResponse::Scalar(
                locations[0].clone(),
            )))
        } else if locations.is_empty() {
            self.0
                .client
                .log_message(
                    MessageType::WARNING,
                    "No implementations were found for the given symbol.",
                )
                .await;
            Ok(None)
        } else {
            Ok(Some(GotoImplementationResponse::Array(locations)))
        }
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let path = uri.path();
        if !path.ends_with(".cls")
            && !path.ends_with(".inc")
            && !path.ends_with(".rtn")
            && !path.ends_with(".mac")
            && !path.ends_with(".int")
            && !path.ends_with(".xml")
        {
            return;
        }
        let file_type = if path.ends_with(".cls") {
            FileType::Cls
        } else if path.ends_with(".xml") {
            FileType::Xml
        } else {
            FileType::Routine
        };
        if file_type == FileType::Xml {
            self.0
                .client
                .log_message(
                    MessageType::INFO,
                    format!("Tracking XML document in objectscript-lsp: {}", uri),
                )
                .await;
        }
        self.0.handle_did_open(
            uri,
            params.text_document.text,
            file_type,
            params.text_document.version,
        );
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        let path = uri.path();
        if !path.ends_with(".cls")
            && !path.ends_with(".inc")
            && !path.ends_with(".rtn")
            && !path.ends_with(".mac")
            && !path.ends_with(".int")
            && !path.ends_with(".xml")
        {
            return;
        }
        let Some(project) = self.0.get_project_from_document_url(&uri) else {
            return;
        };
        let Some((file_type, mut old_text, old_version, mut old_tree)) =
            project.get_document_info(&uri)
        else {
            let new_version = params.text_document.version;
            let file_type = if path.ends_with(".cls") {
                FileType::Cls
            } else if path.ends_with(".xml") {
                FileType::Xml
            } else {
                FileType::Routine
            };
            // Try to get current cached doc

            // Base text: prefer disk if available, otherwise empty.
            let mut text = if let Ok(p) = uri.to_file_path() {
                std::fs::read_to_string(p).unwrap_or_default()
            } else {
                String::new()
            };

            for change in &params.content_changes {
                let Some(range) = change.range else {
                    text = change.text.clone();
                    continue;
                };

                let start_point = position_to_point(&text, range.start);
                let start_byte = point_to_byte(&text, start_point);

                let end_point = position_to_point(&text, range.end);
                let end_byte = point_to_byte(&text, end_point);

                text.replace_range(start_byte..end_byte, &change.text);
            }

            // Reuse normal open handling so XML docs are tracked and ObjectScript docs still
            // populate their semantic state when a change arrives before an explicit didOpen.
            project.handle_document_opened(uri, text, file_type, new_version);
            return;
        };

        let new_version = params.text_document.version;
        if new_version < old_version {
            self.0
                .client
                .log_message(
                    MessageType::ERROR,
                    "New version {new_version} is less than old version {old_version}",
                )
                .await;
        }

        let full_snapshot = params
            .content_changes
            .iter()
            .rev()
            .find(|c| c.range.is_none())
            .map(|c| c.text.clone());

        let did_full_replace = full_snapshot.is_some();
        self.0
            .client
            .log_message(
                MessageType::INFO,
                format!("Full Replace: {:?}", did_full_replace),
            )
            .await;
        if let Some(new_full_text) = full_snapshot {
            // Full replace: overwrite text, DO NOT edit the old tree incrementally.
            old_text = new_full_text;
        } else {
            // Incremental edits: apply each ranged edit sequentially.
            for change in &params.content_changes {
                let range = change
                    .range
                    .expect("no full snapshot, so all changes must have ranges");
                let new_text = change.text.as_str();

                let start_position = position_to_point(old_text.as_str(), range.start);
                let start_byte = point_to_byte(old_text.as_str(), start_position);

                let old_end_position = position_to_point(old_text.as_str(), range.end);
                let old_end_byte = point_to_byte(old_text.as_str(), old_end_position);

                let new_end_byte = start_byte + new_text.len();
                let new_end_position =
                    advance_point(start_position.row, start_position.column, new_text);

                let input_edit = InputEdit {
                    start_byte,
                    old_end_byte,
                    new_end_byte,
                    start_position,
                    old_end_position,
                    new_end_position,
                };
                old_text.replace_range(start_byte..old_end_byte, new_text);
                old_tree.edit(&input_edit);
            }
        }

        let parsed: Option<Tree> = match file_type {
            FileType::Cls => {
                let mut parser = project.parsers.cls.lock();
                if did_full_replace {
                    parser.parse(&old_text, None)
                } else {
                    parser.parse(&old_text, Some(&old_tree))
                }
            }
            FileType::Routine => {
                let mut parser = project.parsers.routine.lock();
                if did_full_replace {
                    parser.parse(&old_text, None)
                } else {
                    parser.parse(&old_text, Some(&old_tree))
                }
            }
            FileType::Xml => {
                let mut parser = project.parsers.xml.lock();
                parser.parse(&old_text, None)
            }
        }; // lock guard drops here

        let new_tree = match parsed {
            Some(t) => t,
            None => {
                self.0
                    .client
                    .log_message(
                        MessageType::WARNING,
                        "Incremental parse failed.".to_string(),
                    )
                    .await;
                return;
            }
        };

        {
            let mut data = project.data.write();
            if let Some(doc) = data.documents.get_mut(&uri) {
                doc.content = old_text.clone();
                doc.tree = new_tree.clone();
                doc.version = Some(new_version);
                doc.file_type = file_type.clone();
            }
        }

        if file_type == FileType::Xml {
            return;
        }

        if new_tree.root_node().has_error() {
            self.0
                .client
                .log_message(MessageType::ERROR, format!("New Tree has Errors"))
                .await;
        } else {
            project.update_document(uri, new_tree, file_type, new_version, old_text.as_str());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend_testing::BackendTester;
    use objectscript_core::workspace::ProjectState;
    use std::env;
    use std::path::PathBuf;
    use tower_lsp::LspService;
    use tower_lsp::lsp_types::{
        DocumentDiagnosticReport, DocumentDiagnosticReportResult, PartialResultParams,
        TextDocumentIdentifier, Url, WorkDoneProgressParams,
    };

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
        let uri = Url::from_file_path(project_root).unwrap();
        backend.add_project(uri.clone(), state);
        let _ = backend.index_workspace(&uri).await;
        (backend, uri)
    }

    #[test]
    fn build_document_refactor_edit_returns_none_when_no_change() {
        let state = ProjectState::new();
        let uri = Url::parse("file:///tmp/example.mac").unwrap();
        let content = "ROUTINE test\n\nmain\n quit\n";
        state.handle_document_opened(uri.clone(), content.to_string(), FileType::Routine, 1);
        assert!(build_document_refactor_edit(&state, &uri, RefactorLevel::DoCommands).is_none());
    }

    #[test]
    fn build_refactor_command_uses_conditionals_label_and_argument() {
        let uri = Url::parse("file:///tmp/example.mac").unwrap();
        let command = build_refactor_command(
            REFACTOR_DOCUMENT_COMMAND,
            &uri,
            RefactorLevel::Conditionals,
            "in this document",
        );

        assert_eq!(command.command, REFACTOR_DOCUMENT_COMMAND);
        assert_eq!(
            command.title,
            "Refactor Legacy If/Else Commands in this document"
        );
        assert_eq!(
            command.arguments,
            Some(vec![
                Value::String(uri.to_string()),
                Value::String("conditionals".to_string()),
            ])
        );
    }

    #[test]
    fn command_refactor_level_argument_supports_new_and_legacy_commands() {
        let uri = Value::String("file:///tmp/example.mac".to_string());

        assert_eq!(
            command_refactor_level_argument(
                REFACTOR_WORKSPACE_COMMAND,
                &[uri.clone(), Value::String("all".to_string())]
            ),
            Some(RefactorLevel::All)
        );
        assert_eq!(
            command_refactor_level_argument(
                REFACTOR_WORKSPACE_COMMAND,
                &[uri.clone(), Value::String("conditionals".to_string())]
            ),
            Some(RefactorLevel::Conditionals)
        );
        assert_eq!(
            command_refactor_level_argument(
                REFACTOR_WORKSPACE_COMMAND,
                &[uri.clone(), Value::String("if/else".to_string())]
            ),
            Some(RefactorLevel::Conditionals)
        );
        assert_eq!(
            command_refactor_level_argument(
                REFACTOR_WORKSPACE_COMMAND,
                &[uri.clone(), Value::String("for".to_string())]
            ),
            Some(RefactorLevel::ForCommands)
        );
        assert_eq!(
            command_refactor_level_argument(LEGACY_REFACTOR_WORKSPACE_DO_COMMAND, &[uri]),
            Some(RefactorLevel::DoCommands)
        );
    }

    #[test]
    fn build_refactor_command_keeps_do_refactor_selectable_for_class_documents() {
        let uri = Url::parse("file:///tmp/example.cls").unwrap();
        let command = build_refactor_command(
            REFACTOR_DOCUMENT_COMMAND,
            &uri,
            RefactorLevel::DoCommands,
            "in this document",
        );

        assert_eq!(command.command, REFACTOR_DOCUMENT_COMMAND);
        assert_eq!(
            command.title,
            "Refactor Legacy Do Commands in this document"
        );
        assert_eq!(
            command.arguments,
            Some(vec![
                Value::String(uri.to_string()),
                Value::String("do".to_string()),
            ])
        );
    }

    #[test]
    fn file_type_from_path_detects_xml_documents() {
        assert_eq!(file_type_from_path("/tmp/test.xml"), Some(FileType::Xml));
    }

    #[test]
    fn handle_document_opened_tracks_xml_without_semantic_analysis() {
        let state = ProjectState::new();
        let uri = Url::parse("file:///tmp/example.xml").unwrap();
        let content = r#"<Class>
<Method name="Test">
<Implementation><![CDATA[
set x = 1
if  {
]]></Implementation>
</Method>
</Class>"#;

        state.handle_document_opened(uri.clone(), content.to_string(), FileType::Xml, 1);

        let data = state.data.read();
        let document = data.documents.get(&uri).expect("missing xml document");

        assert_eq!(document.file_type, FileType::Xml);
        assert!(document.class_id.is_none());
        assert!(document.class_name.is_none());
    }

    #[test]
    fn xml_implementation_blocks_produce_objectscript_diagnostics() {
        let state = ProjectState::new();
        let uri = Url::parse("file:///tmp/example.xml").unwrap();
        let content = r#"<Class>
<Method name="Test">
<Implementation><![CDATA[
set = 1
]]></Implementation>
</Method>
</Class>"#;

        state.handle_document_opened(uri.clone(), content.to_string(), FileType::Xml, 1);
        let (_, stored_content, _, tree) = state
            .get_document_info(&uri)
            .expect("missing stored xml document");

        let mut diagnostics = Vec::new();
        push_xml_injected_objectscript_diagnostics(
            &mut diagnostics,
            stored_content.as_str(),
            &tree,
        );

        assert!(!diagnostics.is_empty());
    }

    #[test]
    fn xml_implementation_blocks_with_fake_cdata_markers_produce_diagnostics() {
        let state = ProjectState::new();
        let uri = Url::parse("file:///tmp/export.xml").unwrap();
        let content = r#"<?xml version="1.0" encoding="UTF-8"?>
<Export generator="IRIS" version="26">
<Class name="%SYS.Python">
<Method name="testing">
<Implementation> ![CDATA [
 s x = 2
 w !, x "2"
 if x = 2 {
    w !, "hello"
    q
 }
 else {
    w "goodbye"
    q
 }
]]
</Implementation>
</Method>
</Class>
</Export>"#;

        state.handle_document_opened(uri.clone(), content.to_string(), FileType::Xml, 1);
        let (_, stored_content, _, tree) = state
            .get_document_info(&uri)
            .expect("missing stored xml document");

        let ranges =
            super::xml_objectscript_implementation_ranges(tree.root_node(), &stored_content);
        assert!(!ranges.is_empty(), "expected injection ranges");

        let mut diagnostics = Vec::new();
        push_xml_injected_objectscript_diagnostics(
            &mut diagnostics,
            stored_content.as_str(),
            &tree,
        );

        assert!(
            !diagnostics.is_empty(),
            "expected injected diagnostics, ranges were: {:?}",
            ranges
        );
    }

    #[tokio::test]
    async fn diagnostic_request_returns_injected_objectscript_errors_for_xml_documents() {
        let (service, _socket) = LspService::build(|client| BackendWrapper::new(client)).finish();
        let backend = service.inner();

        let workspace_root = env::current_dir().unwrap();
        let workspace_uri = Url::from_file_path(&workspace_root).unwrap();
        let state = ProjectState::new();
        state
            .project_root_path
            .set(Some(workspace_root.clone()))
            .expect("failed to set workspace root");
        backend.0.add_project(workspace_uri.clone(), state);

        let uri = Url::from_file_path(workspace_root.join("objectscript-tests").join("export.xml"))
            .unwrap();
        let content = r#"<?xml version="1.0" encoding="UTF-8"?>
<Export generator="IRIS" version="26">
<Class name="%SYS.Python">
<Method name="testing">
<Implementation> ![CDATA [
 s x = 2
 w !, x "2"
 if x = 2 {
    w !, "hello"
    q
 }
 else {
    w "goodbye"
    q
 }
]]
</Implementation>
</Method>
</Class>
</Export>"#;

        let project = backend
            .0
            .get_project(&workspace_uri)
            .expect("missing project state");
        project.handle_document_opened(uri.clone(), content.to_string(), FileType::Xml, 1);

        let report = backend
            .diagnostic(DocumentDiagnosticParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                identifier: None,
                previous_result_id: None,
                work_done_progress_params: WorkDoneProgressParams::default(),
                partial_result_params: PartialResultParams::default(),
            })
            .await
            .expect("diagnostic request failed");

        let items = match report {
            DocumentDiagnosticReportResult::Report(DocumentDiagnosticReport::Full(report)) => {
                report.full_document_diagnostic_report.items
            }
            other => panic!("unexpected diagnostic report: {other:?}"),
        };

        assert!(
            !items.is_empty(),
            "expected diagnostic() to return injected ObjectScript errors for XML"
        );
    }

    #[tokio::test]
    async fn collect_workspace_refactor_changes_finds_routine_edits() {
        let project_root = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("routines");
        let test_mac_url = Url::from_file_path(project_root.join("test-refactor-do.mac")).unwrap();

        let (backend, uri) = setup_backend_and_workspace(project_root).await;
        let project = backend.get_project(&uri).expect("missing project state");
        let changes =
            collect_workspace_refactor_changes(project.as_ref(), RefactorLevel::DoCommands);

        assert!(changes.contains_key(&test_mac_url));
    }

    #[tokio::test]
    async fn collect_workspace_refactor_changes_finds_conditional_edits() {
        let project_root = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("routines");
        let test_mac_url = Url::from_file_path(project_root.join("test-refactor-do.mac")).unwrap();

        let (backend, uri) = setup_backend_and_workspace(project_root).await;
        let project = backend.get_project(&uri).expect("missing project state");
        let changes =
            collect_workspace_refactor_changes(project.as_ref(), RefactorLevel::Conditionals);

        assert!(changes.contains_key(&test_mac_url));
    }
}
