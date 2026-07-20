use crate::common::{
    advance_point, detect_newline, get_node_children, get_string_at_byte_range,
    indent_statement_lines, line_indent_before, normalized_statement_lines,
};
use crate::parse_structures::{FileType, OldStatement};
use std::ops::Range;
use tree_sitter::{InputEdit, Language, Node, Parser, Query, QueryCursor, StreamingIterator};
use tree_sitter_objectscript::LANGUAGE_OBJECTSCRIPT_UDL;
use tree_sitter_objectscript_routine::LANGUAGE_OBJECTSCRIPT_ROUTINE;
fn update_tree_and_content(
    tree: &mut tree_sitter::Tree,
    content: &mut String,
    old_range: tree_sitter::Range,
    replacement: &str,
) {
    let (start_byte, start_point, old_end_byte, old_end_point) = (
        old_range.start_byte,
        old_range.start_point,
        old_range.end_byte,
        old_range.end_point,
    );
    let new_end_byte = start_byte + replacement.len();
    let new_end_position = advance_point(start_point.row, start_point.column, replacement);
    let input_edit = InputEdit {
        start_byte,
        old_end_byte,
        new_end_byte,
        start_position: start_point,
        old_end_position: old_end_point,
        new_end_position,
    };
    content.replace_range(start_byte..old_end_byte, replacement);
    tree.edit(&input_edit);
}

/// Creates a tree-sitter Parser configured for the given language grammar.
pub fn create_parser(language: &Language) -> Option<Parser> {
    let mut parser = Parser::new();
    parser.set_language(&language).ok()?;
    Some(parser)
}

fn remove_unreachable_statements(
    content: &mut String,
    tree: &mut tree_sitter::Tree,
    language: &Language,
    query_str: &str,
    parser: &mut Parser,
) {
    if let Ok(query) = Query::new(language, query_str) {
        let root = tree.root_node();
        let mut cursor = QueryCursor::new();
        let mut iter = cursor.matches(&query, root, content.as_bytes());
        let mut ranges = Vec::new();
        while let Some(m) = iter.next() {
            if m.captures.len() == 1 {
                ranges.push(m.captures[0].node.range());
            }
        }
        ranges.sort_by_key(|range| std::cmp::Reverse(range.start_byte));

        for range in ranges {
            update_tree_and_content(tree, content, range, "");
            let new_tree = parser.parse(content.as_str(), Some(&*tree)).unwrap();
            *tree = new_tree;
        }
    };
}

fn add_comment_to_string(
    statement_struct: &OldStatement,
    updated_string: &mut String,
    replacement_string: &mut String,
    after: bool,
) {
    let comment_range;
    if after {
        comment_range = statement_struct.comment_after_last_statement_range;
    } else {
        comment_range = statement_struct.comment_range;
    }
    if let Some(comment_range) = comment_range {
        let Some(comment) = get_string_at_byte_range(
            updated_string,
            Range {
                start: comment_range.start_byte,
                end: comment_range.end_byte,
            },
        ) else {
            eprintln!("Failed to get string of comment for statement");
            return;
        };
        replacement_string.push_str(comment.as_str());
    }
}

// in classes, for each method, store the conditionals
// in routines, for each file, store the conditionals
fn remove_unreachable_conditionals(
    content: &str,
    language: &Language,
    parser: &mut Parser,
) -> Option<(tree_sitter::Tree, String)> {
    // first remove if and else statements that are pointless (if statements with no expression and no statement)
    // first, refactor the if-else statements
    let mut updated_string = content.to_string();
    let Some(mut tree) = parser.parse(content, None) else {
        eprint!("Failed to parse tree");
        return None;
    };
    let query_str = "(command_if (keyword_old_if) (expression)? @condition (statement)? @statement) @command_if";
    remove_unreachable_statements(&mut updated_string, &mut tree, language, query_str, parser);

    let query_str = "(command_else (keyword_oldelse) (statement)? @statement) @command";
    remove_unreachable_statements(&mut updated_string, &mut tree, language, query_str, parser);

    // first remove unreachable if statements
    Some((tree, updated_string))
}

fn remove_unreachable_for_statements(
    content: &str,
    language: &Language,
    parser: &mut Parser,
) -> Option<(tree_sitter::Tree, String)> {
    let mut updated_string = content.to_string();
    let Some(mut tree) = parser.parse(content, None) else {
        eprint!("Failed to parse tree");
        return None;
    };
    let query_str =
        "(command_for (keyword_for) (for_parameter)? @param (statement)? @statement ) @command";
    remove_unreachable_statements(&mut updated_string, &mut tree, language, query_str, parser);

    let query_str =
        "(command_for (keyword_old_for) (for_parameter)? @param (statement)? @statement ) @command";
    remove_unreachable_statements(&mut updated_string, &mut tree, language, query_str, parser);
    Some((tree, updated_string))
}

fn refactor_if_else_statements(
    tree: &mut tree_sitter::Tree,
    updated_string: &mut String,
    language: &Language,
    query_str: &str,
) -> bool {
    if let Ok(query) = Query::new(language, query_str) {
        let root = tree.root_node();
        let mut cursor = QueryCursor::new();
        let mut iter = cursor.matches(&query, root, updated_string.as_bytes());
        let Some(query_match) = iter.next() else {
            return false;
        };
        let if_statement = query_match.captures[0].node;
        let else_statement = query_match.captures[1].node;
        let Some(if_statement_struct) = build_old_statement_struct(&if_statement, &updated_string)
        else {
            eprintln!("Failed to build if_statement_struct");
            return false;
        };
        let Some(else_statement_struct) =
            build_old_statement_struct(&else_statement, &updated_string)
        else {
            eprintln!("Failed to build if_statement_struct");
            return false;
        };
        let newline = detect_newline(updated_string);
        let start_byte;
        let start_point;
        let (end_byte, end_point, else_has_comment, else_has_comment_after_last_statement) =
            check_statement_fields(&else_statement_struct);
        let if_has_comment = if_statement_struct.comment_range.is_some();
        let if_has_comment_after_last_statement = if_statement_struct
            .comment_after_last_statement_range
            .is_some();
        let mut replacement_string: String = String::new();
        let Some(else_statements) = normalized_statement_lines(
            updated_string,
            else_statement_struct.statement_ranges.as_slice(),
        ) else {
            return false;
        };
        if if_statement_struct.last_expression_end_byte.is_none() {
            start_byte = if_statement_struct.keyword_old_range.end_byte;
            start_point = if_statement_struct.keyword_old_range.end_point;
            // we know there are statements in this case, because
            // otherwise it would have been handled by remove_unreachable_statements
            replacement_string = String::from(" $TEST");
        } else {
            if if_statement_struct.statement_ranges.is_empty() {
                start_byte = if_statement_struct.command_range.start_byte;
                start_point = if_statement_struct.command_range.start_point;
                let base_indent = line_indent_before(updated_string, start_byte);
                let Some(expression) = get_string_at_byte_range(
                    updated_string,
                    Range {
                        start: if_statement_struct.keyword_old_range.end_byte + 1,
                        end: if_statement_struct.last_expression_end_byte.unwrap(),
                    },
                ) else {
                    eprintln!("Failed to get string of expression for if statement");
                    return false;
                };
                replacement_string = format!("{base_indent}if '({expression})");
                if else_has_comment {
                    add_comment_to_string(
                        &else_statement_struct,
                        updated_string,
                        &mut replacement_string,
                        false,
                    );
                }
                replacement_string.push_str(
                    build_replacement_string_block(
                        base_indent.as_str(),
                        newline,
                        else_statements.as_slice(),
                    )
                    .as_str(),
                );

                if else_has_comment_after_last_statement {
                    add_comment_to_string(
                        &else_statement_struct,
                        updated_string,
                        &mut replacement_string,
                        true,
                    );
                }
                let old_text = &updated_string[start_byte..end_byte];
                if old_text == replacement_string {
                    return false;
                }
                let range = tree_sitter::Range {
                    start_byte,
                    end_byte,
                    start_point,
                    end_point,
                };
                update_tree_and_content(tree, updated_string, range, replacement_string.as_str());
                return true;
            } else {
                start_byte = if_statement_struct.last_expression_end_byte.unwrap();
                start_point = if_statement_struct.last_expression_end_point.unwrap();
            }
        }
        let base_indent = line_indent_before(updated_string, start_byte);
        let Some(if_statements) = normalized_statement_lines(
            updated_string,
            if_statement_struct.statement_ranges.as_slice(),
        ) else {
            eprintln!("Failed to normalize if statement ranges");
            return false;
        };
        if if_has_comment {
            add_comment_to_string(
                &if_statement_struct,
                updated_string,
                &mut replacement_string,
                false,
            );
        }
        replacement_string.push_str(
            build_replacement_string_block(base_indent.as_str(), newline, if_statements.as_slice())
                .as_str(),
        );
        if if_has_comment_after_last_statement {
            add_comment_to_string(
                &if_statement_struct,
                updated_string,
                &mut replacement_string,
                true,
            );
        }
        replacement_string.push_str(format!("{base_indent}else").as_str());
        if else_has_comment {
            add_comment_to_string(
                &else_statement_struct,
                updated_string,
                &mut replacement_string,
                false,
            );
        }
        replacement_string.push_str(
            build_replacement_string_block(
                base_indent.as_str(),
                newline,
                else_statements.as_slice(),
            )
            .as_str(),
        );

        if else_has_comment_after_last_statement {
            add_comment_to_string(
                &else_statement_struct,
                updated_string,
                &mut replacement_string,
                true,
            );
        }

        let old_text = &updated_string[start_byte..end_byte];
        if old_text == replacement_string {
            return false;
        }
        let range = tree_sitter::Range {
            start_byte,
            end_byte,
            start_point,
            end_point,
        };
        update_tree_and_content(tree, updated_string, range, replacement_string.as_str());
        return true;
    } else {
        return false;
    }
}

fn refactor_old_for_statements(
    tree: &mut tree_sitter::Tree,
    updated_string: &mut String,
    language: &Language,
    query_str: &str,
) -> bool {
    if let Ok(query) = Query::new(language, query_str) {
        let root = tree.root_node();
        let mut cursor = QueryCursor::new();
        let mut iter = cursor.matches(&query, root, updated_string.as_bytes());
        let Some(query_match) = iter.next() else {
            // everything has been refactored
            return false;
        };
        let for_statement = query_match.captures[0].node;
        let Some(statement_struct) = build_old_statement_struct(&for_statement, &updated_string)
        else {
            eprintln!("Failed to build for statement struct");
            return false;
        };
        let newline = detect_newline(updated_string);
        let start_byte;
        let start_point;
        let (end_byte, end_point, has_comment, has_comment_after_last_statement) =
            check_statement_fields(&statement_struct);
        let mut replacement_string: String = String::new();
        if statement_struct.last_expression_end_byte.is_none() {
            start_byte = statement_struct.keyword_old_range.end_byte;
            start_point = statement_struct.keyword_old_range.end_point;
        } else {
            start_byte = statement_struct.last_expression_end_byte.unwrap();
            start_point = statement_struct.last_expression_end_point.unwrap();
        }
        let base_indent = line_indent_before(updated_string, start_byte);
        let Some(if_statements) = normalized_statement_lines(
            updated_string,
            statement_struct.statement_ranges.as_slice(),
        ) else {
            eprintln!("Failed to normalize if statement ranges");
            return false;
        };
        if has_comment {
            add_comment_to_string(
                &statement_struct,
                updated_string,
                &mut replacement_string,
                false,
            );
        }
        replacement_string.push_str(
            build_replacement_string_block(base_indent.as_str(), newline, if_statements.as_slice())
                .as_str(),
        );

        if has_comment_after_last_statement {
            add_comment_to_string(
                &statement_struct,
                updated_string,
                &mut replacement_string,
                true,
            );
        }

        let old_text = &updated_string[start_byte..end_byte];
        if old_text == replacement_string {
            return false;
        }
        let range = tree_sitter::Range {
            start_byte,
            end_byte,
            start_point,
            end_point,
        };
        update_tree_and_content(tree, updated_string, range, replacement_string.as_str());
        return true;
    } else {
        return false;
    }
}

fn refactor_old_if_statements(
    tree: &mut tree_sitter::Tree,
    updated_string: &mut String,
    language: &Language,
    query_str: &str,
) -> bool {
    if let Ok(query) = Query::new(language, query_str) {
        let root = tree.root_node();
        let mut cursor = QueryCursor::new();
        let mut iter = cursor.matches(&query, root, updated_string.as_bytes());
        let Some(query_match) = iter.next() else {
            // everything has been refactored
            return false;
        };
        let if_statement = query_match.captures[0].node;
        let Some(statement_struct) = build_old_statement_struct(&if_statement, &updated_string)
        else {
            eprintln!("Failed to build if_statement_struct");
            return false;
        };
        let newline = detect_newline(updated_string);
        let start_byte;
        let start_point;
        let (end_byte, end_point, has_comment, has_comment_after_last_statement) =
            check_statement_fields(&statement_struct);
        let mut replacement_string: String = String::new();
        if statement_struct.last_expression_end_byte.is_none() {
            start_byte = statement_struct.keyword_old_range.end_byte;
            start_point = statement_struct.keyword_old_range.end_point;
            // we know there are statements in this case, because
            // otherwise it would have been handled by remove_unreachable_statements
            replacement_string = String::from(" $TEST");
        } else {
            if statement_struct.statement_ranges.is_empty() {
                let range = statement_struct.command_range;
                update_tree_and_content(tree, updated_string, range, "");
                return true;
            } else {
                start_byte = statement_struct.last_expression_end_byte.unwrap();
                start_point = statement_struct.last_expression_end_point.unwrap();
            }
        }
        let base_indent = line_indent_before(updated_string, start_byte);
        let Some(if_statements) = normalized_statement_lines(
            updated_string,
            statement_struct.statement_ranges.as_slice(),
        ) else {
            eprintln!("Failed to normalize if statement ranges");
            return false;
        };
        if has_comment {
            add_comment_to_string(
                &statement_struct,
                updated_string,
                &mut replacement_string,
                false,
            );
        }
        replacement_string.push_str(
            build_replacement_string_block(base_indent.as_str(), newline, if_statements.as_slice())
                .as_str(),
        );

        if has_comment_after_last_statement {
            add_comment_to_string(
                &statement_struct,
                updated_string,
                &mut replacement_string,
                true,
            );
        }

        let old_text = &updated_string[start_byte..end_byte];
        if old_text == replacement_string {
            return false;
        }
        let range = tree_sitter::Range {
            start_byte,
            end_byte,
            start_point,
            end_point,
        };
        update_tree_and_content(tree, updated_string, range, replacement_string.as_str());
        return true;
    } else {
        return false;
    }
}

/// Extracts end byte/point and comment presence flags from an OldStatement.
pub fn check_statement_fields(
    statement_struct: &OldStatement,
) -> (usize, tree_sitter::Point, bool, bool) {
    let end_byte = statement_struct.command_range.end_byte;
    let end_point = statement_struct.command_range.end_point;
    let has_comment = statement_struct.comment_range.is_some();
    let has_comment_after_last_statement = statement_struct
        .comment_after_last_statement_range
        .is_some();
    (
        end_byte,
        end_point,
        has_comment,
        has_comment_after_last_statement,
    )
}

fn refactor_old_else_statements(
    tree: &mut tree_sitter::Tree,
    updated_string: &mut String,
    language: &Language,
    query_str: &str,
) -> bool {
    if let Ok(query) = Query::new(language, query_str) {
        let root = tree.root_node();
        let mut cursor = QueryCursor::new();
        let mut iter = cursor.matches(&query, root, updated_string.as_bytes());
        let Some(query_match) = iter.next() else {
            // everything has been refactored
            return false;
        };
        let else_statement = query_match.captures[0].node;
        let Some(statement_struct) = build_old_statement_struct(&else_statement, &updated_string)
        else {
            eprintln!("Failed to build else_statement_struct");
            return false;
        };
        let (end_byte, end_point, has_comment, has_comment_after_last_statement) =
            check_statement_fields(&statement_struct);
        let newline = detect_newline(updated_string);
        let start_byte = statement_struct.keyword_old_range.start_byte;
        let start_point = statement_struct.keyword_old_range.start_point;
        let base_indent = line_indent_before(updated_string, start_byte);
        let mut replacement_string = String::from(format!("{base_indent}if $TEST = 0"));
        let Some(statements) = normalized_statement_lines(
            updated_string,
            statement_struct.statement_ranges.as_slice(),
        ) else {
            eprintln!("Failed to normalize if statement ranges");
            return false;
        };
        if has_comment {
            add_comment_to_string(
                &statement_struct,
                updated_string,
                &mut replacement_string,
                false,
            );
        }
        replacement_string.push_str(
            build_replacement_string_block(base_indent.as_str(), newline, statements.as_slice())
                .as_str(),
        );

        if has_comment_after_last_statement {
            add_comment_to_string(
                &statement_struct,
                updated_string,
                &mut replacement_string,
                true,
            );
        }
        let old_text = &updated_string[start_byte..end_byte];
        if old_text == replacement_string {
            return false;
        }
        let range = tree_sitter::Range {
            start_byte,
            end_byte,
            start_point,
            end_point,
        };
        update_tree_and_content(tree, updated_string, range, replacement_string.as_str());
        return true;
    } else {
        return false;
    }
}

/// Parses a legacy command node into an OldStatement capturing its keyword, expressions, statements, and comments.
pub fn build_old_statement_struct(node: &Node, content: &str) -> Option<OldStatement> {
    let children = get_node_children(node.clone());
    let mut statement_ranges = Vec::new();
    let mut expression_end_byte = None;
    let mut expression_end_point = None;
    let mut comment_range = None;
    let command_range = node.range();
    let mut keyword_range: Option<tree_sitter::Range> = None;
    let mut saw_statement_comment = false;
    let mut comment_after_statement_range = None;
    // in the case where there's a comment right after,
    // we want to store the statement before the comment
    let mut last_statement_range_stored = None;
    let mut statements_after_do = Vec::new();
    let depth = find_do_statement_depth(node);

    for child in children {
        match child.kind() {
            "for_parameter" => {
                expression_end_byte = Some(child.end_byte());
                expression_end_point = Some(child.range().end_point);
            }
            "expression" => {
                expression_end_byte = Some(child.end_byte());
                expression_end_point = Some(child.range().end_point);
            }
            "do_statement_after" => {
                let range = std::ops::Range {
                    start: child.start_byte(),
                    end: child.end_byte(),
                };
                statements_after_do.push(range);
            }
            "statement" => {
                if saw_statement_comment {
                    saw_statement_comment = false;
                    last_statement_range_stored = None;
                    comment_after_statement_range = None;
                }
                let range = std::ops::Range {
                    start: child.start_byte(),
                    end: child.end_byte(),
                };
                statement_ranges.push(range);
            }
            "dotted_statement" => {
                if saw_statement_comment {
                    saw_statement_comment = false;
                    last_statement_range_stored = None;
                    comment_after_statement_range = None;
                }
                let Some(line) = get_string_at_byte_range(content, child.byte_range()) else {
                    continue;
                };
                let dot_count = count_leading_dots_in_line(&line);
                if dot_count >= depth {
                    let range = std::ops::Range {
                        start: child.start_byte(),
                        end: child.end_byte(),
                    };
                    statement_ranges.push(range);
                }
            }
            "argumentless_inline_comment" => {
                comment_range = Some(child.range());
            }
            "inline_comment" => {
                if !statements_after_do.is_empty() && statement_ranges.is_empty() {
                    // this is between statements, so it should be after the last statement end
                    let Some(last_statement_range) = statements_after_do.pop() else {
                        eprintln!("Failed to get last statement range");
                        continue;
                    };
                    let new_range = Range {
                        start: last_statement_range.start,
                        end: child.end_byte(),
                    };
                    statements_after_do.push(new_range);
                    continue;
                }
                if !statement_ranges.is_empty() {
                    saw_statement_comment = true;
                    // this is between statements, so it should be after the last statement end
                    let Some(last_statement_range) = statement_ranges.pop() else {
                        eprintln!("Failed to get last statement range");
                        continue;
                    };
                    last_statement_range_stored = Some(last_statement_range.clone());
                    comment_after_statement_range = Some(child.range());
                    let new_range = Range {
                        start: last_statement_range.start,
                        end: child.end_byte(),
                    };
                    statement_ranges.push(new_range);
                    continue;
                } else {
                    // this is between the expr and the start of the statements
                    // or in the case of for statements it can be
                    // between the keyword and expr or the expr and statements
                    if node.kind() == "command_for" && expression_end_byte.is_none() {
                        let new_keyword_range_end_byte = child.end_byte();
                        let new_keyword_range_end_point = child.end_position();
                        let Some(original_range) = keyword_range else {
                            eprintln!("Error: keyword range was none");
                            continue;
                        };
                        let new_range = tree_sitter::Range {
                            start_byte: original_range.start_byte,
                            start_point: original_range.start_point,
                            end_byte: new_keyword_range_end_byte,
                            end_point: new_keyword_range_end_point,
                        };
                        keyword_range = Some(new_range);
                        continue;
                    }
                    comment_range = Some(child.range());
                }
            }
            kind if kind.starts_with("keyword_") => {
                keyword_range = Some(child.range());
            }
            _ => {
                continue;
            }
        }
    }
    let Some(keyword_old_range) = keyword_range else {
        eprintln!("Error: keyword_old_range is None");
        return None;
    };

    if saw_statement_comment {
        statement_ranges.pop();
        let Some(new_range) = last_statement_range_stored else {
            eprintln!("Last statement range was never stored");
            return None;
        };
        statement_ranges.push(new_range.clone());
    }

    Some(OldStatement {
        last_expression_end_byte: expression_end_byte,
        last_expression_end_point: expression_end_point,
        statement_ranges,
        keyword_old_range,
        command_range,
        comment_range,
        comment_after_last_statement_range: comment_after_statement_range,
        statements_after: statements_after_do,
    })
}

fn refactor_old_conditional_command(
    tree: &mut tree_sitter::Tree,
    updated_string: &mut String,
    language: &Language,
    parser: &mut Parser,
) {
    // first refactor if-else statements
    // RULES:
    // 1. If the if statement is useless (has no other statements attached to it),
    //    then, the if-else will be replaced by a single if statement that is
    //    if '(expression)
    //
    // 2. argumentless if -> if $TEST
    // 3. Both the if and else statements will be converted to their block form
    // 4. Comments will be preserved.
    let query_str = r#"(source_file
  (statement (command_if (keyword_old_if)) @command_if)
  .
  (statement (command_else) @command_else)
)"#;

    loop {
        let changed = refactor_if_else_statements(tree, updated_string, language, query_str);
        let new_tree = parser.parse(updated_string.as_str(), Some(&*tree)).unwrap();
        *tree = new_tree;

        if !changed {
            break;
        }
    }
    eprintln!("Finished if-else refactoring");

    let query_str = "(command_if (keyword_old_if)) @command";
    loop {
        let changed = refactor_old_if_statements(tree, updated_string, language, query_str);
        let new_tree = parser.parse(updated_string.as_str(), Some(&*tree)).unwrap();
        *tree = new_tree;

        if !changed {
            break;
        }
    }
    eprintln!("Finished old if refactoring");
    let query_str = "(command_else (keyword_oldelse)) @command_else";
    loop {
        let changed = refactor_old_else_statements(tree, updated_string, language, query_str);
        let new_tree = parser.parse(updated_string.as_str(), Some(&*tree)).unwrap();
        *tree = new_tree;

        if !changed {
            break;
        }
    }
    eprintln!("Finished old else refactoring");
}

fn build_replacement_string_block(
    base_indent: &str,
    newline: &str,
    statements: &[String],
) -> String {
    let statement_indent = format!("{base_indent}   ");
    let mut new_str = String::new();
    new_str.push_str(format!(" {{{newline}").as_str());
    new_str
        .push_str(indent_statement_lines(statements, statement_indent.as_str(), newline).as_str());
    new_str.push_str(format!("{newline}{base_indent}}}").as_str());
    new_str
}

fn refactor_legacy_for_statements(
    tree: &mut tree_sitter::Tree,
    updated_string: &mut String,
    language: &Language,
    parser: &mut Parser,
) {
    // turn into block version
    let query_str = "(command_for (keyword_old_for)) @command";
    loop {
        let changed = refactor_old_for_statements(tree, updated_string, language, query_str);
        let new_tree = parser.parse(updated_string.as_str(), Some(&*tree)).unwrap();
        *tree = new_tree;

        if !changed {
            break;
        }
    }
    eprintln!("Finished refactoring legacy for statements");
}

/// Refactors legacy `for` commands in ObjectScript source to block form.
pub fn refactor_for_statements(content: &str, file_type: FileType) -> String {
    let language: Language;
    if file_type == FileType::Cls {
        language = LANGUAGE_OBJECTSCRIPT_UDL.into();
    } else {
        language = LANGUAGE_OBJECTSCRIPT_ROUTINE.into()
    }
    let Some(mut parser) = create_parser(&language) else {
        eprintln!("Error: Failed to create parser");
        return content.to_string();
    };

    let Some((mut updated_tree, mut updated_string)) =
        remove_unreachable_for_statements(content, &language, &mut parser)
    else {
        eprintln!("Failed to get tree and string from remove_unreachable_conditionals");
        return content.to_string();
    };
    refactor_legacy_for_statements(
        &mut updated_tree,
        &mut updated_string,
        &language,
        &mut parser,
    );
    updated_string
}

/// Refactors legacy `if`/`else` commands in ObjectScript source to block form.
pub fn refactor_conditionals(content: &str, file_type: FileType) -> String {
    let language: Language;
    if file_type == FileType::Cls {
        language = LANGUAGE_OBJECTSCRIPT_UDL.into();
    } else {
        language = LANGUAGE_OBJECTSCRIPT_ROUTINE.into()
    }
    let Some(mut parser) = create_parser(&language) else {
        eprintln!("Error: Failed to create parser");
        return content.to_string();
    };
    // first remove if and else statements that are pointless (if statements with no expression and no statement)
    let Some((mut updated_tree, mut updated_string)) =
        remove_unreachable_conditionals(content, &language, &mut parser)
    else {
        eprintln!("Failed to get tree and string from remove_unreachable_conditionals");
        return content.to_string();
    };
    // then, refactor the legacy if-else statements, if statements, and else statements
    refactor_old_conditional_command(
        &mut updated_tree,
        &mut updated_string,
        &language,
        &mut parser,
    );
    updated_string
}

fn routine_members(node: &Node, content: &str) -> Vec<String> {
    let mut names = Vec::new();
    let children = get_node_children(node.clone());
    for child in children {
        match child.kind() {
            "routine_definition" => {
                let Some(routine_name) = child.named_child(1) else {
                    eprintln!("Error: couldn't get routine_definition child");
                    continue;
                };
                let Some(name) = get_string_at_byte_range(content, routine_name.byte_range())
                else {
                    eprintln!("Couldn't get routine name");
                    continue;
                };
                names.push(name.to_string());
            }
            "statement" => {
                let Some(command) = child.named_child(0) else {
                    eprintln!("Error: couldn't get statement child");
                    return names;
                };
                match command.kind() {
                    "tag_statement" | "procedure" => {
                        let Some(tag) = command.named_child(0) else {
                            eprintln!("Error: Couldn't get tag from tag statement/procedure");
                            return names;
                        };
                        let Some(tag_name) = get_string_at_byte_range(content, tag.byte_range())
                        else {
                            eprintln!("Couldn't get tag name");
                            return names;
                        };
                        names.push(tag_name);
                    }
                    _ => continue,
                }
            }
            "dotted_statement" => {
                let Some(command) = child.named_child(0) else {
                    eprintln!("Error: couldn't get statement child");
                    return names;
                };
                if command.kind() == "tag" {
                    let Some(tag_name) = get_string_at_byte_range(content, command.byte_range())
                    else {
                        eprintln!("Couldn't get tag name");
                        return names;
                    };
                    names.push(tag_name);
                }
            }
            _ => continue,
        }
    }
    names
}

fn generated_subroutine_base_name(subroutine_name: &str) -> &str {
    let Some((base_name, suffix)) = subroutine_name.rsplit_once("Subroutine") else {
        return subroutine_name;
    };
    if !base_name.is_empty() && suffix.chars().all(|ch| ch.is_ascii_digit()) {
        base_name
    } else {
        subroutine_name
    }
}

fn generate_subroutine_name(
    subroutine_name: &str,
    mut dot_depth: usize,
    routine_members: &mut Vec<String>,
) -> String {
    let subroutine_name = generated_subroutine_base_name(subroutine_name);
    loop {
        let candidate = format!("{subroutine_name}Subroutine{dot_depth}");

        if !routine_members.contains(&candidate) {
            routine_members.push(candidate.clone());
            return candidate;
        }
        dot_depth += 1;
    }
}

fn line_starts_routine_member(line: &str) -> bool {
    if line.is_empty() || line.starts_with(' ') || line.starts_with('\t') {
        return false;
    }

    let Some(token) = line.split_whitespace().next() else {
        return false;
    };
    if token.eq_ignore_ascii_case("ROUTINE") || token.starts_with("#;") {
        return false;
    }

    let Some(first) = token.chars().next() else {
        return false;
    };
    first.is_ascii_alphabetic() || first == '%' || first == '$'
}

fn has_routine_member_between(content: &str, start_byte: usize, end_byte: usize) -> bool {
    let mut cursor = start_byte;
    let end_byte = end_byte.min(content.len());
    while cursor < end_byte {
        let tail = content.get(cursor..end_byte).unwrap_or("");
        let next_end = tail
            .find('\n')
            .map(|offset| cursor + offset + 1)
            .unwrap_or(end_byte);
        let line = content
            .get(cursor..next_end)
            .unwrap_or("")
            .trim_end_matches('\n')
            .trim_end_matches('\r');
        if line_starts_routine_member(line) {
            return true;
        }
        cursor = next_end;
    }
    false
}

fn line_is_comment_or_blank(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.is_empty()
        || trimmed.starts_with(';')
        || trimmed.starts_with("#;")
        || trimmed.starts_with("//")
        || trimmed.starts_with("/*")
}

fn byte_after_trailing_comments(content: &str, start_byte: usize) -> usize {
    let mut cursor = start_byte.min(content.len());
    while cursor < content.len() {
        let tail = content.get(cursor..).unwrap_or("");
        let line_end = tail
            .find('\n')
            .map(|offset| cursor + offset)
            .unwrap_or(content.len());
        let line = content
            .get(cursor..line_end)
            .unwrap_or("")
            .trim_end_matches('\r');
        if line_starts_routine_member(line) || !line_is_comment_or_blank(line) {
            break;
        }
        cursor = if matches!(content.as_bytes().get(line_end), Some(b'\n')) {
            line_end + 1
        } else {
            line_end
        };
    }
    cursor
}

/// Returns the number of whitespace bytes immediately before `byte_index` on its line.
pub fn whitespace_before_byte(content: &str, byte_index: usize) -> usize {
    let line_start = content
        .get(..byte_index)
        .and_then(|prefix| prefix.rfind('\n').map(|index| index + 1))
        .unwrap_or(0);

    let prefix = content.get(line_start..byte_index).unwrap_or("");

    prefix
        .chars()
        .rev()
        .take_while(|ch| ch.is_whitespace() && *ch != '\n' && *ch != '\r')
        .map(char::len_utf8)
        .sum()
}

/// Counts nested `command_do` ancestors to determine the dot depth for this do block.
pub fn find_do_statement_depth(node: &Node) -> usize {
    let mut count = 1; // outer do has one dot for its statements
    let mut parent = node.clone();
    while let Some(next) = parent.parent() {
        if next.kind() == "command_do" {
            count += 1;
        }
        parent = next;
    }
    count
}

// given command_do node, gets string and checks if it has $TEST, JOB, LOCK, OPEN, or READ
// if so, returns true, otherwise false
fn changes_test_variable(content: &str, node: &Node) -> bool {
    let Some(str) = get_string_at_byte_range(content, node.byte_range()) else {
        eprintln!("Couldn't get string from node {:?}", node.kind());
        return false;
    };
    let str = str.to_lowercase();
    str.contains("$test")
        || str.contains("job")
        || str.contains("lock")
        || str.contains("open")
        || str.contains("read")
}

struct GeneratedDoSubroutine {
    name: String,
    text: String,
}

fn node_has_child_kind(node: Node, kind: &str) -> bool {
    let mut cursor = node.walk();
    let has_child = node
        .named_children(&mut cursor)
        .any(|child| child.kind() == kind);
    has_child
}

fn is_old_do_with_dotted_body(content: &str, node: Node) -> bool {
    node.kind() == "command_do"
        && node_has_child_kind(node, "keyword_do_old")
        && direct_dotted_body_depth(content, node.range()).is_some()
}

fn line_end_before_newline(content: &str, start_byte: usize, max_end_byte: usize) -> usize {
    content
        .get(start_byte..max_end_byte)
        .and_then(|tail| tail.find('\n'))
        .map(|offset| start_byte + offset)
        .unwrap_or(max_end_byte)
}

fn point_at_byte(content: &str, byte_index: usize) -> tree_sitter::Point {
    let safe_byte_index = byte_index.min(content.len());
    let prefix = content.get(..safe_byte_index).unwrap_or("");
    let row = prefix.bytes().filter(|byte| *byte == b'\n').count();
    let line_start = prefix.rfind('\n').map(|index| index + 1).unwrap_or(0);
    tree_sitter::Point {
        row,
        column: safe_byte_index - line_start,
    }
}

fn next_line_start(content: &str, start_byte: usize, max_end_byte: usize) -> Option<usize> {
    content
        .get(start_byte..max_end_byte)
        .and_then(|tail| tail.find('\n'))
        .map(|offset| start_byte + offset + 1)
        .filter(|next| *next < max_end_byte)
}

/// Counts the number of leading dots (ObjectScript block depth indicators) in a line.
pub fn count_leading_dots_in_line(line: &str) -> usize {
    let bytes = line.as_bytes();
    let mut cursor = 0;
    while matches!(bytes.get(cursor), Some(b' ' | b'\t')) {
        cursor += 1;
    }

    let mut count = 0;
    loop {
        if !matches!(bytes.get(cursor), Some(b'.')) {
            break;
        }
        count += 1;
        cursor += 1;
        while matches!(bytes.get(cursor), Some(b' ' | b'\t')) {
            cursor += 1;
        }
    }
    count
}

fn direct_dotted_body_depth(content: &str, range: tree_sitter::Range) -> Option<usize> {
    let command_line_start = content
        .get(..range.start_byte)
        .and_then(|prefix| prefix.rfind('\n').map(|index| index + 1))
        .unwrap_or(0);
    let command_line_end = line_end_before_newline(content, command_line_start, content.len());
    let command_line = content
        .get(command_line_start..command_line_end)
        .unwrap_or("")
        .trim_end_matches('\r');
    let expected_depth = count_leading_dots_in_line(command_line) + 1;
    let mut cursor = next_line_start(content, range.start_byte, range.end_byte)?;
    while cursor < range.end_byte {
        let line_end = line_end_before_newline(content, cursor, range.end_byte);
        let line = content
            .get(cursor..line_end)
            .unwrap_or("")
            .trim_end_matches('\r');
        if count_leading_dots_in_line(line) == expected_depth {
            return Some(expected_depth);
        }
        cursor = match next_line_start(content, cursor, range.end_byte) {
            Some(next) => next,
            None => break,
        };
    }
    None
}

fn dotted_body_line_ranges(
    content: &str,
    range: tree_sitter::Range,
    dot_depth: usize,
) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let Some(mut cursor) = next_line_start(content, range.start_byte, range.end_byte) else {
        return ranges;
    };

    while cursor < range.end_byte {
        let line_end = line_end_before_newline(content, cursor, range.end_byte);
        let line = content
            .get(cursor..line_end)
            .unwrap_or("")
            .trim_end_matches('\r');
        if count_leading_dots_in_line(line) >= dot_depth {
            ranges.push(cursor..line_end);
        }
        cursor = match next_line_start(content, cursor, range.end_byte) {
            Some(next) => next,
            None => break,
        };
    }

    ranges
}

fn dotted_body_replacement_end(content: &str, range: tree_sitter::Range) -> Option<usize> {
    let dot_depth = direct_dotted_body_depth(content, range)?;
    let body_line_ranges = dotted_body_line_ranges(content, range, dot_depth);
    let last_body_end = body_line_ranges.last()?.end;
    Some(
        if matches!(content.as_bytes().get(last_body_end), Some(b'\n')) {
            last_body_end + 1
        } else {
            last_body_end
        },
    )
}

fn strip_dotted_prefix(line: &str, dot_depth: usize) -> Option<(String, usize)> {
    let bytes = line.as_bytes();
    let mut cursor = 0;
    while matches!(bytes.get(cursor), Some(b' ' | b'\t')) {
        cursor += 1;
    }

    for index in 0..dot_depth {
        if !matches!(bytes.get(cursor), Some(b'.')) {
            return None;
        }
        cursor += 1;
        if index + 1 < dot_depth {
            while matches!(bytes.get(cursor), Some(b' ' | b'\t')) {
                cursor += 1;
            }
        } else if matches!(bytes.get(cursor), Some(b' ' | b'\t')) {
            cursor += 1;
        }
    }

    Some((
        line.get(cursor..)
            .unwrap_or("")
            .trim_end_matches('\r')
            .to_string(),
        cursor,
    ))
}

fn build_new_do_call(content: &str, node: Node, sub_name: &str) -> Option<String> {
    let mut new_do_call = format!("do {sub_name}");
    let Some(statement_struct) = build_old_statement_struct(&node, content) else {
        eprintln!("Failed to build do command struct");
        return None;
    };
    for range in statement_struct.statements_after {
        let Some(statement) = get_string_at_byte_range(content, range) else {
            eprintln!("Failed to get statement from do statements after");
            return None;
        };
        new_do_call.push_str(format!(" {statement}").as_str());
    }
    Some(new_do_call)
}

fn line_starts_with_quit_or_return(line: &str) -> bool {
    let trimmed = line.trim_start().to_ascii_lowercase();
    trimmed == "q"
        || trimmed == "quit"
        || trimmed == "return"
        || trimmed.starts_with("q ")
        || trimmed.starts_with("quit ")
        || trimmed.starts_with("return ")
}

fn normalize_generated_dotted_line(line: &str) -> String {
    let trimmed = line.trim();
    let mut normalized = String::with_capacity(trimmed.len());

    for ch in trimmed.chars() {
        if ch == '{' {
            while normalized.ends_with(' ') || normalized.ends_with('\t') {
                normalized.pop();
            }
            if !normalized.is_empty() {
                normalized.push(' ');
            }
        }
        normalized.push(ch);
    }

    normalized
}

fn leading_closing_braces(line: &str) -> usize {
    line.chars().take_while(|ch| *ch == '}').count()
}

fn brace_counts(line: &str) -> (usize, usize) {
    line.chars().fold((0, 0), |(open, close), ch| match ch {
        '{' => (open + 1, close),
        '}' => (open, close + 1),
        _ => (open, close),
    })
}

fn build_generated_dotted_do(
    content: &str,
    command_do: Node,
    outer_subroutine_name: &str,
    routine_members: &mut Vec<String>,
    newline: &str,
) -> Option<GeneratedDoSubroutine> {
    let dot_depth = direct_dotted_body_depth(content, command_do.range())?;
    let sub_name = generate_subroutine_name(outer_subroutine_name, dot_depth, routine_members);
    let body_line_ranges = dotted_body_line_ranges(content, command_do.range(), dot_depth);
    if body_line_ranges.is_empty() {
        eprintln!("There should be at least one dotted statement, found none");
        return None;
    }

    let base_indent = "    ";
    let block_indent = "   ";
    let mut body = String::new();
    let mut quit_or_return_end = false;
    let mut block_depth = 0usize;

    if changes_test_variable(content, &command_do) {
        body.push_str(format!("{base_indent}set temp=$TEST{newline}").as_str());
    }

    for line_range in body_line_ranges {
        let Some(raw_line) = content.get(line_range.clone()) else {
            eprintln!("Error: couldn't get dotted statement string from range");
            return None;
        };
        let Some((line, _)) = strip_dotted_prefix(raw_line, dot_depth) else {
            eprintln!("Error: couldn't strip dotted statement prefix");
            return None;
        };
        let line = normalize_generated_dotted_line(line.as_str());
        let indent_depth = block_depth.saturating_sub(leading_closing_braces(line.as_str()));
        quit_or_return_end = line_starts_with_quit_or_return(line.as_str());
        body.push_str(base_indent);
        for _ in 0..indent_depth {
            body.push_str(block_indent);
        }
        body.push_str(line.as_str());
        body.push_str(newline);

        let (open_braces, close_braces) = brace_counts(line.as_str());
        block_depth = block_depth.saturating_add(open_braces);
        block_depth = block_depth.saturating_sub(close_braces);
    }

    if changes_test_variable(content, &command_do) {
        body.push_str(format!("{base_indent}set $TEST=temp{newline}").as_str());
    }
    if !quit_or_return_end {
        body.push_str(format!("{base_indent}quit{newline}").as_str());
    }

    let text = format!("{newline}{sub_name} Private{newline}{body}");

    Some(GeneratedDoSubroutine {
        name: sub_name,
        text,
    })
}

fn refactor_smallest_dotted_do(
    tree: &mut tree_sitter::Tree,
    updated_string: &mut String,
    language: &Language,
    query_str: &str,
    _parser: &mut Parser,
    routine_members: &mut Vec<String>,
) -> bool {
    if let Ok(query) = Query::new(language, query_str) {
        let mut nodes = Vec::new();
        let root = tree.root_node();
        let mut cursor = QueryCursor::new();
        let mut iter = cursor.matches(&query, root, updated_string.as_bytes());
        while let Some(m) = iter.next() {
            nodes.push(m.captures[0].node);
        }
        nodes.sort_by_key(|node| node.start_byte());
        if nodes.is_empty() {
            return false;
        }
        let Some(command_do) = nodes
            .into_iter()
            .find(|node| is_old_do_with_dotted_body(updated_string.as_str(), *node))
        else {
            return false;
        };

        let Some(associated_parent) = command_do.parent() else {
            eprintln!("Couldn't get parent node for do command");
            return false;
        };

        let Some((outer_subroutine_name, sub_last_statement_range)) =
            find_do_statement_subroutine(updated_string.as_str(), &associated_parent)
        else {
            eprintln!(
                "Couldn't find do statement subroutine {:?}",
                command_do.kind()
            );
            return false;
        };
        let newline = detect_newline(updated_string);
        let Some(generated) = build_generated_dotted_do(
            updated_string.as_str(),
            command_do,
            outer_subroutine_name.as_str(),
            routine_members,
            newline,
        ) else {
            return false;
        };
        let Some(mut new_do_call) =
            build_new_do_call(updated_string.as_str(), command_do, generated.name.as_str())
        else {
            return false;
        };

        let mut old_do_range = command_do.range();
        if let Some(end_byte) =
            dotted_body_replacement_end(updated_string.as_str(), command_do.range())
        {
            if end_byte < old_do_range.end_byte {
                old_do_range.end_byte = end_byte;
                old_do_range.end_point = point_at_byte(updated_string.as_str(), end_byte);
            }
        }
        let old_do_spans_lines = old_do_range.start_point.row != old_do_range.end_point.row;
        let mut added_comment = false;
        let Some(statement_struct) =
            build_old_statement_struct(&command_do, updated_string.as_str())
        else {
            eprintln!("Failed to build do command struct");
            return false;
        };
        if let Some(comment_range) = statement_struct.comment_after_last_statement_range {
            let Some(comment) = get_string_at_byte_range(
                updated_string.as_str(),
                comment_range.start_byte..comment_range.end_byte,
            ) else {
                eprintln!("Failed to get comment after dotted do");
                return false;
            };
            new_do_call.push_str(newline);
            new_do_call.push_str(comment.as_str());
            added_comment = true;
        } else if let Some(comment_range) = statement_struct.comment_range {
            let Some(comment) = get_string_at_byte_range(
                updated_string.as_str(),
                comment_range.start_byte..comment_range.end_byte,
            ) else {
                eprintln!("Failed to get comment for dotted do");
                return false;
            };
            new_do_call.push_str(newline);
            new_do_call.push_str(comment.as_str());
            added_comment = true;
        }
        if old_do_spans_lines || added_comment {
            new_do_call.push_str(newline);
        }
        let insert_byte = if has_routine_member_between(
            updated_string.as_str(),
            old_do_range.end_byte,
            sub_last_statement_range.end_byte,
        ) {
            byte_after_trailing_comments(updated_string.as_str(), old_do_range.end_byte)
        } else {
            byte_after_trailing_comments(updated_string.as_str(), sub_last_statement_range.end_byte)
        };
        let insert_point = point_at_byte(updated_string.as_str(), insert_byte);
        let insert_range = tree_sitter::Range {
            start_byte: insert_byte,
            end_byte: insert_byte,
            start_point: insert_point,
            end_point: insert_point,
        };
        update_tree_and_content(tree, updated_string, insert_range, generated.text.as_str());
        update_tree_and_content(tree, updated_string, old_do_range, new_do_call.as_str());
        return true;
    } else {
        return false;
    }
}

/// Given a source_file node, parse the routine and update spacing for first level statements
fn refactor_spacing_for_subroutines(root: Node, content: &mut String) -> Option<String> {
    let base_indent = "    ";
    let mut replacement_string = String::new();
    let statement_children = get_node_children(root);
    let newline = detect_newline(content);
    for statement in statement_children {
        match statement.kind() {
            "line_comment_1" | "line_comment_2" | "line_comment_4" | "routine_definition" => {
                let Some(statement_str) =
                    get_string_at_byte_range(content.as_str(), statement.byte_range())
                else {
                    eprintln!("Error: Failed to get statement string at byte range");
                    return None;
                };
                replacement_string.push_str(format!("{statement_str}{newline}").as_str());
            }
            "block_comment" | "line_comment_3" => {
                let Some(statement_str) =
                    get_string_at_byte_range(content.as_str(), statement.byte_range())
                else {
                    eprintln!("Error: Failed to get statement string at byte range");
                    return None;
                };
                replacement_string
                    .push_str(format!("{base_indent}{statement_str}{newline}").as_str());
            }
            _ => {
                let Some(command) = statement.named_child(0) else {
                    eprintln!("Error: Expected statement to have a child, but got None");
                    return None;
                };
                let Some(statement_str) =
                    get_string_at_byte_range(content.as_str(), statement.byte_range())
                else {
                    eprintln!("Error: Failed to get statement string at byte range");
                    return None;
                };
                if command.kind() == "tag_statement" || command.kind() == "procedure" {
                    replacement_string.push_str(newline);
                    let str = format!("{statement_str}{newline}");
                    replacement_string.push_str(str.as_str());
                    continue;
                }
                let trimmed = statement_str.trim_start();
                let new = format!("{base_indent}{trimmed}{newline}");
                replacement_string.push_str(new.as_str());
            }
        }
    }
    Some(replacement_string)
}

/// Extracts dotted `do` bodies into named subroutines and replaces them with `do subroutineName` calls.
pub fn refactor_legacy_do_statements(content: &str) -> String {
    let language = LANGUAGE_OBJECTSCRIPT_ROUTINE.into();
    let Some(mut parser) = create_parser(&language) else {
        eprintln!("Error: Failed to create parser");
        return content.to_string();
    };
    let Some(mut tree) = parser.parse(content, None) else {
        eprintln!("Failed to parse content");
        return content.to_string();
    };
    let mut curr_routine_members = routine_members(&tree.root_node(), content);

    let mut updated_string = content.to_string();

    let query_str = "(command_do (keyword_do_old)) @command";
    let mut at_least_one_change = false;

    loop {
        let changed = refactor_smallest_dotted_do(
            &mut tree,
            &mut updated_string,
            &language,
            query_str,
            &mut parser,
            &mut curr_routine_members,
        );
        let new_tree = parser.parse(updated_string.as_str(), Some(&tree)).unwrap();
        tree = new_tree;
        if !changed {
            break;
        }
        at_least_one_change = true;
    }
    if at_least_one_change {
        if let Some(replacement_str) =
            refactor_spacing_for_subroutines(tree.root_node(), &mut updated_string)
        {
            let range = tree.root_node().range();
            update_tree_and_content(&mut tree, &mut updated_string, range, &replacement_str);
        };
    }

    updated_string
}

/// given a statement node of a do statement, find the subroutine or procedure that the
/// node is a part of
pub fn find_do_statement_subroutine(
    content: &str,
    node: &Node,
) -> Option<(String, tree_sitter::Range)> {
    let mut tracker = node.clone();

    while let Some(next) = tracker.parent() {
        if next.kind() == "procedure" || next.kind() == "source_file" {
            break;
        }
        tracker = next;
    }
    let node = tracker;
    if let Some(parent) = node.parent() {
        if parent.kind() == "procedure" {
            let Some(tag_with_params) = parent.named_child(0) else {
                eprintln!("Expected procedure child to be tag_with_params, but it dne");
                return None;
            };
            let Some(tag) = tag_with_params.named_child(0) else {
                eprintln!("Expected tag_with_params child to be tag, but it dne");
                return None;
            };
            let Some(name) = get_string_at_byte_range(content, tag.byte_range()) else {
                eprintln!("Failed to get tag name");
                return None;
            };

            return Some((name, parent.range()));
        }
    }
    let mut prev_sibling = node.prev_named_sibling();
    let name;
    // now outside of this, we need to look for the closest tag statement
    while let Some(statement) = prev_sibling {
        let Some(command) = statement.named_child(0) else {
            prev_sibling = statement.prev_named_sibling();
            continue;
        };
        if command.kind() == "tag_statement" || command.kind() == "tag_with_params" {
            let Some(tag) = command.named_child(0) else {
                eprintln!("Couldn't get tag statement child");
                return None;
            };
            let Some(tag_name) = get_string_at_byte_range(content, tag.byte_range()) else {
                eprintln!("Failed to get tag name string");
                return None;
            };
            name = tag_name;

            let mut next_sibling = node.next_named_sibling();
            let mut last_statement_before_end = None;
            while let Some(next_statement) = next_sibling {
                let Some(command) = next_statement.named_child(0) else {
                    next_sibling = next_statement.next_named_sibling();
                    continue;
                };
                if command.kind() == "tag_statement"
                    || command.kind() == "tag_with_params"
                    || command.kind() == "procedure"
                {
                    let Some(last_statement_before_end) = next_statement.prev_named_sibling()
                    else {
                        eprintln!("Couldn't get last statement before end");
                        return None;
                    };
                    return Some((name, last_statement_before_end.range()));
                }
                last_statement_before_end = Some(next_statement);
                next_sibling = next_statement.next_named_sibling();
            }
            if let Some(last_statement_before_end) = last_statement_before_end {
                let mut sibling = node.next_sibling();
                let mut last_node = None;

                while let Some(next) = sibling {
                    last_node = Some(next);
                    sibling = next.next_sibling();
                }
                if let Some(last_node) = last_node {
                    return Some((name, last_node.range()));
                }
                return Some((name, last_statement_before_end.range()));
            } else {
                return Some((name, node.range()));
            }
        }
        prev_sibling = statement.prev_named_sibling();
        continue;
    }
    return None;
}
