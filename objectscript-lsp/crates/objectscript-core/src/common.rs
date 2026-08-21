use crate::parse_structures::{ClassId, MemberType, MethodType, ReturnType};
use crate::refactor::count_leading_dots_in_line;
use crate::scope_structures::ScopeId;
use crate::scope_tree::ScopeTree;
use regex::Regex;
use std::collections::HashSet;
use std::ops::Range as CoreRange;
use std::sync::OnceLock;
use tower_lsp::lsp_types::{Position, Range as LspRange, Url};
use tree_sitter::{
    Language as TsLanguage, Node, Point, Query, QueryCursor, Range as TsRange, Range,
    StreamingIterator, Tree, TreeCursor,
};
use tree_sitter_objectscript::LANGUAGE_OBJECTSCRIPT_UDL;

use tree_sitter_xml::LANGUAGE_XML;

const XML_OBJECTSCRIPT_INJECTIONS_QUERY: &str = r#"
(
  element
    (STag (Name) @_start_tag)
    (content (CDSect (CData) @injection.content))
    (ETag (Name) @_end_tag)
  (#eq? @_start_tag "Implementation")
  (#eq? @_end_tag "Implementation")
  (#set! injection.language "objectscript")
)
(
  element
    (STag (Name) @_start_tag)
    (content (CharData) @injection.content)
    (ETag (Name) @_end_tag)
  (#eq? @_start_tag "Implementation")
  (#eq? @_end_tag "Implementation")
  (#set! injection.language "objectscript")
)
"#;

const CLASS_NAME_QUERY: &str = "(class_definition (class_name (identifier) @classname))";

fn cached_query(
    query: &'static OnceLock<Query>,
    language: TsLanguage,
    source: &str,
    name: &str,
) -> &'static Query {
    query.get_or_init(|| {
        Query::new(&language, source)
            .unwrap_or_else(|error| panic!("failed to compile {name} Tree-sitter query: {error}"))
    })
}

fn class_name_query() -> &'static Query {
    static QUERY: OnceLock<Query> = OnceLock::new();
    cached_query(
        &QUERY,
        LANGUAGE_OBJECTSCRIPT_UDL.into(),
        CLASS_NAME_QUERY,
        "class name",
    )
}

fn xml_objectscript_injections_query() -> &'static Query {
    static QUERY: OnceLock<Query> = OnceLock::new();
    cached_query(
        &QUERY,
        LANGUAGE_XML.into(),
        XML_OBJECTSCRIPT_INJECTIONS_QUERY,
        "XML ObjectScript injections",
    )
}

/// Logs override resolution results for a method/superclass pair for debugging.
pub fn print_statements_exit_method_overrides_fn(
    method_name: &str,
    superclass_name: &str,
    locations: Vec<(Url, Range)>,
) {
    if locations.is_empty() {
        eprintln!(
            "Leaving ProjectData function: get_variable_symbol_location.., there are no overrides of method {:?} from superclass {:?}",
            method_name, superclass_name
        );
        eprintln!("------------------------");
        eprintln!();
        return;
    }
    eprintln!(
        "Leaving ProjectData function: get_variable_symbol_location.. the number of method implementations of the method named: {:?} in the superclass {:?} are:  \n {:?}",
        method_name,
        superclass_name,
        locations.len()
    );
    eprintln!("------------------------");
    eprintln!();
}

/// Converts a tree-sitter `Point` (UTF-8 byte column) to an LSP `Position` (UTF-16 code-unit offset).
pub fn point_to_lsp_position(text: &str, p: Point) -> Position {
    let starts = line_starts(text);
    let (line_start, _line_end_incl, line_end_excl) = line_bounds(text, &starts, p.row);

    // If point is on EOF row, map to UTF-16 character 0
    if line_start == text.len() && line_end_excl == text.len() {
        eprintln!("Info: Point is on EOF row, mapping to UTF-16 character 0");
        return Position {
            line: p.row as u32,
            character: 0,
        };
    }

    // Clamp column to visible line (exclude '\n')
    let max_col = line_end_excl.saturating_sub(line_start);
    let target_col = p.column.min(max_col);

    let line = &text[line_start..line_end_excl];

    let mut bytes = 0usize;
    let mut utf16_units = 0u32;

    for ch in line.chars() {
        let ch_bytes = ch.len_utf8();
        if bytes + ch_bytes > target_col {
            break;
        }
        bytes += ch_bytes;
        utf16_units += ch.len_utf16() as u32;
        if bytes == target_col {
            break;
        }
    }
    Position {
        line: p.row as u32,
        character: utf16_units,
    }
}

fn line_starts(text: &str) -> Vec<usize> {
    let mut starts = Vec::new();
    starts.push(0); // line 0 starts at byte 0

    for (i, b) in text.as_bytes().iter().enumerate() {
        if *b == b'\n' {
            starts.push(i + 1); // next line starts right after '\n'
        }
    }

    starts.push(text.len());

    starts
}

/// Returns byte bounds for a specific line in `text` using a precomputed line-start table.
///
/// `starts` is a slice of byte offsets where each element is the start index of a line.
/// It must include a final **sentinel** entry equal to `text.len()` (or the byte offset
/// immediately after the last line), so `starts.len() == number_of_lines + 1`.
///
/// For a valid `row` (0-based), this function returns:
/// - `start`: the byte offset where line `row` begins.
/// - `end_incl`: the byte offset of the start of the *next* line (i.e. one past the end of this line,
///   including the trailing `\n` if present).
/// - `end_excl`: the byte offset one past the end of the line content, excluding a trailing `\n` if present.
///
/// If `row` is out of range (`row >= starts.len() - 1`), returns `(len, len, len)` where `len = text.len()`.
///
/// - If `starts` is missing the sentinel, too short, or contains out-of-range offsets,
///   prints a warning and returns `(len, len, len)`.
fn line_bounds(text: &str, starts: &[usize], row: usize) -> (usize, usize, usize) {
    let len = text.len();

    // Need at least one line start + sentinel
    if starts.len() < 2 {
        eprintln!(
            "Error: line_bounds: invalid starts table (len={}), expected at least 2 (including sentinel).",
            starts.len()
        );
        return (len, len, len);
    }

    // Sentinel should typically be == text.len()
    let sentinel = *starts.last().unwrap();
    if sentinel > len {
        eprintln!(
            "Error: line_bounds: sentinel {} out of bounds for text len {}.",
            sentinel, len
        );
        return (len, len, len);
    }

    let eof_row = starts.len() - 1; // last entry is sentinel
    if row >= eof_row {
        // out of range row => "EOF bounds"
        return (len, len, len);
    }

    let start = starts[row];
    let end_incl = starts[row + 1];

    // Validate monotonic + in-bounds
    if start > end_incl || end_incl > len {
        eprintln!(
            "Error: line_bounds: invalid bounds for row {}: start={}, end_incl={}, text_len={}.",
            row, start, end_incl, len
        );
        return (len, len, len);
    }

    let end_excl = if end_incl > start && text.as_bytes().get(end_incl - 1) == Some(&b'\n') {
        end_incl - 1
    } else {
        end_incl
    };

    (start, end_incl, end_excl)
}

/// Converts an LSP `Position` (line + UTF-16 character offset) into a Tree-sitter `Point`
/// (row + UTF-8 byte column) for the given source `text`.
///
/// LSP positions encode `character` as a count of UTF-16 code units from the start of the line.
/// Tree-sitter points encode `column` as a byte offset (UTF-8) from the start of the line.
/// This function bridges those two coordinate systems by:
/// 1) locating the requested line bounds in `text`, and
/// 2) walking the line’s Unicode scalar values to convert a UTF-16 offset into a UTF-8 byte column.
///
/// If `position.line` is at or beyond EOF (as determined by `line_bounds`), this returns an
/// EOF-like point with `{ row, column: 0 }`.
///
/// If `position.character` lands in the middle of a surrogate pair boundary (i.e. between the
/// two UTF-16 code units used by a single non-BMP character), the conversion stops early and
/// returns the column at the start of that character (it does not split the pair).
///
/// # Notes
/// - `row` is zero-based, matching both LSP and Tree-sitter.
/// - The returned `column` is a byte offset within the line (0-based).
pub fn position_to_point(text: &str, position: Position) -> Point {
    let starts = line_starts(text);
    let row = position.line as usize;

    let (line_start, _line_end_incl, line_end_excl) = line_bounds(text, &starts, row);

    // If row is EOF (or beyond), return EOF point
    if line_start == text.len() && line_end_excl == text.len() {
        return Point { row, column: 0 };
    }

    let line = &text[line_start..line_end_excl];

    // Convert UTF-16 units to byte offset within this line
    let mut remaining = position.character as usize;
    let mut col_bytes = 0usize;

    for ch in line.chars() {
        if remaining == 0 {
            break;
        }
        let u16 = ch.len_utf16();
        if remaining < u16 {
            break; // don't split a surrogate pair
        }
        remaining -= u16;
        col_bytes += ch.len_utf8();
    }
    Point {
        row,
        column: col_bytes,
    }
}

/// Converts a Tree-sitter `Range` into an LSP `Range` for the given source `text`.
///
/// Tree-sitter ranges are expressed as start/end `Point`s where the `column` is a UTF-8 byte
/// offset within the line. LSP ranges are expressed as start/end `Position`s where the
/// `character` is a UTF-16 code-unit offset within the line.
///
/// This function performs the conversion by translating both `start_point` and `end_point`
/// via `point_to_lsp_position`.
pub fn ts_range_to_lsp_range(text: &str, r: TsRange) -> LspRange {
    let start = point_to_lsp_position(text, r.start_point);
    let end = point_to_lsp_position(text, r.end_point);
    LspRange { start, end }
}

/// Converts a Tree-sitter `Point` (row + UTF-8 byte column) into an absolute byte offset
/// into `text`.
///
/// This function uses a precomputed line-start table (`line_starts`) and `line_bounds` to:
/// - find the start of `point.row`,
/// - interpret `point.column` as a UTF-8 byte offset within that line, and
/// - return the absolute byte index `line_start + column`.
///
/// Behavior at boundaries:
/// - If `point.row` is at or beyond the EOF row (based on the sentinel in `line_starts`),
///   this returns `text.len()`.
/// - The column is clamped to the end of the line content (excluding a trailing `'\n'`),
///   so the returned offset will not point past the line’s non-newline characters.
///
/// # Notes
/// - `point.row` and `point.column` are both zero-based.
/// - The returned value is a byte index into `text` (suitable for slicing on UTF-8
///   boundaries, assuming `point.column` came from Tree-sitter / valid byte columns).
pub fn point_to_byte(text: &str, point: Point) -> usize {
    let starts = line_starts(text);

    // starts has a sentinel at text.len(), so EOF row is starts.len() - 1
    let eof_row = starts.len().saturating_sub(1);

    // If point is on EOF row (or beyond), it's EOF byte offset
    if point.row >= eof_row {
        eprintln!("Info: reached the EOF row");
        return text.len();
    }

    let (line_start, _line_end_incl, line_end_excl) = line_bounds(text, &starts, point.row);

    // Clamp column to the visible line (excluding '\n')
    let max_col = line_end_excl.saturating_sub(line_start);
    let col = point.column.min(max_col);
    line_start + col
}

/// Advances `(row, column)` by `changed_text`, returning the resulting `Point`.
///
/// Newlines increment `row` and reset `column`; other chars add their UTF-8 byte length.
pub fn advance_point(mut row: usize, mut column: usize, changed_text: &str) -> Point {
    for c in changed_text.chars() {
        if c == '\n' {
            row += 1;
            column = 0;
        } else {
            column += c.len_utf8();
        }
    }
    Point { row, column }
}

/// Returns a Vec of all named children nodes for a given Tree Sitter Node.
pub fn get_node_children(node: Node) -> Vec<Node> {
    let mut cursor = node.walk();
    let result = node.named_children(&mut cursor).collect::<Vec<Node>>();
    result
}

/// Given a Node, finds if there is a class definition child node. If so, returns that.
pub fn find_class_definition(root: Node) -> Option<Node> {
    let mut cursor = root.walk();
    let result = root
        .named_children(&mut cursor)
        .find(|n| n.kind() == "class_definition");
    match result {
        None => {
            eprintln!("Error: Could not find class definition node from tree.",);
            result
        }
        Some(_) => result,
    }
}

/// Dispatches to class or routine name extraction based on the `is_rtn` flag.
pub fn get_member_name_and_range_from_root(
    content: &str,
    node: Node,
    is_rtn: bool,
) -> Option<(Range, String, Range)> {
    return if is_rtn {
        get_routine_name_from_root(content, node)
    } else {
        get_class_name_from_root(content, node)
    };
}

/// Extracts the class name from a parsed Tree-sitter root `node`.
///
/// Finds the `class_definition` node (via `find_class_definition`), then reads the class name
/// from its second named child (index `1`) and slices it from `content` using the node’s byte range.
///
/// Returns `None` if no class definition/name is found or if the byte range is invalid; prints a
/// warning on unexpected/mismatched structure.
fn get_class_name_from_root(content: &str, node: Node) -> Option<(Range, String, Range)> {
    let query = class_name_query();
    let mut cursor = QueryCursor::new();
    let mut iter = cursor.matches(query, node, content.as_bytes());
    while let Some(query_match) = iter.next() {
        let matched_node = query_match.captures[0].node; // this is the identifier node
        let mut parent_node = matched_node.parent();
        while let Some(parent) = parent_node {
            if parent.kind() == "class_definition" {
                if let Some(class_name) =
                    get_string_at_byte_range(content, matched_node.byte_range())
                {
                    return Some((parent.range(), class_name, matched_node.range()));
                }
            }
            parent_node = parent.parent();
        }
    }
    None
}

/// Given root node (source_file), find the routine name
fn get_routine_name_from_root(content: &str, root: Node) -> Option<(Range, String, Range)> {
    // either it starts as a statement or as a routine_def
    if let Some(node) = root.named_child(0) {
        match node.kind() {
            "routine_definition" => {
                let Some(name_node) = node.named_child(1) else {
                    eprintln!(
                        "Error: Routine definition node doesn't have a named child at index 0"
                    );
                    return None;
                };
                if name_node.kind() != "routine_name" {
                    eprintln!(
                        "Error: Expected routine def child node at index 0 to be routine_name"
                    );
                    return None;
                }

                if let Some(routine_name) =
                    get_string_at_byte_range(content, name_node.byte_range())
                {
                    return Some((root.range(), routine_name, name_node.range()));
                }
            }
            "statement" | "compiled_header" => {
                let routine_children = get_node_children(root);
                for statement_node in routine_children {
                    if statement_node.kind() != "statement" {
                        eprintln!(
                            "Skipping {:?} in get_routine_name_from_root",
                            statement_node.kind()
                        );
                        continue;
                    }
                    let Some(command) = statement_node.named_child(0) else {
                        eprintln!(
                            "Statement node did not have a child at index 0, continuing (get_routine_name_from_root)"
                        );
                        continue;
                    };
                    if command.kind() == "tag_statement" {
                        let Some(tag) = command.named_child(0) else {
                            eprintln!("Error: Expected tag_statement to have child at index 0");
                            continue;
                        };
                        if let Some(routine_name) =
                            get_string_at_byte_range(content, tag.byte_range())
                        {
                            return Some((root.range(), routine_name, tag.range()));
                        }
                    }
                }
            }
            _ => return None,
        }
    }
    None
}

/// Returns the substring for `range` (byte offsets) within `content`.
///
/// Logs a warning and returns `None` if the range is out of bounds.
pub fn get_string_at_byte_range(content: &str, range: CoreRange<usize>) -> Option<String> {
    let Some(s) = content.get(range) else {
        eprintln!("Error: Couldn't get string from given byte range");
        return None;
    };
    Some(s.to_string())
}

/// Maps a type name string (e.g. InterSystems % types) to a `ReturnType`.
///
/// Unrecognized names return `ReturnType::Other(typename)` and are logged as unimplemented.
pub fn find_return_type(typename: String) -> ReturnType {
    return match typename.to_lowercase().as_str() {
        "%exactstring" | "%enumstring" | "%string" | "%char" | "text" | "string" => {
            ReturnType::String
        }
        "%bigint" | "%smallint" | "%integer" | "%posixtime" | "%counter" | "integer" => {
            ReturnType::Integer
        }
        "%tinyint" => ReturnType::TinyInteger,
        "%binary" => ReturnType::Binary,
        "%date" => ReturnType::Date,
        "%double" => ReturnType::Double,
        "%numeric" | "%time" => ReturnType::Number,
        "%status" => ReturnType::Status,
        "%sqlquery" | "sql" => ReturnType::SqlQuery,
        "boolean" => ReturnType::Boolean,
        "classname" => ReturnType::ClassName,
        "coscode" => ReturnType::CosCode,
        "cosidentifier" => ReturnType::Variable,
        "cosexpression" => ReturnType::Expression,
        "sqlidentifier" => ReturnType::SqlIdentifier,
        "configvalue" => ReturnType::ConfigValue,
        _ => {
            // eprintln!("Unimplemented return type: {:?}", typename);
            ReturnType::Other(typename)
        }
    };
}

// given an expression node, parses the node and tries to evaluate the expression
pub fn evaluate_expr_from_node(offset: &str) -> Option<usize> {
    let val = evalexpr::eval_int(offset).ok();
    if let Some(val) = val {
        let new_val = val as usize;
        return Some(new_val);
    }
    None
}
/// Given an expression node, find all var types and var references within it
pub fn find_var_dependencies(
    node: Node,
    content: &str,
    var_dependencies: &mut Vec<String>,
) -> (bool, Option<String>) {
    let expression_children = get_node_children(node);
    let mut is_oref = false;
    let mut curr_class = None;
    for child in expression_children {
        match child.kind() {
            "expression" | "unary_expression" => {
                let (check_is_oref, cls) = find_var_dependencies(child, content, var_dependencies);
                if !is_oref {
                    is_oref = check_is_oref;
                }
                if curr_class.is_none() {
                    curr_class = cls;
                }
            }
            "class_method_call" => {
                if let Some(class_ref) = child.named_child(0)
                    && let Some(method_name_node) = child.named_child(1)
                    && let Some(class_name_node) = class_ref.named_child(1)
                {
                    // this part will remove the strings and such (it grabs the actual $.identifier node)
                    if let Some(method_name) = method_name_node.named_child(0)
                        && let Some(class_name) = class_name_node.named_child(0)
                    {
                        if let Some(method_name) =
                            get_string_at_byte_range(content, method_name.byte_range())
                        {
                            if method_name.eq_ignore_ascii_case("%new") {
                                is_oref = true;
                                curr_class =
                                    get_string_at_byte_range(content, class_name.byte_range());
                            }
                        }
                    }
                }
            }
            "gvn" => {
                let gvn_children = get_node_children(child);
                for gvn_child in gvn_children {
                    if gvn_child.kind() == "identifier" {
                        if let Some(gvn_id) =
                            get_string_at_byte_range(content, gvn_child.byte_range())
                        {
                            let var_name = gvn_id;
                            var_dependencies.push(var_name);
                        }
                    }
                }
            }
            "lvn" => {
                let Some(lvn_id_node) = child.named_child(0) else {
                    eprintln!("Parsing Error: lvn must have a child at index 0, update parsing");
                    continue;
                };
                if let Some(lvn_id) = get_string_at_byte_range(content, lvn_id_node.byte_range()) {
                    let var_name = lvn_id;
                    var_dependencies.push(var_name);
                }
            }
            _ => continue,
        }
    }
    (is_oref, curr_class)
}

/// Given a *_keyword string, returns:
/// bool: true if not is before the keyword
/// String: the keyword name
/// Option<String>: the value of the keyword (if one exists)
pub fn get_keyword_and_value(keyword: &str) -> (bool, String, Vec<&str>) {
    let mut not = false;
    let mut keyword_name = "".to_string();
    let mut keyword_value: Vec<&str> = Vec::new();
    // splits string by spaces or equal sign
    let regex = Regex::new(r"[^\s=,()]+").unwrap();
    let mut count = 0;
    let values: Vec<&str> = regex.find_iter(keyword).map(|m| m.as_str()).collect();
    for value in values {
        count += 1;
        let normalized_str = value.to_lowercase();
        if normalized_str == "not" {
            not = true;
        } else if count > 1 && !not {
            keyword_value.push(value);
        } else {
            keyword_name = normalized_str.to_string();
        }
    }
    (not, keyword_name, keyword_value)
}

/// Builds an initial `ScopeTree` skeleton from a parsed `Tree`.
///
/// Creates a new `ScopeTree` rooted at `class_symbol_id`, then walks the syntax tree and adds
/// scopes for nodes considered "scope nodes" (see `cls_is_scope_node`).
pub fn initial_build_scope_tree(
    tree: &Tree,
    class_symbol_id: ClassId,
    content: &str,
    is_rtn: bool,
) -> ScopeTree {
    let mut scope_tree = ScopeTree::new(Some(class_symbol_id));
    let mut scope_stack = vec![scope_tree.root];

    let root = tree.root_node();
    build_scope_skeleton(root, &mut scope_tree, &mut scope_stack, is_rtn, content);

    scope_tree
}

/// Recursively traverses `node` and adds scope entries to `scope_tree`, maintaining a stack of
/// active scope ids in `scope_stack`.
fn build_scope_skeleton(
    node: Node,
    scope_tree: &mut ScopeTree,
    scope_stack: &mut Vec<ScopeId>,
    is_rtn: bool,
    content: &str,
) {
    let is_scope;
    let method_name;
    let in_routine_statements;
    if !is_rtn {
        (is_scope, method_name) = cls_is_scope_node(node, content);
        in_routine_statements = false;
    } else {
        (is_scope, method_name, in_routine_statements) = rtn_is_scope_node(node, content);
    }
    if is_scope {
        let scope_start;
        let scope_end;
        if method_name.is_none() {
            let scope_range = get_scope_range(node);
            (scope_start, scope_end) = (scope_range.start_point, scope_range.end_point)
        } else if !is_rtn {
            scope_start = node.start_position();
            scope_end = node.end_position();
        } else {
            (scope_start, scope_end) = get_routine_scope_node_range(node, content);
        }
        let Some(&parent) = scope_stack.last() else {
            eprintln!(
                "Error: Failed to get Scope Parent from scope stack, aborting build_scope_skeleton"
            );
            return;
        };
        let scope_id = scope_tree.add_scope(
            scope_start,
            scope_end,
            parent,
            method_name,
            in_routine_statements,
        );
        scope_stack.push(scope_id);
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        build_scope_skeleton(child, scope_tree, scope_stack, is_rtn, content);
    }

    if is_scope {
        scope_stack.pop();
    }
}

/// Returns `true` if `pos` lies in the half-open range `[start, end)`.
/// Returns `false` otherwise.
pub fn point_in_range(pos: Point, start: Point, end: Point) -> bool {
    if pos >= start && pos < end {
        return true;
    };
    false
}

pub fn range_within_range(inner: &Range, outer: &Range) -> bool {
    // Tree-sitter end points are exclusive, so equality at the end still means contained.
    inner.start_point >= outer.start_point && inner.end_point <= outer.end_point
}

/// Returns `true` if `node` is treated as a scope boundary in `.cls` parsing.
/// Returns `false` otherwise.
pub fn cls_is_scope_node(node: Node, content: &str) -> (bool, Option<String>) {
    let mut method_name_str = None;
    let is_scope;
    match node.kind() {
        "method" | "classmethod" => {
            if let Some(method_definition) = node.named_child(1)
                && let Some(method_name) = method_definition.named_child(0)
                && method_name.kind() == "method_name"
                && let Some(method_id) = method_name.named_child(0)
            {
                method_name_str = get_string_at_byte_range(content, method_id.byte_range());
            }
            is_scope = true;
        }
        "command_if"
        | "else_block"
        | "elseif_block"
        | "else_block_dotted"
        | "elseif_block_dotted"
        | "command_if_dotted_block"
        | "command_else" => {
            is_scope = true;
        }
        _ => is_scope = false,
    }
    (is_scope, method_name_str)
}

fn get_scope_range(node: Node) -> Range {
    if node.kind() == "command_else"
        || node.kind() == "else_block"
        || node.kind() == "elseif_block"
        || node.kind() == "else_block_dotted"
        || node.kind() == "elseif_block_dotted"
    {
        return node.range();
    }
    let children = get_node_children(node);
    let mut end_point = node.end_position();
    let mut end_byte = node.end_byte();
    for child in children {
        match child.kind() {
            "else_block" | "elseif_block" | "else_block_dotted" | "elseif_block_dotted" => break,
            _ => {
                end_byte = child.end_byte();
                end_point = child.end_position();
            }
        }
    }

    Range {
        start_byte: node.start_byte(),
        start_point: node.start_position(),
        end_byte,
        end_point,
    }
}

/// Determines if a node kind terminates a routine method scope.
pub fn is_rtn_method_end(node_str: &str, compiled_header: bool) -> bool {
    if compiled_header {
        return node_str == "command_quit"
            || node_str == "procedure"
            || node_str == "tag_statement";
    } else {
        return node_str == "command_quit" || node_str == "procedure";
    }
}

/// Given a statement node of a procedure statement, get the range of the method it defines
pub fn get_procedure_info(
    node: &Node,
    content: &str,
) -> Option<(String, Range, Range, MethodType, HashSet<String>)> {
    let Some(statement_type) = node.named_child(0) else {
        eprintln!("Error: Expected Statement node to have child at index 0");
        return None;
    };
    let Some(tag) = statement_type.named_child(0) else {
        eprintln!(
            "Expected procedure node to have a child at index 0, aborting initial_build_procedure"
        );
        return None;
    };
    let Some(method_name) = get_string_at_byte_range(content, tag.byte_range()) else {
        return None;
    };
    let method_name_range = tag.range();
    let procedure_range = statement_type.range();
    let mut is_public = false;
    let procedure_children = get_node_children(statement_type);
    let mut public_variables_declared = HashSet::new();
    for procedure_statement in procedure_children {
        match procedure_statement.kind() {
            "keyword_public" => {
                is_public = true;
            }
            "procedure_pub_vars" => {
                let variables = get_node_children(procedure_statement);
                for var in variables {
                    let Some(var_name) = get_string_at_byte_range(content, var.byte_range()) else {
                        continue;
                    };
                    public_variables_declared.insert(var_name);
                }
            }
            _ => {
                continue;
            }
        }
    }
    return Some((
        method_name,
        method_name_range,
        procedure_range,
        MethodType::Procedure(is_public),
        public_variables_declared,
    ));
}

/// Given a routine_definition node or compiled_header node, get the range of the statements that come before the first tag statement or procedure or quit.
pub fn get_routine_method_range(node: &Node, end_point: Point, end_byte: usize) -> Option<Range> {
    let mut saw_first_tag_statement = false;
    let is_compiled_header = node.kind() == "compiled_header";
    // get statement siblings until one is tag_statement or procedure
    let mut next_sibling = node.next_named_sibling();
    let routine_start_point = node.start_position();
    let routine_start_byte = node.start_byte();
    let mut routine_scope_end_point = end_point;
    let mut routine_scope_end_byte = end_byte;
    while let Some(sib) = next_sibling {
        if sib.kind() == "statement" {
            if let Some(future_statement_type) = sib.named_child(0) {
                if !is_compiled_header || saw_first_tag_statement {
                    if is_rtn_method_end(future_statement_type.kind(), is_compiled_header) {
                        break;
                    }
                } else if future_statement_type.kind() == "tag_statement" {
                    saw_first_tag_statement = true
                }
            }
        }
        routine_scope_end_point = sib.end_position();
        routine_scope_end_byte = sib.end_byte();
        next_sibling = sib.next_named_sibling();
    }
    let routine_range = Range {
        start_byte: routine_start_byte,
        start_point: routine_start_point,
        end_point: routine_scope_end_point,
        end_byte: routine_scope_end_byte,
    };
    return Some(routine_range);
}

/// Given a property node, get the name
pub fn get_property_name(node: &Node, content: &str) -> Option<String> {
    let Some(property_name_node_outer) = node.named_child(1) else {
        eprintln!(
            "Error: expected property node {:?} to have child at node 1",
            node.kind()
        );
        return None;
    };
    let Some(property_name_node) = property_name_node_outer.named_child(0) else {
        eprintln!("Error: expected property name node to have child at node 0");
        return None;
    };

    get_string_at_byte_range(content, property_name_node.byte_range())
}

/// Given a parameter node, get the name
pub fn get_parameter_name(node: &Node, content: &str) -> Option<String> {
    let Some(parameter_name_node_outer) = node.named_child(1) else {
        eprintln!("Error: expected parameter node to have child at node 1");
        return None;
    };
    let Some(parameter_name_node) = parameter_name_node_outer.named_child(0) else {
        eprintln!("Error: expected parameter name node to have child at node 0");
        return None;
    };

    get_string_at_byte_range(content, parameter_name_node.byte_range())
}

/// Given the dotted statement line, find the tag
pub fn get_dotted_subroutine_info(
    node: &Node,
    content: &str,
) -> Option<(String, Range, Range, MethodType)> {
    if let Some(dotted_statement_parent) = node.parent()
        && let Some(tag_node) = node.named_child(0)
        && let Some(method_name) = get_string_at_byte_range(content, tag_node.byte_range())
    {
        if dotted_statement_parent.kind() != "command_do" {
            eprintln!(
                "Error: Expected dotted statement node to have parent of kind command_do, but got {:?}",
                dotted_statement_parent.kind()
            );
            return None;
        }
        return Some((
            method_name,
            tag_node.range(),
            dotted_statement_parent.range(),
            MethodType::DottedSubroutine(true),
        ));
    }
    eprintln!("Error: Expected dotted statement node to have a parent, but it didn't");

    None
}

/// Given a statement node of a tag statement, get the range of the method it defines
pub fn get_subroutine_info(
    node: &Node,
    content: &str,
) -> Option<(String, Range, Range, MethodType)> {
    let mut is_public = true;
    let Some(statement_type) = node.named_child(0) else {
        eprintln!("Error: Expected Statement node to have child at index 0");
        // curr_routine_child = routine_child.next_named_sibling();
        return None;
    };
    if statement_type.kind() != "tag_statement" {
        return None;
    }
    let Some(tag) = statement_type.named_child(0) else {
        eprintln!("Error: expected tag statement node to have child at node 0");
        return None;
    };

    let Some(method_name) = get_string_at_byte_range(content, tag.byte_range()) else {
        return None;
    };

    if let Some(tag_keyword) =
        statement_type.named_child((statement_type.named_child_count() - 1) as u32)
    {
        match tag_keyword.kind() {
            "keyword_methodimpl" => {
                eprintln!("TODO: Verify if there is anything to be done for methodimpl keyword");
            }
            "keyword_private" => {
                is_public = false;
            }
            _ => {}
        }
    }
    // get statement siblings until one is tag_statement or procedure
    let mut next_sibling = node.next_named_sibling();
    let subroutine_start_point = statement_type.start_position();
    let subroutine_start_byte = statement_type.start_byte();
    let mut subroutine_scope_end_point = node.end_position();
    let mut subroutine_scope_end_byte = node.end_byte();
    while let Some(sib) = next_sibling {
        if sib.kind() == "statement" {
            if let Some(future_statement_type) = sib.named_child(0) {
                if is_rtn_method_end(future_statement_type.kind(), false) {
                    break;
                }
            }
        }
        subroutine_scope_end_point = sib.end_position();
        subroutine_scope_end_byte = sib.end_byte();
        next_sibling = sib.next_named_sibling();
    }
    let subroutine_range = Range {
        start_byte: subroutine_start_byte,
        start_point: subroutine_start_point,
        end_point: subroutine_scope_end_point,
        end_byte: subroutine_scope_end_byte,
    };
    return Some((
        method_name,
        tag.range(),
        subroutine_range,
        MethodType::Subroutine(is_public),
    ));
}

/// Determines if a tree-sitter node starts a new subroutine scope in a routine file.
pub fn rtn_is_scope_node(node: Node, content: &str) -> (bool, Option<String>, bool) {
    let mut method_name_str = None;
    let mut is_scope = false;
    match node.kind() {
        "command_if"
        | "else_block"
        | "elseif_block"
        | "else_block_dotted"
        | "elseif_block_dotted"
        | "command_if_dotted_block"
        | "command_else" => return (true, None, false),
        _ => {
            if node.kind() == "tag_statement" {
                let mut sib = node.parent().and_then(|p| p.prev_named_sibling());
                while let Some(sibling) = sib {
                    let Some(command) = sibling.named_child(0) else {
                        eprintln!(
                            "Sibling node {:?} for tag statement {:?} did not have a child at index 0, skipping (rtn_is_scope_node)",
                            sibling, node
                        );
                        sib = sibling.prev_named_sibling();
                        continue;
                    };
                    if command.kind() == "tag_statement" {
                        sib = sibling.prev_named_sibling();
                        while let Some(last_sib) = sib {
                            if last_sib.kind() == "routine_definition"
                                || last_sib.kind() == "compiled_header"
                            {
                                return (true, None, true);
                            }
                            let Some(command) = last_sib.named_child(0) else {
                                eprintln!(
                                    "Sibling node {:?} for tag statement {:?} did not have a child at index 0, skipping (rtn_is_scope_node)",
                                    last_sib, node
                                );
                                sib = last_sib.prev_named_sibling();
                                continue;
                            };
                            if command.kind() == "procedure" || command.kind() == "command_quit" {
                                return (true, None, false);
                            }
                            sib = last_sib.prev_named_sibling();
                        }
                        // No quit or procedure found, this is a routine statement
                        return (true, None, true);
                    } else if command.kind() == "procedure" || command.kind() == "command_quit" {
                        if let Some(tag) = node.named_child(0) {
                            method_name_str = get_string_at_byte_range(content, tag.byte_range());
                        }
                        return (true, method_name_str, false);
                    }
                    sib = sibling.prev_named_sibling();
                }
                // No previous tag or quit found — first subroutine in the file.
                if let Some(tag) = node.named_child(0) {
                    method_name_str = get_string_at_byte_range(content, tag.byte_range());
                }
                return (true, method_name_str, true);
            }
            if node.kind() == "procedure" {
                if let Some(tag) = node.named_child(0) {
                    method_name_str = get_string_at_byte_range(content, tag.byte_range());
                }
                is_scope = true;
            } else if node.kind() == "dotted_statement" {
                if let Some(sibling) = node.prev_named_sibling()
                    && sibling.kind() == "dotted_statement"
                {
                    let Some(dotted_statement_line) =
                        get_string_at_byte_range(content, node.byte_range())
                    else {
                        return (is_scope, method_name_str, false);
                    };
                    let depth = count_leading_dots_in_line(&dotted_statement_line);
                    let Some(sib_dotted_statement_line) =
                        get_string_at_byte_range(content, sibling.byte_range())
                    else {
                        return (is_scope, method_name_str, false);
                    };
                    let sib_depth = count_leading_dots_in_line(&sib_dotted_statement_line);
                    if depth > sib_depth {
                        is_scope = true;
                    }
                } else {
                    is_scope = true;
                }
                if is_scope {
                    let mut curr_parent = node.parent();
                    while let Some(parent) = curr_parent
                        && let Some(_) = parent.parent()
                    {
                        if parent.kind() == "procedure" {
                            if let Some(tag) = parent.named_child(0) {
                                method_name_str =
                                    get_string_at_byte_range(content, tag.byte_range());
                            }
                            break;
                        }
                        curr_parent = parent.parent();
                    }

                    if method_name_str.is_none() {
                        if let Some(parent) = curr_parent {
                            let mut curr_sib = parent.prev_named_sibling();
                            while let Some(sibling) = curr_sib {
                                let Some(command) = sibling.named_child(0) else {
                                    eprintln!(
                                        "Sibling node {:?} did not have a child at index 0, skipping (rtn_is_scope_node)",
                                        sibling.kind()
                                    );
                                    curr_sib = sibling.prev_named_sibling();
                                    continue;
                                };
                                if command.kind() == "tag_statement" {
                                    if let Some(tag) = sibling.named_child(0) {
                                        method_name_str =
                                            get_string_at_byte_range(content, tag.byte_range());
                                    }
                                } else if command.kind() == "command_quit"
                                    || command.kind() == "procedure"
                                {
                                    break;
                                }
                                curr_sib = sibling.prev_named_sibling();
                            }
                        }
                    }
                }
            }

            (is_scope, method_name_str, false)
        }
    }
}

/// Given either a tag_statement, procedure, or dotted_statement node,
/// returns the start and end point of the scope of that node.
pub fn get_routine_scope_node_range(node: Node, content: &str) -> (Point, Point) {
    match node.kind() {
        "procedure" => return (node.start_position(), node.end_position()),
        "tag_statement" => {
            let mut next_sibling = node.parent();
            let subroutine_start_point = node.start_position();
            let mut subroutine_scope_end_point = node.end_position();
            while let Some(sib) = next_sibling {
                if sib.kind() == "statement" {
                    if let Some(future_statement_type) = sib.named_child(0) {
                        if future_statement_type.kind() == "command_quit" {
                            return (subroutine_start_point, future_statement_type.end_position());
                        } else if future_statement_type.kind() == "procedure" {
                            return (subroutine_start_point, subroutine_scope_end_point);
                        }
                    }
                }
                subroutine_scope_end_point = sib.end_position();
                next_sibling = sib.next_named_sibling();
            }
            return (subroutine_start_point, subroutine_scope_end_point);
        }
        "dotted_statement" => {
            let dotted_statement_start_point = node.start_position();
            let mut dotted_statement_scope_end_point = node.end_position();
            let Some(dotted_statement_line) = get_string_at_byte_range(content, node.byte_range())
            else {
                return (
                    dotted_statement_start_point,
                    dotted_statement_scope_end_point,
                );
            };
            let depth = count_leading_dots_in_line(&dotted_statement_line);
            let mut next_sibling = node.next_named_sibling();
            while let Some(sib) = next_sibling {
                if sib.kind() == "dotted_statement" {
                    let Some(line) = get_string_at_byte_range(content, sib.byte_range()) else {
                        next_sibling = sib.next_named_sibling();
                        continue;
                    };
                    let curr_depth = count_leading_dots_in_line(&line);
                    if curr_depth < depth {
                        break;
                    }
                    dotted_statement_scope_end_point = sib.end_position();
                    next_sibling = sib.next_named_sibling();
                }
            }
            return (
                dotted_statement_start_point,
                dotted_statement_scope_end_point,
            );
        }
        _ => {
            eprintln!("Error: {:?} is not a scope node for routines", node.kind());
            return (node.start_position(), node.end_position());
        }
    }
}

/// Given an identifier node, determine if it represents a method name, class name, etc.
/// OPTIONS:
/// [Class, Relationship, Foreignkey, Parameter, Projection,Index,Xdata,Storage,Method, Query, Trigger]
pub fn get_outer_type_from_identifier(node: &Node) -> Option<MemberType> {
    return match node.kind() {
        "parameter_name" => {
            let Some(parameter_name_parent) = node.parent() else {
                eprintln!("Error: expected method_name node to have parent");
                return None;
            };
            return match parameter_name_parent.kind() {
                "oref_parameter" => {
                    let Some(oref_parameter_parent) = parameter_name_parent.parent() else {
                        eprintln!("Error: expected oref_parameter node to have parent");
                        return None;
                    };
                    if oref_parameter_parent.kind() == "relative_dot_parameter" {
                        Some(MemberType::RelativeParameter)
                    } else {
                        Some(MemberType::OrefParameter)
                    }
                }
                "parameter" => Some(MemberType::ParameterDef),
                _ => Some(MemberType::RelativeParameter),
            };
        }
        "projection_name" => Some(MemberType::Projection),
        "class_name" => {
            let Some(class_name_parent) = node.parent() else {
                eprintln!("Error: expected class_name node to have parent");
                return None;
            };
            if class_name_parent.kind() == "class_definition" {
                return Some(MemberType::ClassDef);
            } else {
                return Some(MemberType::Class);
            }
        }
        "query_name" => Some(MemberType::Query),
        "trigger_name" => Some(MemberType::Trigger),
        "property_name" => {
            let Some(property_name_parent) = node.parent() else {
                eprintln!("Error: expected method_name node to have parent");
                return None;
            };
            return match property_name_parent.kind() {
                "oref_property" => {
                    let Some(oref_property_parent) = property_name_parent.parent() else {
                        eprintln!("Error: expected oref_property node to have parent");
                        return None;
                    };
                    if oref_property_parent.kind() == "relative_dot_property" {
                        Some(MemberType::RelativeProperty)
                    } else {
                        Some(MemberType::OrefProperty)
                    }
                }
                "property" => Some(MemberType::PropertyDef),
                _ => Some(MemberType::RelativeProperty),
            };
        }
        "relationship_name" => Some(MemberType::Relationship),
        "foreignkey_name" => Some(MemberType::Foreignkey),
        "index_name" => Some(MemberType::Index),
        "xdata_name" => Some(MemberType::Xdata),
        "storage_name" => Some(MemberType::Storage),
        "method_name" => {
            let Some(method_name_parent) = node.parent() else {
                eprintln!("Error: expected method_name node to have parent");
                return None;
            };
            return match method_name_parent.kind() {
                "oref_method" => {
                    let Some(oref_method_parent) = method_name_parent.parent() else {
                        eprintln!("Error: expected oref_method node to have parent");
                        return None;
                    };
                    if oref_method_parent.kind() == "relative_dot_method" {
                        return Some(MemberType::RelativeMethodCall);
                    } else {
                        return Some(MemberType::OrefMethod);
                    }
                }
                "routine_tag_call" | "print_argument" | "goto_argument" | "extrinsic_function"
                | "line_ref" => Some(MemberType::RoutineMethodCall),
                "class_method_call" | "system_defined_function" => {
                    Some(MemberType::ClassMethodCall)
                }
                "method_definition" => Some(MemberType::MethodDef),
                _ => None,
            };
        }
        "lvn" => {
            let Some(lvn_parent) = node.parent() else {
                eprintln!("Error: expected lvn node to have parent");
                return Some(MemberType::LocalVariable);
            };
            match lvn_parent.kind() {
                "oref_chain_expr" | "class_ref" => Some(MemberType::OrefMethod),
                _ => Some(MemberType::LocalVariable),
            }
        }
        "ssvn" => Some(MemberType::SystemMember),
        "gvn" => Some(MemberType::GlobalVariable),
        "routine_name" => Some(MemberType::Routine),
        _ => None,
    };
}

/// Parses a `line_ref` node into (routine_name, method_name, optional_offset).
pub fn parse_line_ref(
    node: Node,
    content: &str,
    curr_class: String,
) -> (String, String, Option<usize>) {
    let line_ref_children = get_node_children(node);
    let mut method_name = None;
    let mut routine_name = None;
    let mut offset = None;
    for line_ref_child in line_ref_children {
        match line_ref_child.kind() {
            "label_offset" => {
                let Some(offset_expression) = line_ref_child.named_child(0) else {
                    eprintln!(
                        "Error: failed to get label offset for line ref, skipping (get_method_calls)"
                    );
                    continue;
                };
                if let Some(expr_str) =
                    get_string_at_byte_range(content, offset_expression.byte_range())
                {
                    offset = evaluate_expr_from_node(&expr_str);
                }
            }
            "objectscript_identifier" | "objectscript_identifier_special" => {
                // this should be the subroutine name
                method_name = get_string_at_byte_range(content, line_ref_child.byte_range());
            }
            "method_name" => {
                if let Some(identifier) = line_ref_child.named_child(0) {
                    method_name = get_string_at_byte_range(content, identifier.byte_range());
                } else {
                    method_name = get_string_at_byte_range(content, line_ref_child.byte_range());
                }
            }
            "routine_ref" => {
                if let Some(routine_name_node) =
                    line_ref_child.named_child((line_ref_child.named_child_count() - 1) as u32)
                    && routine_name_node.kind() == "routine_name"
                {
                    routine_name =
                        get_string_at_byte_range(content, routine_name_node.byte_range());
                }
            }
            _ => continue,
        }
    }
    let final_routine_name;
    let final_method_name;
    if let Some(routine_name) = routine_name {
        final_routine_name = routine_name;
    } else {
        final_routine_name = curr_class.to_string();
    }
    if let Some(method_name) = method_name {
        final_method_name = method_name;
    } else {
        final_method_name = final_routine_name.clone();
    }
    (final_routine_name, final_method_name, offset)
}

/// Given Method_arg node, find if there is something that could be equivalent to an identifier and return that as a String if so
pub fn get_identifier_from_method_arg(node: Node, content: &str) -> Option<String> {
    if let Some(expression) = node.named_child(0)
        && expression.kind() == "expression"
    {
        let Some(name_node) = expression.named_child(0) else {
            eprintln!(
                "Error: This node can't be an expression, it doesn't have any children {:?}",
                expression.kind()
            );
            return None;
        };

        match name_node.kind() {
            "lvn" | "oref_chain_expr" | "string_literal" => {
                let Some(name) = get_string_at_byte_range(content, node.byte_range()) else {
                    return None;
                };
                return Some(name.trim_matches('"').to_string());
            }

            _ => {
                eprintln!(
                    "Error: name was type {:?}. Update get_identifier_from_method_arg",
                    node.kind()
                );
                return None;
            }
        }
    }
    return None;
}

/// Logs a formatted "aborting function" debug message for early-exit diagnostics.
pub fn generic_exit_statements(struct_name: &str, function_name: &str) {
    eprintln!(
        "Aborting function early. Leaving {:?} function: {:?}",
        struct_name, function_name
    );
    eprintln!("------------------------");
    eprintln!();
}

/// Logs a formatted "skipping" debug message when a function bypasses a struct.
pub fn generic_skipping_statements(function_name: &str, struct_name: &str, struct_type: &str) {
    eprintln!(
        "Skipping applying the logic from {function_name} to {struct_type} named {struct_name}"
    );
    eprintln!("------------------------");
    eprintln!();
}

/// Collects all ERROR and MISSING nodes from a tree-sitter parse tree.
pub fn collect_error_nodes<'tree>(root: Node<'tree>) -> Vec<Node<'tree>> {
    let mut out = Vec::new();
    let mut cursor = root.walk();
    visit_errors(root, &mut cursor, &mut out);
    out
}

/// Extracts byte ranges of ObjectScript code embedded in XML `<Implementation>` CDATA sections.
pub fn xml_objectscript_implementation_ranges(root: Node, content: &str) -> Vec<Range> {
    let query = xml_objectscript_injections_query();
    let capture_names = query.capture_names();
    let mut cursor = QueryCursor::new();
    let mut ranges = Vec::new();
    let mut matches = cursor.matches(query, root, content.as_bytes());

    while let Some(query_match) = matches.next() {
        for capture in query_match.captures {
            let Some(name) = capture_names.get(capture.index as usize) else {
                continue;
            };
            if *name != "injection.content" {
                continue;
            }
            push_non_empty_xml_content_range(capture.node, content, &mut ranges);
        }
    }

    ranges.sort_by_key(|range| (range.start_byte, range.end_byte));
    ranges.dedup_by_key(|range| (range.start_byte, range.end_byte));
    ranges
}

fn push_non_empty_xml_content_range(node: Node, content: &str, out: &mut Vec<Range>) {
    let Some(text) = get_string_at_byte_range(content, node.byte_range()) else {
        return;
    };
    if !text.trim().is_empty() {
        out.push(node.range());
    }
}

fn visit_errors<'tree>(
    node: Node<'tree>,
    cursor: &mut TreeCursor<'tree>,
    out: &mut Vec<Node<'tree>>,
) {
    if !node.has_error() && !node.is_error() && !node.is_missing() {
        return;
    }

    if node.is_error() || node.is_missing() {
        out.push(node);
    }

    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            visit_errors(child, cursor, out);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
        cursor.goto_parent();
    }
}
/// Returns "\r\n" or "\n" based on which line ending the content uses.
pub fn detect_newline(content: &str) -> &'static str {
    if content.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

/// Extracts and trims statement text from byte ranges, skipping empty lines.
pub fn normalized_statement_lines(
    content: &str,
    statement_ranges: &[std::ops::Range<usize>],
) -> Option<Vec<String>> {
    let mut statements = Vec::new();
    for range in statement_ranges {
        let statement = get_string_at_byte_range(content, range.clone())?;
        let trimmed = statement.trim();
        if !trimmed.is_empty() {
            statements.push(trimmed.to_string());
        }
    }
    Some(statements)
}

/// Joins statement strings with a given indent prefix and newline separator.
pub fn indent_statement_lines(statements: &[String], indent: &str, newline: &str) -> String {
    statements
        .iter()
        .map(|statement| indent_statement(statement.as_str(), indent, newline))
        .collect::<Vec<_>>()
        .join(newline)
}

fn indent_statement(statement: &str, indent: &str, newline: &str) -> String {
    let lines = statement.split(newline).collect::<Vec<_>>();
    let nested_base_indent = lines
        .iter()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .map(|line| leading_whitespace_len(line))
        .min()
        .unwrap_or(0);

    lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            let normalized = if index == 0 || nested_base_indent == 0 {
                *line
            } else {
                line.get(nested_base_indent..).unwrap_or("")
            };
            format!("{indent}{normalized}")
        })
        .collect::<Vec<_>>()
        .join(newline)
}

fn leading_whitespace_len(line: &str) -> usize {
    line.char_indices()
        .find(|(_, ch)| !ch.is_whitespace())
        .map(|(index, _)| index)
        .unwrap_or(line.len())
}

/// Returns the leading whitespace string on the line containing `byte_index`.
pub fn line_indent_before(content: &str, byte_index: usize) -> String {
    let line_start = content
        .get(..byte_index)
        .and_then(|prefix| prefix.rfind('\n').map(|index| index + 1))
        .unwrap_or(0);
    let line_prefix = content.get(line_start..byte_index).unwrap_or("");
    let indent_end = line_prefix
        .char_indices()
        .find(|(_, ch)| !ch.is_whitespace())
        .map(|(index, _)| index)
        .unwrap_or(line_prefix.len());
    line_prefix[..indent_end].to_string()
}
