use objectscript_core::common::{point_to_byte, position_to_point};
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};
use std::env;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tower_lsp::lsp_types::{Position, Url};
use walkdir::WalkDir;

const MCP_PROTOCOL_VERSION: &str = "2024-11-05";
const SERVER_NAME: &str = "objectscript-lsp-mcp";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const REFACTOR_DOCUMENT_COMMAND: &str = "objectscript.refactorDocument";
const REFACTOR_WORKSPACE_COMMAND: &str = "objectscript.refactorWorkspace";
const LEGACY_REFACTOR_WORKSPACE_DO_COMMAND: &str = "objectscript.refactorWorkspaceDottedDo";

fn main() {
    let mut server = McpServer::new();
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(err) => {
                eprintln!("failed to read MCP stdin: {err}");
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }

        let request: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(err) => {
                eprintln!("failed to parse MCP message: {err}");
                continue;
            }
        };

        if let Some(response) = server.handle_message(request) {
            if let Err(err) = write_mcp_message(&mut stdout, &response) {
                eprintln!("failed to write MCP response: {err}");
                break;
            }
        }
    }
}

struct McpServer {
    root: PathBuf,
    lsp: Option<LspSession>,
}

impl McpServer {
    fn new() -> Self {
        Self {
            root: default_workspace_root(),
            lsp: None,
        }
    }

    fn handle_message(&mut self, request: Value) -> Option<Value> {
        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        let id = request.get("id").cloned();

        match method {
            "initialize" => id.map(|id| {
                json_rpc_result(
                    id,
                    json!({
                        "protocolVersion": MCP_PROTOCOL_VERSION,
                        "capabilities": {
                            "tools": {}
                        },
                        "serverInfo": {
                            "name": SERVER_NAME,
                            "version": SERVER_VERSION
                        }
                    }),
                )
            }),
            "notifications/initialized" => {
                self.eager_initialize_workspace();
                None
            }
            "$/cancelRequest" => None,
            "ping" => id.map(|id| json_rpc_result(id, json!({}))),
            "tools/list" => id.map(|id| json_rpc_result(id, tools_list())),
            "resources/list" => id.map(|id| json_rpc_result(id, json!({ "resources": [] }))),
            "prompts/list" => id.map(|id| json_rpc_result(id, json!({ "prompts": [] }))),
            "tools/call" => id.map(|id| {
                let result = self.handle_tool_call(request.get("params").unwrap_or(&Value::Null));
                json_rpc_result(id, result)
            }),
            _ => id.map(|id| {
                json_rpc_error(
                    id,
                    -32601,
                    format!("unknown MCP method: {method}"),
                    Value::Null,
                )
            }),
        }
    }

    fn handle_tool_call(&mut self, params: &Value) -> Value {
        let name = params.get("name").and_then(Value::as_str).unwrap_or("");
        let arguments = params.get("arguments").unwrap_or(&Value::Null);

        match name {
            "objectscript_initialize_workspace" => {
                match self.tool_initialize_workspace(arguments) {
                    Ok(result) => tool_json(result),
                    Err(err) => tool_error(err),
                }
            }
            "objectscript_diagnostics" => match self.tool_diagnostics(arguments) {
                Ok(result) => tool_json(result),
                Err(err) => tool_error(err),
            },
            "objectscript_goto_definition" => match self.tool_goto_definition(arguments) {
                Ok(result) => tool_json(result),
                Err(err) => tool_error(err),
            },
            "objectscript_code_actions" => match self.tool_code_actions(arguments) {
                Ok(result) => tool_json(result),
                Err(err) => tool_error(err),
            },
            "objectscript_execute_command" => match self.tool_execute_command(arguments) {
                Ok(result) => tool_json(result),
                Err(err) => tool_error(err),
            },
            "objectscript_workspace_diagnostics" => {
                match self.tool_workspace_diagnostics(arguments) {
                    Ok(result) => tool_json(result),
                    Err(err) => tool_error(err),
                }
            }
            "objectscript_lsp_status" => tool_json(self.tool_status()),
            _ => tool_error(format!("unknown tool: {name}")),
        }
    }

    fn tool_initialize_workspace(&mut self, arguments: &Value) -> Result<Value, String> {
        let root = optional_string(arguments, "root")
            .map(PathBuf::from)
            .unwrap_or_else(default_workspace_root);
        let root = absolute_path(root)?;

        self.ensure_lsp_for_root(root.clone())?;

        Ok(json!({
            "workspaceRoot": root.display().to_string(),
            "workspaceUri": file_uri(&root)?.to_string(),
            "lspBinary": lsp_binary_display(),
            "initialized": true
        }))
    }

    fn tool_diagnostics(&mut self, arguments: &Value) -> Result<Value, String> {
        let Some(file_path) = optional_string(arguments, "file_path") else {
            return Err("missing required argument: file_path".to_string());
        };
        let root = optional_string(arguments, "root")
            .map(PathBuf::from)
            .unwrap_or_else(|| self.root.clone());
        let root = absolute_path(root)?;
        let path = resolve_workspace_path(&root, file_path)?;

        self.ensure_lsp_for_root(root.clone())?;
        let lsp = self
            .lsp
            .as_mut()
            .ok_or_else(|| "LSP session was not initialized".to_string())?;

        let response = lsp.diagnostics(&path)?;
        Ok(json!({
            "workspaceRoot": root.display().to_string(),
            "filePath": path.display().to_string(),
            "uri": file_uri(&path)?.to_string(),
            "diagnosticReport": response
        }))
    }

    fn tool_goto_definition(&mut self, arguments: &Value) -> Result<Value, String> {
        let Some(file_path) = optional_string(arguments, "file_path") else {
            return Err("missing required argument: file_path".to_string());
        };
        let line = required_one_based_u32(arguments, "line")?;
        let character = required_one_based_u32(arguments, "character")?;
        let root = optional_string(arguments, "root")
            .map(PathBuf::from)
            .unwrap_or_else(|| self.root.clone());
        let root = absolute_path(root)?;
        let path = resolve_workspace_path(&root, file_path)?;

        self.ensure_lsp_for_root(root.clone())?;
        let lsp = self
            .lsp
            .as_mut()
            .ok_or_else(|| "LSP session was not initialized".to_string())?;

        let response = lsp.goto_definition(&path, line - 1, character - 1)?;
        Ok(json!({
            "workspaceRoot": root.display().to_string(),
            "filePath": path.display().to_string(),
            "uri": file_uri(&path)?.to_string(),
            "position": {
                "line": line,
                "character": character
            },
            "definitionResult": response
        }))
    }

    fn tool_code_actions(&mut self, arguments: &Value) -> Result<Value, String> {
        let Some(file_path) = optional_string(arguments, "file_path") else {
            return Err("missing required argument: file_path".to_string());
        };
        let root = optional_string(arguments, "root")
            .map(PathBuf::from)
            .unwrap_or_else(|| self.root.clone());
        let root = absolute_path(root)?;
        let path = resolve_workspace_path(&root, file_path)?;
        let range = code_action_range(arguments)?;
        let only = optional_string_array(arguments, "only")?
            .unwrap_or_else(|| vec!["refactor.rewrite".to_string()]);
        let trigger_kind = optional_code_action_trigger_kind(arguments)?;

        self.ensure_lsp_for_root(root.clone())?;
        let lsp = self
            .lsp
            .as_mut()
            .ok_or_else(|| "LSP session was not initialized".to_string())?;

        let response = lsp.code_actions(&path, range, only, trigger_kind)?;
        Ok(json!({
            "workspaceRoot": root.display().to_string(),
            "filePath": path.display().to_string(),
            "uri": file_uri(&path)?.to_string(),
            "codeActions": response
        }))
    }

    fn tool_execute_command(&mut self, arguments: &Value) -> Result<Value, String> {
        let Some(command) = optional_string(arguments, "command") else {
            return Err("missing required argument: command".to_string());
        };
        if !is_allowed_lsp_command(command) {
            return Err(format!("unsupported ObjectScript LSP command: {command}"));
        }
        let command_arguments = optional_array(arguments, "arguments")?.unwrap_or_default();
        let root = optional_string(arguments, "root")
            .map(PathBuf::from)
            .unwrap_or_else(|| self.root.clone());
        let root = absolute_path(root)?;

        self.ensure_lsp_for_root(root.clone())?;
        let lsp = self
            .lsp
            .as_mut()
            .ok_or_else(|| "LSP session was not initialized".to_string())?;

        for path in command_document_paths(&root, &command_arguments)? {
            if is_supported_file(&path) {
                lsp.open_document(&path)?;
            }
        }

        let response = lsp.execute_command(command, command_arguments)?;
        Ok(json!({
            "workspaceRoot": root.display().to_string(),
            "command": command,
            "executeCommandResult": response.result,
            "appliedEditPaths": response.applied_edit_paths,
            "appliedEditCount": response.applied_edit_count
        }))
    }

    fn tool_workspace_diagnostics(&mut self, arguments: &Value) -> Result<Value, String> {
        let root = optional_string(arguments, "root")
            .map(PathBuf::from)
            .unwrap_or_else(|| self.root.clone());
        let root = absolute_path(root)?;
        let include_clean = optional_bool(arguments, "include_clean");
        let max_files = optional_usize(arguments, "max_files")?;

        self.ensure_lsp_for_root(root.clone())?;
        let lsp = self
            .lsp
            .as_mut()
            .ok_or_else(|| "LSP session was not initialized".to_string())?;

        let mut files_checked = 0usize;
        let mut files_with_diagnostics = 0usize;
        let mut diagnostics_count = 0usize;
        let mut files = Vec::new();
        let mut clean_files = Vec::new();
        let mut errors = Vec::new();
        let mut truncated = false;

        for entry in WalkDir::new(&root).into_iter() {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err) => {
                    errors.push(json!({
                        "path": err.path().map(|path| path.display().to_string()),
                        "error": err.to_string()
                    }));
                    continue;
                }
            };

            let path = entry.path();
            if !path.is_file() || !is_supported_file(path) {
                continue;
            }

            if let Some(max_files) = max_files {
                if files_checked >= max_files {
                    truncated = true;
                    break;
                }
            }

            files_checked += 1;
            let label = workspace_relative_path(&root, path);
            match lsp.diagnostics(path) {
                Ok(report) => {
                    let items = diagnostic_items(&report);
                    let count = items.len();
                    diagnostics_count += count;
                    if count > 0 {
                        files_with_diagnostics += 1;
                        files.push(json!({
                            "path": label,
                            "uri": file_uri(path)?.to_string(),
                            "diagnosticsCount": count,
                            "diagnosticReport": report
                        }));
                    } else if include_clean {
                        clean_files.push(label);
                    }
                }
                Err(err) => {
                    errors.push(json!({
                        "path": label,
                        "error": err
                    }));
                }
            }
        }

        Ok(json!({
            "workspaceRoot": root.display().to_string(),
            "workspaceUri": file_uri(&root)?.to_string(),
            "filesChecked": files_checked,
            "filesWithDiagnostics": files_with_diagnostics,
            "diagnosticsCount": diagnostics_count,
            "truncated": truncated,
            "files": files,
            "cleanFiles": if include_clean { Value::Array(clean_files.into_iter().map(Value::String).collect()) } else { Value::Null },
            "errors": errors
        }))
    }

    fn tool_status(&self) -> Value {
        json!({
            "workspaceRoot": self.root.display().to_string(),
            "workspaceUri": file_uri(&self.root).map(|uri| uri.to_string()).ok(),
            "lspBinary": lsp_binary_display(),
            "initialized": self.lsp.is_some()
        })
    }

    fn ensure_lsp_for_root(&mut self, root: PathBuf) -> Result<(), String> {
        if self.root != root {
            self.lsp = None;
            self.root = root;
        }

        if self.lsp.is_none() {
            self.lsp = Some(LspSession::start(self.root.clone())?);
        }

        Ok(())
    }

    fn eager_initialize_workspace(&mut self) {
        let root = match absolute_path(default_workspace_root()) {
            Ok(root) => root,
            Err(err) => {
                eprintln!("failed to resolve MCP workspace root during eager init: {err}");
                return;
            }
        };

        if let Err(err) = self.ensure_lsp_for_root(root) {
            eprintln!("failed to eagerly initialize objectscript-lsp: {err}");
        }
    }
}

struct LspSession {
    root: PathBuf,
    root_uri: String,
    child: Child,
    writer: Arc<Mutex<ChildStdin>>,
    pending: Arc<Mutex<HashMap<u64, Sender<Value>>>>,
    applied_edit_paths: Arc<Mutex<Vec<PathBuf>>>,
    next_id: AtomicU64,
    document_versions: HashMap<PathBuf, i32>,
}

struct ExecuteCommandResult {
    result: Value,
    applied_edit_count: usize,
    applied_edit_paths: Vec<String>,
}

#[derive(Clone)]
struct TextReplacement {
    start_byte: usize,
    end_byte: usize,
    new_text: String,
}

impl LspSession {
    fn start(root: PathBuf) -> Result<Self, String> {
        let root_uri = file_uri(&root)?.to_string();
        let mut child = Command::new(resolve_lsp_binary())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| format!("failed to start objectscript-lsp: {err}"))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "failed to open objectscript-lsp stdin".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "failed to open objectscript-lsp stdout".to_string())?;

        if let Some(stderr) = child.stderr.take() {
            std::thread::spawn(move || forward_child_stderr(stderr));
        }

        let writer = Arc::new(Mutex::new(stdin));
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let applied_edit_paths = Arc::new(Mutex::new(Vec::new()));
        std::thread::spawn({
            let writer = Arc::clone(&writer);
            let pending = Arc::clone(&pending);
            let applied_edit_paths = Arc::clone(&applied_edit_paths);
            let root = root.clone();
            let root_uri = root_uri.clone();
            let root_name = workspace_name(&root);
            move || {
                read_lsp_messages(
                    stdout,
                    writer,
                    pending,
                    applied_edit_paths,
                    root,
                    root_uri,
                    root_name,
                )
            }
        });

        let session = Self {
            root,
            root_uri,
            child,
            writer,
            pending,
            applied_edit_paths,
            next_id: AtomicU64::new(1),
            document_versions: HashMap::new(),
        };

        session.initialize()?;
        Ok(session)
    }

    fn initialize(&self) -> Result<(), String> {
        self.send_request(
            "initialize",
            json!({
                "processId": null,
                "rootUri": self.root_uri,
                "workspaceFolders": [
                    {
                        "uri": self.root_uri,
                        "name": workspace_name(&self.root)
                    }
                ],
                "capabilities": {
                    "workspace": {
                        "applyEdit": true,
                        "workspaceFolders": true,
                        "didChangeWatchedFiles": {
                            "dynamicRegistration": true
                        }
                    },
                    "textDocument": {
                        "codeAction": {
                            "dynamicRegistration": false
                        },
                        "diagnostic": {
                            "dynamicRegistration": false
                        }
                    }
                },
                "initializationOptions": {}
            }),
        )?;
        self.send_notification("initialized", json!({}))?;
        Ok(())
    }

    fn open_document(&mut self, path: &Path) -> Result<(), String> {
        let uri = file_uri(path)?.to_string();
        let text = std::fs::read_to_string(path)
            .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        let version = {
            let entry = self
                .document_versions
                .entry(path.to_path_buf())
                .or_insert(0);
            *entry += 1;
            *entry
        };
        self.send_notification(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": uri,
                    "languageId": language_id(path)?,
                    "version": version,
                    "text": text
                }
            }),
        )
    }

    fn diagnostics(&mut self, path: &Path) -> Result<Value, String> {
        self.open_document(path)?;

        self.send_request(
            "textDocument/diagnostic",
            json!({
                "textDocument": {
                    "uri": file_uri(path)?.to_string()
                }
            }),
        )
    }

    fn goto_definition(&mut self, path: &Path, line: u32, character: u32) -> Result<Value, String> {
        self.open_document(path)?;

        self.send_request(
            "textDocument/definition",
            json!({
                "textDocument": {
                    "uri": file_uri(path)?.to_string()
                },
                "position": {
                    "line": line,
                    "character": character
                }
            }),
        )
    }

    fn code_actions(
        &mut self,
        path: &Path,
        range: Value,
        only: Vec<String>,
        trigger_kind: u64,
    ) -> Result<Value, String> {
        self.open_document(path)?;

        self.send_request(
            "textDocument/codeAction",
            json!({
                "textDocument": {
                    "uri": file_uri(path)?.to_string()
                },
                "range": range,
                "context": {
                    "diagnostics": [],
                    "only": only,
                    "triggerKind": trigger_kind
                }
            }),
        )
    }

    fn execute_command(
        &mut self,
        command: &str,
        arguments: Vec<Value>,
    ) -> Result<ExecuteCommandResult, String> {
        let result = self.send_request(
            "workspace/executeCommand",
            json!({
                "command": command,
                "arguments": arguments
            }),
        )?;
        let edited_paths = self.drain_applied_edit_paths()?;
        for path in &edited_paths {
            self.sync_document(path)?;
        }

        Ok(ExecuteCommandResult {
            result,
            applied_edit_count: edited_paths.len(),
            applied_edit_paths: edited_paths
                .iter()
                .map(|path| path.display().to_string())
                .collect(),
        })
    }

    fn sync_document(&mut self, path: &Path) -> Result<(), String> {
        let text = std::fs::read_to_string(path).map_err(|err| {
            format!(
                "failed to read {} after applying edits: {err}",
                path.display()
            )
        })?;
        let version = {
            let entry = self
                .document_versions
                .entry(path.to_path_buf())
                .or_insert(0);
            *entry += 1;
            *entry
        };

        self.send_notification(
            "textDocument/didChange",
            json!({
                "textDocument": {
                    "uri": file_uri(path)?.to_string(),
                    "version": version
                },
                "contentChanges": [
                    {
                        "text": text
                    }
                ]
            }),
        )
    }

    fn drain_applied_edit_paths(&self) -> Result<Vec<PathBuf>, String> {
        let mut paths = self
            .applied_edit_paths
            .lock()
            .map_err(|_| "applied edit path lock poisoned".to_string())?;
        let mut seen = HashSet::new();
        let mut unique_paths = Vec::new();
        for path in paths.drain(..) {
            if seen.insert(path.clone()) {
                unique_paths.push(path);
            }
        }
        Ok(unique_paths)
    }

    fn send_request(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        self.pending
            .lock()
            .map_err(|_| "pending request map lock poisoned".to_string())?
            .insert(id, tx);

        let message = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });

        if let Err(err) = write_lsp_message(&self.writer, &message) {
            let _ = self.pending.lock().map(|mut pending| pending.remove(&id));
            return Err(err);
        }

        let response = rx
            .recv_timeout(DEFAULT_REQUEST_TIMEOUT)
            .map_err(|_| format!("timed out waiting for LSP response to {method}"))?;

        if let Some(error) = response.get("error") {
            return Err(format!("LSP request {method} failed: {error}"));
        }

        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }

    fn send_notification(&self, method: &str, params: Value) -> Result<(), String> {
        write_lsp_message(
            &self.writer,
            &json!({
                "jsonrpc": "2.0",
                "method": method,
                "params": params
            }),
        )
    }
}

impl Drop for LspSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn read_lsp_messages(
    stdout: impl Read,
    writer: Arc<Mutex<ChildStdin>>,
    pending: Arc<Mutex<HashMap<u64, Sender<Value>>>>,
    applied_edit_paths: Arc<Mutex<Vec<PathBuf>>>,
    root: PathBuf,
    root_uri: String,
    root_name: String,
) {
    let mut reader = BufReader::new(stdout);

    loop {
        let Some(content_length) = read_content_length(&mut reader) else {
            break;
        };

        let mut body = vec![0; content_length];
        if let Err(err) = reader.read_exact(&mut body) {
            if err.kind() == std::io::ErrorKind::UnexpectedEof {
                break;
            }
            eprintln!("failed to read LSP body: {err}");
            break;
        }

        let Ok(message) = serde_json::from_slice::<Value>(&body) else {
            eprintln!("failed to parse LSP message");
            continue;
        };

        if message.get("method").is_some() && message.get("id").is_some() {
            respond_to_lsp_request(
                &writer,
                &message,
                &root,
                &root_uri,
                &root_name,
                &applied_edit_paths,
            );
        } else if let Some(id) = message.get("id").and_then(Value::as_u64) {
            if let Ok(mut pending) = pending.lock() {
                if let Some(tx) = pending.remove(&id) {
                    let _ = tx.send(message);
                }
            }
        }
    }
}

fn read_content_length(reader: &mut BufReader<impl Read>) -> Option<usize> {
    let mut content_length = None;

    loop {
        let mut line = String::new();
        let bytes = reader.read_line(&mut line).ok()?;
        if bytes == 0 {
            return None;
        }

        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }

        if let Some(value) = trimmed.strip_prefix("Content-Length:") {
            content_length = value.trim().parse::<usize>().ok();
        }
    }

    content_length
}

fn respond_to_lsp_request(
    writer: &Arc<Mutex<ChildStdin>>,
    request: &Value,
    root: &Path,
    root_uri: &str,
    root_name: &str,
    applied_edit_paths: &Arc<Mutex<Vec<PathBuf>>>,
) {
    let Some(id) = request.get("id").cloned() else {
        return;
    };
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    let result = match method {
        "workspace/workspaceFolders" => json!([
            {
                "uri": root_uri,
                "name": root_name
            }
        ]),
        "workspace/applyEdit" => match apply_workspace_edit(root, request, applied_edit_paths) {
            Ok(_) => json!({
                "applied": true
            }),
            Err(err) => json!({
                "applied": false,
                "failureReason": err
            }),
        },
        _ => Value::Null,
    };

    let response = json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    });
    if let Err(err) = write_lsp_message(writer, &response) {
        eprintln!("failed to respond to LSP request {method}: {err}");
    }
}

fn write_lsp_message(writer: &Arc<Mutex<ChildStdin>>, message: &Value) -> Result<(), String> {
    let body = serde_json::to_vec(message).map_err(|err| err.to_string())?;
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    let mut writer = writer
        .lock()
        .map_err(|_| "LSP stdin lock poisoned".to_string())?;
    writer
        .write_all(header.as_bytes())
        .map_err(|err| format!("failed to write LSP header: {err}"))?;
    writer
        .write_all(&body)
        .map_err(|err| format!("failed to write LSP body: {err}"))?;
    writer
        .flush()
        .map_err(|err| format!("failed to flush LSP message: {err}"))
}

fn forward_child_stderr(stderr: impl Read) {
    let reader = BufReader::new(stderr);
    for line in reader.lines().map_while(Result::ok) {
        eprintln!("[objectscript-lsp] {line}");
    }
}

fn write_mcp_message(stdout: &mut impl Write, response: &Value) -> std::io::Result<()> {
    serde_json::to_writer(&mut *stdout, response)?;
    stdout.write_all(b"\n")?;
    stdout.flush()
}

fn json_rpc_result(id: Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    })
}

fn json_rpc_error(id: Value, code: i64, message: String, data: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message,
            "data": data
        }
    })
}

fn tool_json(value: Value) -> Value {
    json!({
        "content": [
            {
                "type": "text",
                "text": serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string())
            }
        ],
        "isError": false
    })
}

fn tool_error(message: String) -> Value {
    json!({
        "content": [
            {
                "type": "text",
                "text": message
            }
        ],
        "isError": true
    })
}

fn tools_list() -> Value {
    json!({
        "tools": [
            {
                "name": "objectscript_initialize_workspace",
                "description": "Start objectscript-lsp and initialize it for the Claude project directory, or for the optional root path.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "root": {
                            "type": "string",
                            "description": "Optional workspace root. Defaults to CLAUDE_PROJECT_DIR, then the MCP process current directory."
                        }
                    }
                }
            },
            {
                "name": "objectscript_diagnostics",
                "description": "Open an ObjectScript/XML file in objectscript-lsp and return pull diagnostics from textDocument/diagnostic.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "file_path": {
                            "type": "string",
                            "description": "Path to a .cls, .mac, .inc, .rtn, .int, or .xml file. Relative paths resolve from the workspace root."
                        },
                        "root": {
                            "type": "string",
                            "description": "Optional workspace root override."
                        }
                    },
                    "required": ["file_path"]
                }
            },
            {
                "name": "objectscript_goto_definition",
                "description": "Open an ObjectScript/XML file in objectscript-lsp and return the textDocument/definition result for a 1-based source position.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "file_path": {
                            "type": "string",
                            "description": "Path to a .cls, .mac, .inc, .rtn, .int, or .xml file. Relative paths resolve from the workspace root."
                        },
                        "line": {
                            "type": "integer",
                            "minimum": 1,
                            "description": "1-based line number for the symbol reference."
                        },
                        "character": {
                            "type": "integer",
                            "minimum": 1,
                            "description": "1-based character/column for the symbol reference."
                        },
                        "root": {
                            "type": "string",
                            "description": "Optional workspace root override."
                        }
                    },
                    "required": ["file_path", "line", "character"]
                }
            },
            {
                "name": "objectscript_code_actions",
                "description": "Return ObjectScript LSP textDocument/codeAction results for a file. Defaults to refactor.rewrite actions.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "file_path": {
                            "type": "string",
                            "description": "Path to a .cls, .mac, .inc, .rtn, .int, or .xml file. Relative paths resolve from the workspace root."
                        },
                        "line": {
                            "type": "integer",
                            "minimum": 1,
                            "description": "Optional 1-based cursor line. Used as a collapsed range when start/end are omitted."
                        },
                        "character": {
                            "type": "integer",
                            "minimum": 1,
                            "description": "Optional 1-based cursor character/column. Used with line as a collapsed range."
                        },
                        "start_line": {
                            "type": "integer",
                            "minimum": 1,
                            "description": "Optional 1-based start line for the code-action range."
                        },
                        "start_character": {
                            "type": "integer",
                            "minimum": 1,
                            "description": "Optional 1-based start character/column for the code-action range."
                        },
                        "end_line": {
                            "type": "integer",
                            "minimum": 1,
                            "description": "Optional 1-based end line for the code-action range."
                        },
                        "end_character": {
                            "type": "integer",
                            "minimum": 1,
                            "description": "Optional 1-based end character/column for the code-action range."
                        },
                        "only": {
                            "type": "array",
                            "items": {
                                "type": "string"
                            },
                            "description": "Optional LSP code-action kinds. Defaults to [\"refactor.rewrite\"]."
                        },
                        "trigger_kind": {
                            "type": "string",
                            "enum": ["invoked", "automatic"],
                            "description": "Optional LSP code-action trigger kind. Defaults to invoked."
                        },
                        "root": {
                            "type": "string",
                            "description": "Optional workspace root override."
                        }
                    },
                    "required": ["file_path"]
                }
            },
            {
                "name": "objectscript_execute_command",
                "description": "Execute an allowlisted ObjectScript LSP command returned by a code action and apply any workspace edits to files under the workspace root.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "command": {
                            "type": "string",
                            "description": "ObjectScript LSP command to execute, such as objectscript.refactorDocument or objectscript.refactorWorkspace."
                        },
                        "arguments": {
                            "type": "array",
                            "description": "Command arguments exactly as returned from objectscript_code_actions."
                        },
                        "root": {
                            "type": "string",
                            "description": "Optional workspace root override."
                        }
                    },
                    "required": ["command"]
                }
            },
            {
                "name": "objectscript_workspace_diagnostics",
                "description": "Run pull diagnostics for every supported ObjectScript/XML file under the workspace root.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "root": {
                            "type": "string",
                            "description": "Optional workspace root override. Defaults to the initialized workspace root."
                        },
                        "include_clean": {
                            "type": "boolean",
                            "description": "When true, include clean file paths in cleanFiles. Defaults to false."
                        },
                        "max_files": {
                            "type": "integer",
                            "minimum": 1,
                            "description": "Optional safety limit for the number of supported files to check."
                        }
                    }
                }
            },
            {
                "name": "objectscript_lsp_status",
                "description": "Show the workspace root, LSP binary path, and whether the MCP bridge has initialized objectscript-lsp.",
                "inputSchema": {
                    "type": "object",
                    "properties": {}
                }
            }
        ]
    })
}

fn optional_string<'a>(arguments: &'a Value, key: &str) -> Option<&'a str> {
    arguments.get(key).and_then(Value::as_str)
}

fn optional_bool(arguments: &Value, key: &str) -> bool {
    arguments.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn optional_usize(arguments: &Value, key: &str) -> Result<Option<usize>, String> {
    let Some(value) = arguments.get(key) else {
        return Ok(None);
    };
    let Some(value) = value.as_u64() else {
        return Err(format!("{key} must be a positive integer"));
    };
    if value == 0 {
        return Err(format!("{key} must be greater than zero"));
    }
    usize::try_from(value)
        .map(Some)
        .map_err(|_| format!("{key} is too large"))
}

fn optional_array(arguments: &Value, key: &str) -> Result<Option<Vec<Value>>, String> {
    let Some(value) = arguments.get(key) else {
        return Ok(None);
    };
    value
        .as_array()
        .cloned()
        .map(Some)
        .ok_or_else(|| format!("{key} must be an array"))
}

fn optional_string_array(arguments: &Value, key: &str) -> Result<Option<Vec<String>>, String> {
    let Some(values) = optional_array(arguments, key)? else {
        return Ok(None);
    };
    let mut strings = Vec::with_capacity(values.len());
    for value in values {
        let Some(string) = value.as_str() else {
            return Err(format!("{key} must contain only strings"));
        };
        strings.push(string.to_string());
    }
    Ok(Some(strings))
}

fn required_one_based_u32(arguments: &Value, key: &str) -> Result<u32, String> {
    let Some(value) = arguments.get(key) else {
        return Err(format!("missing required argument: {key}"));
    };
    let Some(value) = value.as_u64() else {
        return Err(format!("{key} must be a positive integer"));
    };
    if value == 0 {
        return Err(format!("{key} must be greater than zero"));
    }
    u32::try_from(value).map_err(|_| format!("{key} is too large"))
}

fn optional_one_based_u32(arguments: &Value, key: &str) -> Result<Option<u32>, String> {
    let Some(value) = arguments.get(key) else {
        return Ok(None);
    };
    let Some(value) = value.as_u64() else {
        return Err(format!("{key} must be a positive integer"));
    };
    if value == 0 {
        return Err(format!("{key} must be greater than zero"));
    }
    u32::try_from(value)
        .map(Some)
        .map_err(|_| format!("{key} is too large"))
}

fn optional_code_action_trigger_kind(arguments: &Value) -> Result<u64, String> {
    match optional_string(arguments, "trigger_kind").unwrap_or("invoked") {
        "invoked" => Ok(1),
        "automatic" => Ok(2),
        trigger_kind => Err(format!(
            "trigger_kind must be either \"invoked\" or \"automatic\", got {trigger_kind:?}"
        )),
    }
}

fn code_action_range(arguments: &Value) -> Result<Value, String> {
    let line = optional_one_based_u32(arguments, "line")?.unwrap_or(1);
    let character = optional_one_based_u32(arguments, "character")?.unwrap_or(1);
    let start_line = optional_one_based_u32(arguments, "start_line")?.unwrap_or(line);
    let start_character =
        optional_one_based_u32(arguments, "start_character")?.unwrap_or(character);
    let end_line = optional_one_based_u32(arguments, "end_line")?.unwrap_or(start_line);
    let end_character =
        optional_one_based_u32(arguments, "end_character")?.unwrap_or(start_character);

    Ok(json!({
        "start": {
            "line": start_line - 1,
            "character": start_character - 1
        },
        "end": {
            "line": end_line - 1,
            "character": end_character - 1
        }
    }))
}

fn is_allowed_lsp_command(command: &str) -> bool {
    matches!(
        command,
        REFACTOR_DOCUMENT_COMMAND
            | REFACTOR_WORKSPACE_COMMAND
            | LEGACY_REFACTOR_WORKSPACE_DO_COMMAND
    )
}

fn command_document_paths(root: &Path, arguments: &[Value]) -> Result<Vec<PathBuf>, String> {
    let mut paths = Vec::new();
    let mut seen = HashSet::new();
    for argument in arguments {
        let Some(uri) = argument.as_str() else {
            continue;
        };
        if !uri.starts_with("file://") {
            continue;
        }
        let path = path_from_uri_under_root(root, uri)?;
        if seen.insert(path.clone()) {
            paths.push(path);
        }
    }
    Ok(paths)
}

fn apply_workspace_edit(
    root: &Path,
    request: &Value,
    applied_edit_paths: &Arc<Mutex<Vec<PathBuf>>>,
) -> Result<usize, String> {
    let edit = request
        .get("params")
        .and_then(|params| params.get("edit"))
        .ok_or_else(|| "workspace/applyEdit request did not include params.edit".to_string())?;

    let mut changed_paths = Vec::new();
    let mut edit_count = 0usize;

    if let Some(changes) = edit.get("changes").and_then(Value::as_object) {
        edit_count += apply_changes_map(root, changes, &mut changed_paths)?;
    }

    if let Some(document_changes) = edit.get("documentChanges").and_then(Value::as_array) {
        edit_count += apply_document_changes(root, document_changes, &mut changed_paths)?;
    }

    if !changed_paths.is_empty() {
        applied_edit_paths
            .lock()
            .map_err(|_| "applied edit path lock poisoned".to_string())?
            .extend(changed_paths);
    }

    Ok(edit_count)
}

fn apply_changes_map(
    root: &Path,
    changes: &Map<String, Value>,
    changed_paths: &mut Vec<PathBuf>,
) -> Result<usize, String> {
    let mut edit_count = 0usize;
    for (uri, edits) in changes {
        edit_count += apply_uri_text_edits(root, uri, edits, changed_paths)?;
    }
    Ok(edit_count)
}

fn apply_document_changes(
    root: &Path,
    document_changes: &[Value],
    changed_paths: &mut Vec<PathBuf>,
) -> Result<usize, String> {
    let mut edit_count = 0usize;
    for change in document_changes {
        if let Some(kind) = change.get("kind").and_then(Value::as_str) {
            return Err(format!(
                "workspace/applyEdit does not support {kind:?} document changes"
            ));
        }
        let uri = change
            .get("textDocument")
            .and_then(|text_document| text_document.get("uri"))
            .and_then(Value::as_str)
            .ok_or_else(|| "documentChanges item is missing textDocument.uri".to_string())?;
        let edits = change
            .get("edits")
            .ok_or_else(|| "documentChanges item is missing edits".to_string())?;
        edit_count += apply_uri_text_edits(root, uri, edits, changed_paths)?;
    }
    Ok(edit_count)
}

fn apply_uri_text_edits(
    root: &Path,
    uri: &str,
    edits: &Value,
    changed_paths: &mut Vec<PathBuf>,
) -> Result<usize, String> {
    let edits = edits
        .as_array()
        .ok_or_else(|| format!("text edits for {uri} must be an array"))?;
    if edits.is_empty() {
        return Ok(0);
    }

    let path = path_from_uri_under_root(root, uri)?;
    apply_text_edits_to_file(&path, edits)?;
    changed_paths.push(path);
    Ok(edits.len())
}

fn path_from_uri_under_root(root: &Path, uri: &str) -> Result<PathBuf, String> {
    let url = Url::parse(uri).map_err(|err| format!("invalid file URI {uri:?}: {err}"))?;
    let path = url
        .to_file_path()
        .map_err(|_| format!("URI is not a file path: {uri}"))?;
    if !path.exists() {
        return Err(format!(
            "cannot apply edit to missing file: {}",
            path.display()
        ));
    }

    let canonical_root = std::fs::canonicalize(root)
        .map_err(|err| format!("failed to canonicalize root {}: {err}", root.display()))?;
    let canonical_path = std::fs::canonicalize(&path)
        .map_err(|err| format!("failed to canonicalize edit path {}: {err}", path.display()))?;
    if !canonical_path.starts_with(&canonical_root) {
        return Err(format!(
            "refusing to apply edit outside workspace root: {}",
            path.display()
        ));
    }

    Ok(path)
}

fn apply_text_edits_to_file(path: &Path, edits: &[Value]) -> Result<(), String> {
    let original = std::fs::read_to_string(path)
        .map_err(|err| format!("failed to read {} for text edits: {err}", path.display()))?;
    let mut replacements = Vec::with_capacity(edits.len());
    for edit in edits {
        replacements.push(parse_text_replacement(&original, edit)?);
    }

    replacements.sort_by(|left, right| {
        right
            .start_byte
            .cmp(&left.start_byte)
            .then_with(|| right.end_byte.cmp(&left.end_byte))
    });

    let mut next_start = usize::MAX;
    for replacement in &replacements {
        if replacement.end_byte > next_start {
            return Err(format!("overlapping text edits for {}", path.display()));
        }
        next_start = replacement.start_byte;
    }

    let mut updated = original;
    for replacement in replacements {
        updated.replace_range(
            replacement.start_byte..replacement.end_byte,
            replacement.new_text.as_str(),
        );
    }
    std::fs::write(path, updated)
        .map_err(|err| format!("failed to write text edits to {}: {err}", path.display()))
}

fn parse_text_replacement(original: &str, edit: &Value) -> Result<TextReplacement, String> {
    let range = edit
        .get("range")
        .ok_or_else(|| "text edit is missing range".to_string())?;
    let start = parse_zero_based_position(
        range
            .get("start")
            .ok_or_else(|| "text edit range is missing start".to_string())?,
        "range.start",
    )?;
    let end = parse_zero_based_position(
        range
            .get("end")
            .ok_or_else(|| "text edit range is missing end".to_string())?,
        "range.end",
    )?;
    let new_text = edit
        .get("newText")
        .and_then(Value::as_str)
        .ok_or_else(|| "text edit is missing newText".to_string())?
        .to_string();

    let start_byte = point_to_byte(original, position_to_point(original, start));
    let end_byte = point_to_byte(original, position_to_point(original, end));
    if start_byte > end_byte {
        return Err("text edit range start is after range end".to_string());
    }

    Ok(TextReplacement {
        start_byte,
        end_byte,
        new_text,
    })
}

fn parse_zero_based_position(value: &Value, label: &str) -> Result<Position, String> {
    let line = value
        .get("line")
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{label}.line must be a non-negative integer"))?;
    let character = value
        .get("character")
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{label}.character must be a non-negative integer"))?;
    Ok(Position::new(
        u32::try_from(line).map_err(|_| format!("{label}.line is too large"))?,
        u32::try_from(character).map_err(|_| format!("{label}.character is too large"))?,
    ))
}

fn default_workspace_root() -> PathBuf {
    env::var_os("CLAUDE_PROJECT_DIR")
        .map(PathBuf::from)
        .or_else(|| env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn absolute_path(path: PathBuf) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path)
    } else {
        env::current_dir()
            .map(|cwd| cwd.join(path))
            .map_err(|err| format!("failed to resolve current directory: {err}"))
    }
}

fn resolve_workspace_path(root: &Path, file_path: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(file_path);
    let path = if path.is_absolute() {
        path
    } else {
        root.join(path)
    };
    if !path.exists() {
        return Err(format!("file does not exist: {}", path.display()));
    }
    Ok(path)
}

fn file_uri(path: &Path) -> Result<Url, String> {
    Url::from_file_path(path)
        .map_err(|_| format!("failed to convert path to URI: {}", path.display()))
}

fn workspace_name(root: &Path) -> String {
    root.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("workspace")
        .to_string()
}

fn language_id(path: &Path) -> Result<&'static str, String> {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("cls") => Ok("objectscript_udl"),
        Some("inc") | Some("rtn") | Some("mac") | Some("int") => Ok("objectscript_routine"),
        Some("xml") => Ok("xml"),
        _ => Err(format!(
            "unsupported file extension for diagnostics: {}",
            path.display()
        )),
    }
}

fn is_supported_file(path: &Path) -> bool {
    language_id(path).is_ok()
}

fn workspace_relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn diagnostic_items(report: &Value) -> Vec<Value> {
    report
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn resolve_lsp_binary() -> PathBuf {
    if let Some(binary) = env::var_os("OBJECTSCRIPT_LSP_BINARY") {
        return PathBuf::from(binary);
    }

    if let Ok(current_exe) = env::current_exe() {
        if let Some(parent) = current_exe.parent() {
            let sibling = parent.join("objectscript-lsp");
            if sibling.exists() {
                return sibling;
            }
        }
    }

    PathBuf::from("objectscript-lsp")
}

fn lsp_binary_display() -> String {
    resolve_lsp_binary().display().to_string()
}
