#[cfg(test)]
mod tests {
    use crate::backend_testing::BackendTester;
    use objectscript_core::common::get_keyword_and_value;
    use objectscript_core::parse_structures::{FileType, Language, RefactorLevel, VariableRef};
    use objectscript_core::refactor::{refactor_conditionals, refactor_legacy_do_statements};
    use objectscript_core::workspace::ProjectState;
    use std::collections::HashSet;
    use std::env;
    use std::path::PathBuf;
    use tower_lsp::lsp_types::Url;
    use tree_sitter::{Parser, Point, Range};
    use tree_sitter_objectscript_routine::LANGUAGE_OBJECTSCRIPT_ROUTINE;

    fn parse_routine(code: &str) -> tree_sitter::Tree {
        let mut parser = Parser::new();
        parser
            .set_language(&LANGUAGE_OBJECTSCRIPT_ROUTINE.into())
            .expect("failed to load objectscript grammar");
        parser.parse(code, None).expect("parse returned None")
    }

    async fn setup_backend_and_workspace(project_root: PathBuf) -> (BackendTester, Url) {
        // create projectState and set the projectRoot
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

    fn point_for_substring_n(content: &str, needle: &str, occurrence: usize) -> Point {
        assert!(occurrence > 0, "occurrence must be >= 1");
        let mut start = 0usize;
        let mut found_at = None;
        for _ in 0..occurrence {
            let Some(idx) = content[start..].find(needle) else {
                panic!(
                    "failed to find occurrence {} of substring {:?}",
                    occurrence, needle
                );
            };
            found_at = Some(start + idx);
            start = start + idx + needle.len();
        }
        let byte_index = found_at.expect("occurrence lookup should set found_at");
        point_from_byte_index(content, byte_index)
    }

    fn point_for_substring(content: &str, needle: &str) -> Point {
        point_for_substring_n(content, needle, 1)
    }

    fn range_for_substring(content: &str, needle: &str) -> Range {
        range_for_substring_n(content, needle, 1)
    }

    fn range_for_substring_n(content: &str, needle: &str, occurrence: usize) -> Range {
        assert!(occurrence > 0, "occurrence must be >= 1");
        let mut start = 0usize;
        let mut found_at = None;
        for _ in 0..occurrence {
            let Some(idx) = content[start..].find(needle) else {
                panic!(
                    "failed to find occurrence {} of substring {:?}",
                    occurrence, needle
                );
            };
            found_at = Some(start + idx);
            start = start + idx + needle.len();
        }
        let start_byte = found_at.expect("occurrence lookup should set found_at");
        let end_byte = start_byte + needle.len();
        let start_point = point_from_byte_index(content, start_byte);
        let end_point = point_from_byte_index(content, end_byte);
        Range {
            start_byte,
            end_byte,
            start_point,
            end_point,
        }
    }

    fn point_from_byte_index(content: &str, byte_index: usize) -> Point {
        assert!(
            byte_index <= content.len(),
            "byte index out of bounds for content"
        );
        let mut row = 0usize;
        let mut column = 0usize;
        for b in content.as_bytes().iter().take(byte_index) {
            if *b == b'\n' {
                row += 1;
                column = 0;
            } else {
                column += 1;
            }
        }
        Point { row, column }
    }

    #[test]
    fn test_get_keyword_and_value() {
        let (not, keyword, values) = get_keyword_and_value("ClientDataType = longvarchar");
        let value = values.get(0).copied();
        assert!(!not);
        assert_eq!(keyword, "clientdatatype");
        assert_eq!(value, Some("longvarchar"));
        let (not, keyword, values) = get_keyword_and_value("ClientDataType=longvarchar");
        let value = values.get(0).copied();
        assert!(!not);
        assert_eq!(keyword, "clientdatatype");
        assert_eq!(value, Some("longvarchar"));
        let (not, keyword, values) = get_keyword_and_value("ProcedureBlock = 1");
        let value = values.get(0).copied();
        assert!(!not);
        assert_eq!(keyword, "procedureblock");
        assert_eq!(value, Some("1"));
        let (not, keyword, _) = get_keyword_and_value("Not ProcedureBlock");
        assert!(not);
        assert_eq!(keyword, "procedureblock");
    }

    #[tokio::test]
    async fn test_goto_def_routines() {
        let project_root = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("gotodef");
        let (backend, uri) = setup_backend_and_workspace(project_root).await;
        let project_state = backend.get_project(&uri).unwrap();
        let project_data = project_state.data.read();

        eprintln!("CLASSES: {:#?}", project_data.classes.clone());
        eprintln!("VARIABLES: {:#?}", project_data.pub_var_defs.clone())
    }

    #[tokio::test]
    async fn test_scope_tree_for_routine() {
        let project_root = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("gotodef")
            .join("routines");
        let (backend, uri) = setup_backend_and_workspace(project_root).await;
        let project_state = backend.get_project(&uri).unwrap();
        let project_data = project_state.data.read();

        let methods = project_data.method_defs.get("crossref").unwrap();

        println!("METHOD LEN {:?}", methods.len());
    }

    #[tokio::test]
    async fn test_goto_def_inherited_method_relative() {
        let project_root = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("gotodef")
            .join("relative-method-call");
        let (backend, uri) = setup_backend_and_workspace(project_root).await;
        let project_state = backend.get_project(&uri).unwrap();
        let project_data = project_state.data.read();

        let superclass_id = project_data.classes.get("hk").unwrap();
        let superclass = project_data
            .global_semantic_model
            .get_class(superclass_id)
            .unwrap();
        let superclass_method_ref = superclass.get_method_ref("print2").unwrap();
        let methods = project_data
            .override_index
            .effective_methods
            .get("hksubclass")
            .unwrap();
        eprintln!("METHODS {:?}", methods);
        let method_ref = methods.get("print2").unwrap();
        assert_eq!(method_ref, superclass_method_ref);
    }

    #[tokio::test]
    async fn test_variables() {
        let project_root = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("variables");
        let document_url = Url::from_file_path(
            env::current_dir()
                .unwrap()
                .join("objectscript-tests")
                .join("variables")
                .join("testing-variable-building.cls"),
        )
        .unwrap();
        let (backend, uri) = setup_backend_and_workspace(project_root).await;
        let project_state = backend.get_project(&uri).unwrap();

        let (tree, content, before_public_variables, before_classes) = {
            let project_data = project_state.data.read();
            let (_, content, _, tree) = project_data.get_document_info(&document_url).unwrap();
            (
                tree,
                content,
                project_data.pub_var_defs.clone(),
                project_data.classes.clone(),
            )
        };

        assert_eq!(before_classes.len(), 4);
        assert!(before_classes.contains_key("SuperClass"));
        assert!(before_classes.contains_key("SubClassOne"));
        assert!(before_classes.contains_key("SubClassTwo"));
        assert!(before_classes.contains_key("ScopeResolution"));

        let superclass_id = before_classes.get("SuperClass").unwrap();
        let subclassone_id = before_classes.get("SubClassOne").unwrap();
        let subclasstwo_id = before_classes.get("SubClassTwo").unwrap();

        let mut superclass_count = 0;
        let mut subclassone_count = 0;
        let mut subclasstwo_count = 0;
        let before_x = before_public_variables
            .get("x")
            .expect("missing public variable x");

        for (method_ref, _) in before_x {
            if method_ref.class == *superclass_id {
                superclass_count += 1;
            } else if method_ref.class == *subclassone_id {
                subclassone_count += 1;
            } else if method_ref.class == *subclasstwo_id {
                subclasstwo_count += 1;
            }
        }
        assert_eq!(superclass_count, 2);
        assert_eq!(subclassone_count, 1);
        assert_eq!(subclasstwo_count, 1);
        let before_y = before_public_variables
            .get("y")
            .expect("missing public variable y");
        superclass_count = 0;

        for (method_ref, _) in before_y {
            if method_ref.class == *superclass_id {
                superclass_count += 1;
            }
        }
        assert_eq!(before_y.len(), 3);
        assert_eq!(superclass_count, 2);

        project_state.update_document(
            document_url,
            &tree,
            FileType::Cls,
            1,
            content.as_str(),
            vec![],
        );

        let (
            after_public_variables,
            after_classes,
            super_class_id,
            sub_one_class_id,
            sub_two_class_id,
            gsm_classes,
        ) = {
            let project_data = project_state.data.read();
            let super_class_id = *project_data
                .classes
                .get("SuperClass")
                .expect("missing SuperClass id");
            let sub_one_class_id = *project_data
                .classes
                .get("SubClassOne")
                .expect("missing SubClassOne id");
            let sub_two_class_id = *project_data
                .classes
                .get("SubClassTwo")
                .expect("missing SubClassTwo id");
            (
                project_data.pub_var_defs.clone(),
                project_data.classes.clone(),
                super_class_id,
                sub_one_class_id,
                sub_two_class_id,
                project_data.global_semantic_model.classes.clone(),
            )
        };

        assert_eq!(after_classes.len(), 4);
        assert!(after_classes.contains_key("SuperClass"));
        assert!(after_classes.contains_key("SubClassOne"));
        assert!(after_classes.contains_key("SubClassTwo"));
        assert!(after_classes.contains_key("ScopeResolution"));

        let after_x = after_public_variables
            .get("x")
            .expect("missing public variable x after update");
        superclass_count = 0;
        subclassone_count = 0;
        subclasstwo_count = 0;
        for (method_ref, _) in after_x {
            if method_ref.class == super_class_id {
                superclass_count += 1;
            } else if method_ref.class == sub_one_class_id {
                subclassone_count += 1;
            } else if method_ref.class == sub_two_class_id {
                subclasstwo_count += 1;
            }
        }
        assert_eq!(superclass_count, 2);
        assert_eq!(subclassone_count, 1);
        assert_eq!(subclasstwo_count, 1);
        let after_y = after_public_variables
            .get("y")
            .expect("missing public variable y after update");
        superclass_count = 0;
        for (method_ref, _) in after_y {
            if method_ref.class == super_class_id {
                superclass_count += 1;
            }
        }
        assert_eq!(after_y.len(), 3);
        assert_eq!(superclass_count, 2);
        let Some(sub_one_class_inherited) = gsm_classes.get(&sub_one_class_id) else {
            panic!("Error: subclass one DNE in classes");
        };
        let Some(sub_two_class_inherited) = gsm_classes.get(&sub_two_class_id) else {
            panic!("Error: subclass two DNE in classes");
        };

        assert_eq!(
            sub_one_class_inherited.inherited_classes,
            vec!["SuperClass".to_string()]
        );
        assert_eq!(
            sub_two_class_inherited.inherited_classes,
            vec!["SuperClass".to_string()]
        );
    }

    #[tokio::test]
    async fn test_class_keyword_inheritance() {
        // KEYWORDS: language = objectscript, inheritance = right, Not ProcedureBlock
        let project_root = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("inheritance")
            .join("class-keyword-inheritance.cls");
        let (backend, uri) = setup_backend_and_workspace(project_root).await;
        let project_state = backend.get_project(&uri).unwrap();
        let project_data = project_state.data.read();
        let classes = project_data.classes.clone();
        let gsm = project_data.global_semantic_model.clone();
        for class_id in classes.values() {
            let Some(class) = gsm.classes.get(class_id) else {
                panic!("Class DNE");
            };
            // eprintln!("CLASS: {:#?}", class);

            assert_eq!(class.is_procedure_block, Some(false));
            assert_eq!(class.default_language, Some(Language::Objectscript));
            assert_eq!(class.inheritance_direction, Some("right".to_string()));
            // get methods
            for (_, method_ref) in class.methods.clone() {
                let method = gsm.methods.get(&method_ref).unwrap();
                eprintln!("METHOD: {:#?}", method);
                if method.name == "newVarChange" {
                    assert_eq!(method.variables.len(), 1);
                    let variable_refs = method.variables.get("x").unwrap();
                    assert_eq!(variable_refs.len(), 1);
                    for (variable_ref, _scope_id) in variable_refs {
                        assert!(variable_ref.pub_id.is_none());
                        assert!(variable_ref.priv_id.is_some());
                    }
                    assert_eq!(method.is_procedure_block, Some(true));
                    assert_eq!(method.language, None);
                } else {
                    let all_var_refs: Vec<&Vec<(VariableRef, _)>> =
                        method.variables.values().collect();
                    for variable_refs in all_var_refs {
                        for (variable_ref, _scope_id) in variable_refs {
                            assert!(variable_ref.pub_id.is_some());
                            assert!(variable_ref.priv_id.is_none());
                        }
                    }
                    assert_eq!(method.is_procedure_block, None);
                    assert_eq!(method.language, None);
                }
            }
        }
    }

    #[tokio::test]
    // EC-GDEF-001
    async fn test_goto_definition_private_variable_procedure_block() {
        let file_path = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("inheritance")
            .join("class-keyword-inheritance.cls");
        let (backend, uri) = setup_backend_and_workspace(file_path.clone()).await;
        let project_state = backend.get_project(&uri).expect("missing project state");
        let document_url = Url::from_file_path(file_path).expect("failed to create file url");
        let project_data = project_state.data.read();

        let document = project_data
            .documents
            .get(&document_url)
            .expect("missing class-keyword-inheritance document");
        let content = document.content.as_str();
        let use_point = point_for_substring(content, "w x");
        let set_point = point_for_substring(content, "set x = 2");

        let locations =
            project_data.get_variable_definition(&document_url, use_point, "x".to_string());

        assert_eq!(locations.len(), 1);
        assert_eq!(locations[0].0, document_url);
        assert_eq!(locations[0].1.start_point.row, set_point.row);
    }

    #[tokio::test]
    async fn test_goto_definition_public_variable_scope_and_workspace_resolution() {
        let project_root = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("variables");
        let document_url = Url::from_file_path(
            env::current_dir()
                .unwrap()
                .join("objectscript-tests")
                .join("variables")
                .join("testing-variable-building.cls"),
        )
        .expect("failed to build document url");
        let (backend, uri) = setup_backend_and_workspace(project_root).await;
        let project_state = backend.get_project(&uri).expect("missing project state");
        let project_data = project_state.data.read();

        let document = project_data
            .documents
            .get(&document_url)
            .expect("missing testing-variable-building document");
        let content = document.content.as_str();

        // In multiplePubVarDefs, y is defined in the same method scope and should resolve to one symbol.
        let y_use_point = point_for_substring_n(content, "w y", 3);
        let y_def_point = point_for_substring(content, "set y = 3");
        let y_locations =
            project_data.get_variable_definition(&document_url, y_use_point, "y".to_string());
        assert_eq!(y_locations.len(), 1);
        assert_eq!(y_locations[0].0, document_url);
        assert_eq!(y_locations[0].1.start_point.row, y_def_point.row);

        let superclass_id = *project_data
            .classes
            .get("SuperClass")
            .expect("missing SuperClass");
        let dependent_names: HashSet<String> = project_data
            .dependent_class_index
            .dependent_classes
            .get(&superclass_id)
            .expect("missing dependents for SuperClass")
            .iter()
            .filter_map(|class_id| {
                project_data
                    .global_semantic_model
                    .get_class(&class_id)
                    .map(|class| class.name.clone())
            })
            .collect();
        assert!(dependent_names.contains("SubClassOne"));
        assert!(dependent_names.contains("SubClassTwo"));

        // In multiplePubVarDefs, x is not in current scope, so workspace-wide public definitions are returned.
        let x_use_point = point_for_substring_n(content, "w x", 2);
        let x_locations =
            project_data.get_variable_definition(&document_url, x_use_point, "x".to_string());
        assert_eq!(x_locations.len(), 2);
        let paths: HashSet<String> = x_locations
            .into_iter()
            .map(|(url, _)| url.path().to_string())
            .collect();
        assert!(
            paths
                .iter()
                .any(|p| p.ends_with("testing-variable-building.cls"))
        );
        assert!(paths.iter().any(|p| p.ends_with("subclass.cls")));
    }

    #[tokio::test]
    async fn test_nested_refactor() {
        let test_route = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("nested_dots");
        let actual_result_path = test_route.join("test-nested-refactor-actual.mac");
        let expected_result_path = test_route.join("test-nested-refactor-expected.mac");
        let test_mac_path = test_route.join("test-nested-refactor.mac");
        let test_mac_url = Url::from_file_path(&test_mac_path).unwrap();
        let (backend, uri) = setup_backend_and_workspace(test_route).await;
        let project_state = backend.get_project(&uri).expect("missing project state");
        let project_data = project_state.data.read();
        let refactored = project_data
            .refactor(RefactorLevel::DoCommands)
            .into_iter()
            .find_map(|(content, url)| (url == test_mac_url).then_some(content))
            .expect("missing planned refactor for test_nested_refactor.mac");
        std::fs::write(&actual_result_path, &refactored).unwrap();
        let actual_contents = std::fs::read_to_string(&actual_result_path).unwrap();
        let expected_contents = std::fs::read_to_string(expected_result_path).unwrap();
        assert_eq!(actual_contents, expected_contents)
    }

    #[tokio::test]
    async fn test_dotted_block() {
        let test_route = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("dotted-block");
        let actual_result_path = test_route.join("test-dotted-block-actual.mac");
        let expected_result_path = test_route.join("test-dotted-block-expected.mac");
        let _ = std::fs::remove_file(&actual_result_path);
        let test_mac_path = test_route.join("test-dotted-block.mac");
        let test_mac_url = Url::from_file_path(&test_mac_path).unwrap();
        let (backend, uri) = setup_backend_and_workspace(test_route).await;
        let project_state = backend.get_project(&uri).expect("missing project state");
        let project_data = project_state.data.read();
        let refactored = project_data
            .refactor(RefactorLevel::DoCommands)
            .into_iter()
            .find_map(|(content, url)| (url == test_mac_url).then_some(content))
            .expect("missing planned refactor for test_dotted_block.mac");
        std::fs::write(&actual_result_path, &refactored).unwrap();
        let actual_contents = std::fs::read_to_string(&actual_result_path).unwrap();
        let expected_contents = std::fs::read_to_string(expected_result_path).unwrap();
        assert_eq!(expected_contents, actual_contents);
    }

    #[tokio::test]
    async fn test_large_dotted_statements() {
        let routines_root = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("local");
        let actual_result_path = routines_root.join("test-large-dotted-statements-actual.mac");
        let expected_result_path = routines_root.join("test-large-dotted-statements-expected.mac");
        let _ = std::fs::remove_file(&actual_result_path);
        let test_mac_path = routines_root.join("test-large-dotted-statements.mac");
        let test_mac_url = Url::from_file_path(&test_mac_path).unwrap();
        let (backend, uri) = setup_backend_and_workspace(routines_root).await;
        let project_state = backend.get_project(&uri).expect("missing project state");
        let project_data = project_state.data.read();
        let refactored = project_data
            .refactor(RefactorLevel::DoCommands)
            .into_iter()
            .find_map(|(content, url)| (url == test_mac_url).then_some(content))
            .expect("missing planned refactor for test-refactor-do.mac");
        std::fs::write(&actual_result_path, &refactored).unwrap();
        let actual_content = std::fs::read_to_string(&actual_result_path).unwrap();
        let expected_content = std::fs::read_to_string(&expected_result_path).unwrap();
        assert_eq!(actual_content, expected_content,);
    }

    #[tokio::test]
    async fn test_refactor_do() {
        let routines_root = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("routines");
        let actual_result_path = routines_root.join("test-refactor-do-actual.mac");
        let _ = std::fs::remove_file(&actual_result_path);

        let test_mac_path = routines_root.join("test-refactor-do.mac");
        let test_mac_url = Url::from_file_path(&test_mac_path).unwrap();

        let (backend, uri) = setup_backend_and_workspace(routines_root).await;
        let project_state = backend.get_project(&uri).expect("missing project state");
        let project_data = project_state.data.read();
        let refactored = project_data
            .refactor(RefactorLevel::DoCommands)
            .into_iter()
            .find_map(|(content, url)| (url == test_mac_url).then_some(content))
            .expect("missing planned refactor for test-refactor-do.mac");
        let tree = parse_routine(refactored.as_str());
        std::fs::write(&actual_result_path, &refactored).unwrap();
        let contents =
            std::fs::read_to_string("objectscript-tests/routines/test-refactor-do-expected.mac")
                .unwrap();
        let expected_tree = parse_routine(contents.as_str());
        assert_eq!(
            tree.root_node().to_sexp(),
            expected_tree.root_node().to_sexp(),
        );
    }

    #[test]
    fn refactors_old_dotted_do_into_generated_subroutines() {
        let input = r#"ROUTINE test

check() private
 set x = 2
 set y = 5
 i x = 2 d  d okay
 . set x = 1.2
 . set y = 1.2
 . w !,"goodbye" d
 . . new x
 . . set x = 250
 . quit
  w !,"again x=",x
  w !,"y=",y
  w !,"leaving"
  quit

after
 quit
"#;
        let expected = r#"ROUTINE test

check() private
    set x = 2
    set y = 5
    i x = 2 do checkSubroutine1 d okay
    w !,"again x=",x
    w !,"y=",y
    w !,"leaving"
    quit

checkSubroutine1 Private
    set x = 1.2
    set y = 1.2
    w !,"goodbye"
    do checkSubroutine2
    quit

checkSubroutine2 Private
    new x
    set x = 250
    quit

after
    quit
"#;

        let actual = refactor_legacy_do_statements(input);
        assert_eq!(actual, expected);
    }

    #[test]
    fn refactors_old_if_into_braced_block() {
        let input = r#"ROUTINE test

check()
 i x = 2 set y = 5 set z = 6
 quit
"#;
        let expected = r#"ROUTINE test

check()
 i x = 2 {
    set y = 5
    set z = 6
 }
 quit
"#;

        let actual = refactor_conditionals(input, FileType::Routine);
        assert_eq!(actual, expected);
    }

    #[test]
    fn refactors_nested_old_if_into_consistently_indented_blocks() {
        let input = r#"ROUTINE test

check()
 i x = 2 w hi i y = 5 w goodbye
 quit
"#;
        let expected = r#"ROUTINE test

check()
 i x = 2 {
    w hi
    i y = 5 {
       w goodbye
    }
 }
 quit
"#;

        let actual = refactor_conditionals(input, FileType::Routine);
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    // EC-GIMP-002
    async fn test_goto_implementation_returns_all_subclass_overrides() {
        let project_root = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("variables");

        let (backend, uri) = setup_backend_and_workspace(project_root).await;
        let project_state = backend.get_project(&uri).expect("missing project state");
        let project_data = project_state.data.read();
        let method_ref = project_data
            .method_defs
            .get("SuperClass")
            .and_then(|m| m.get("newVarChange"))
            .unwrap();
        let locations = project_data.get_method_overrides(method_ref);
        assert_eq!(locations.len(), 2);
        let paths: HashSet<String> = locations
            .into_iter()
            .map(|(url, _)| url.path().to_string())
            .collect();
        assert!(paths.iter().any(|p| p.ends_with("subclass.cls")));
        assert!(paths.iter().any(|p| p.ends_with("subclass_two.cls")));
    }

    #[tokio::test]
    // EC-GIMP-003
    async fn test_goto_implementation_includes_private_override() {
        let project_root = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("navigation")
            .join("implementation");
        let (backend, uri) = setup_backend_and_workspace(project_root).await;
        let project_state = backend.get_project(&uri).expect("missing project state");
        let project_data = project_state.data.read();
        let method_ref = project_data
            .method_defs
            .get("Demo.NavSuper")
            .and_then(|m| m.get("Overridden"))
            .unwrap();
        let locations = project_data.get_method_overrides(method_ref);

        assert_eq!(locations.len(), 2);
        let paths: HashSet<String> = locations
            .into_iter()
            .map(|(url, _)| url.path().to_string())
            .collect();
        assert!(paths.iter().any(|p| p.ends_with("sub_public.cls")));
        assert!(paths.iter().any(|p| p.ends_with("sub_private.cls")));
    }

    #[tokio::test]
    // EC-GDEF-002: Simulates what happens when did_open fires for sub_public BEFORE
    // workspace indexing completes (super.cls not yet known)
    async fn test_goto_definition_from_subclass_override_to_superclass() {
        use objectscript_core::parse_structures::FileType;

        let project_root = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("navigation")
            .join("implementation");

        let state = ProjectState::new();
        if state
            .project_root_path
            .set(Some(project_root.clone()))
            .is_err()
        {
            eprintln!("failed to set the root path");
        }
        let backend = BackendTester::new();
        let uri = Url::from_file_path(project_root.clone()).unwrap();
        backend.add_project(uri.clone(), state);
        let project_state = backend.get_project(&uri).expect("missing project state");

        let sub_public_url = Url::from_file_path(project_root.join("sub_public.cls")).unwrap();
        let sub_private_url = Url::from_file_path(project_root.join("sub_private.cls")).unwrap();
        let super_url = Url::from_file_path(project_root.join("super.cls")).unwrap();

        // Simulate: user opens sub_public.cls FIRST (before workspace is indexed)
        let sub_public_content =
            std::fs::read_to_string(project_root.join("sub_public.cls")).unwrap();
        project_state.handle_document_opened(
            sub_public_url.clone(),
            sub_public_content.clone(),
            FileType::Cls,
            1,
        );

        // Now simulate workspace indexing completing (super.cls and sub_private.cls get added)
        let super_content = std::fs::read_to_string(project_root.join("super.cls")).unwrap();
        project_state.handle_document_opened(super_url.clone(), super_content, FileType::Cls, 1);
        let sub_private_content =
            std::fs::read_to_string(project_root.join("sub_private.cls")).unwrap();
        project_state.handle_document_opened(
            sub_private_url.clone(),
            sub_private_content,
            FileType::Cls,
            1,
        );

        // Simulate user making an edit to sub_public (which triggers update_document)
        {
            let data = project_state.data.read();
            let (file_type, _, version, tree) = data
                .get_document_info(&sub_public_url)
                .expect("sub_public should exist");
            drop(data);
            project_state.update_document(
                sub_public_url.clone(),
                &tree,
                file_type,
                version,
                sub_public_content.as_str(),
                vec![],
            );
        }

        // Now check goto definition from each subclass method
        let project_data = project_state.data.read();

        let sub_public_class_id = project_data
            .documents
            .get(&sub_public_url)
            .and_then(|d| d.class_id)
            .expect("sub_public should have class_id");
        let sub_private_class_id = project_data
            .documents
            .get(&sub_private_url)
            .and_then(|d| d.class_id)
            .expect("sub_private should have class_id");

        // Test goto definition from sub_public's Overridden -> super's Overridden
        let locations_public =
            project_data.get_method_superclass("Overridden".to_string(), &sub_public_class_id);
        assert!(
            !locations_public.is_empty(),
            "sub_public's Overridden should resolve to super's Overridden (after late indexing)"
        );
        assert!(locations_public[0].0.path().ends_with("super.cls"));

        // Test goto definition from sub_private's Overridden -> super's Overridden
        let locations_private =
            project_data.get_method_superclass("Overridden".to_string(), &sub_private_class_id);
        assert!(
            !locations_private.is_empty(),
            "sub_private's Overridden should resolve to super's Overridden"
        );
        assert!(locations_private[0].0.path().ends_with("super.cls"));
    }

    #[tokio::test]
    async fn test_routine_goto_definition_variable() {
        let project_root = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("gotodef");
        let (backend, uri) = setup_backend_and_workspace(project_root.clone()).await;
        let project_state = backend.get_project(&uri).expect("missing project state");
        let project_data = project_state.data.read();

        let routine_url = Url::from_file_path(project_root.join("test-routine-goto.mac")).unwrap();
        let _document = project_data
            .documents
            .get(&routine_url)
            .expect("routine doc exists");

        // "w x" is on line 8 (0-indexed row=7), x is at column 3
        let point = Point { row: 7, column: 3 };

        let locations = project_data.get_variable_definition(&routine_url, point, "x".to_string());
        assert!(
            !locations.is_empty(),
            "should find x definition in gotosubroutine"
        );
    }

    // =========================================================================
    // GOTO DEFINITION — CLASS METHOD CALLS (Test Suite 1.3)
    // =========================================================================

    #[tokio::test]
    async fn test_goto_def_class_method_call_resolves_method() {
        let project_root = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("gotodef")
            .join("class-method-call");
        let (backend, uri) = setup_backend_and_workspace(project_root.clone()).await;
        let project_state = backend.get_project(&uri).expect("missing project state");
        let project_data = project_state.data.read();

        let method_ref = project_data
            .method_defs
            .get("Demo.Utility")
            .and_then(|m| m.get("Helper"))
            .expect("Demo.Utility.Helper should exist");

        let locations = project_data.get_method_definition(method_ref, None);
        assert_eq!(locations.len(), 1);
        assert!(locations[0].0.path().ends_with("utility.cls"));
    }

    #[tokio::test]
    async fn test_goto_def_class_method_call_nonexistent_returns_empty() {
        let project_root = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("gotodef")
            .join("class-method-call");
        let (backend, uri) = setup_backend_and_workspace(project_root).await;
        let project_state = backend.get_project(&uri).expect("missing project state");
        let project_data = project_state.data.read();
        let result = project_data
            .method_defs
            .get("Demo.Utility")
            .and_then(|m| m.get("NonExistent"));
        assert!(result.is_none(), "nonexistent method should not be indexed");
    }

    #[tokio::test]
    async fn test_goto_def_class_reference_resolves_to_class() {
        let project_root = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("gotodef")
            .join("class-method-call");
        let (backend, uri) = setup_backend_and_workspace(project_root).await;
        let project_state = backend.get_project(&uri).expect("missing project state");
        let project_data = project_state.data.read();

        let locations = project_data.get_class_definition("Demo.Utility");
        assert_eq!(locations.len(), 1);
        assert!(locations[0].0.path().ends_with("utility.cls"));
    }

    // =========================================================================
    // GOTO DEFINITION — OREF CONTEXTS (Test Suite 1.4)
    // =========================================================================

    #[tokio::test]
    async fn test_goto_def_oref_resolves_method_in_target_class() {
        let project_root = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("gotodef")
            .join("oref-contexts");
        let (backend, uri) = setup_backend_and_workspace(project_root.clone()).await;
        let project_state = backend.get_project(&uri).expect("missing project state");
        let project_data = project_state.data.read();

        let document_url = Url::from_file_path(project_root.join("oref-do-parameter.cls")).unwrap();
        let document = project_data
            .documents
            .get(&document_url)
            .expect("oref-do-parameter.cls should exist");
        let content = document.content.as_str();

        let oref_range = range_for_substring(content, "do obj.Run()");
        let class_name = &document.class_name;

        let locations =
            project_data.get_oref_definitions("obj", "Run", class_name, oref_range, true);
        assert!(
            !locations.is_empty(),
            "oref method call in do_parameter should resolve to Demo.Target.Run"
        );
        assert!(locations[0].0.path().ends_with("target.cls"));
    }

    #[tokio::test]
    async fn test_goto_def_oref_job_argument_resolves_method() {
        let project_root = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("gotodef")
            .join("oref-contexts");
        let (backend, uri) = setup_backend_and_workspace(project_root.clone()).await;
        let project_state = backend.get_project(&uri).expect("missing project state");
        let project_data = project_state.data.read();

        let document_url = Url::from_file_path(project_root.join("oref-job-argument.cls")).unwrap();
        let document = project_data
            .documents
            .get(&document_url)
            .expect("oref-job-argument.cls should exist");
        let content = document.content.as_str();

        let oref_range = range_for_substring(content, "job worker.Execute()");
        let class_name = &document.class_name;

        let locations =
            project_data.get_oref_definitions("worker", "Execute", class_name, oref_range, true);
        assert!(
            !locations.is_empty(),
            "oref method call in job_argument should resolve to Demo.Target.Execute"
        );
        assert!(locations[0].0.path().ends_with("target.cls"));
    }

    // =========================================================================
    // GOTO DEFINITION — MULTIPLE INHERITANCE (Test Suite 1.1.3, 1.1.4)
    // =========================================================================

    #[tokio::test]
    async fn test_goto_def_multiple_inheritance_default_left() {
        let project_root = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("gotodef")
            .join("multiple-inheritance");
        let (backend, uri) = setup_backend_and_workspace(project_root.clone()).await;
        let project_state = backend.get_project(&uri).expect("missing project state");
        let project_data = project_state.data.read();

        let child_class_id = project_data
            .classes
            .get("Demo.ChildDefault")
            .expect("Demo.ChildDefault should exist");

        let locations = project_data.get_method_superclass("UseParent".to_string(), child_class_id);
        // UseParent is not in any parent, so no superclass resolution
        assert!(
            locations.is_empty(),
            "UseParent is unique to ChildDefault, no superclass def"
        );

        // Verify inheritance is recorded (left parent first)
        let class = project_data
            .global_semantic_model
            .get_class(child_class_id)
            .expect("class should exist");
        assert!(
            !class.inherited_classes.is_empty(),
            "should have inherited classes"
        );
    }

    #[tokio::test]
    async fn test_goto_def_multiple_inheritance_right_direction() {
        let project_root = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("gotodef")
            .join("multiple-inheritance");
        let (backend, uri) = setup_backend_and_workspace(project_root.clone()).await;
        let project_state = backend.get_project(&uri).expect("missing project state");
        let project_data = project_state.data.read();

        let child_right_id = project_data
            .classes
            .get("Demo.ChildRight")
            .expect("Demo.ChildRight should exist");

        let class = project_data
            .global_semantic_model
            .get_class(child_right_id)
            .expect("class should exist");
        assert_eq!(class.inheritance_direction, Some("right".to_string()));
        assert!(
            !class.inherited_classes.is_empty(),
            "should have inherited classes"
        );
    }

    // =========================================================================
    // GOTO IMPLEMENTATION — DEEP HIERARCHY (Test Suite 2.1)
    // =========================================================================

    #[tokio::test]
    async fn test_goto_implementation_deep_hierarchy_from_super() {
        let project_root = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("implementation");
        let (backend, uri) = setup_backend_and_workspace(project_root).await;
        let project_state = backend.get_project(&uri).expect("missing project state");
        let project_data = project_state.data.read();

        let method_ref = project_data
            .method_defs
            .get("Demo.DeepSuper")
            .and_then(|m| m.get("DeepMethod"))
            .expect("Demo.DeepSuper.DeepMethod should exist");

        let locations = project_data.get_method_overrides(method_ref);
        // DeepMid overrides DeepSuper, and DeepLeafOne/Two override DeepMid
        // Direct overrides of DeepSuper.DeepMethod is DeepMid
        assert!(
            !locations.is_empty(),
            "DeepSuper.DeepMethod should have at least one override"
        );
        let paths: HashSet<String> = locations
            .iter()
            .map(|(url, _)| url.path().to_string())
            .collect();
        assert!(
            paths.iter().any(|p| p.ends_with("deep-mid.cls")),
            "DeepMid should override DeepSuper.DeepMethod"
        );
    }

    #[tokio::test]
    async fn test_goto_implementation_deep_hierarchy_from_mid() {
        let project_root = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("implementation");
        let (backend, uri) = setup_backend_and_workspace(project_root).await;
        let project_state = backend.get_project(&uri).expect("missing project state");
        let project_data = project_state.data.read();
        eprintln!("PROJECT DATA OVERRIDES {:#?}", project_data.override_index);
        let method_ref = project_data
            .method_defs
            .get("Demo.DeepMid")
            .and_then(|m| m.get("DeepMethod"))
            .expect("Demo.DeepMid.DeepMethod should exist");

        let locations = project_data.get_method_overrides(method_ref);
        assert!(
            !locations.is_empty(),
            "DeepMid.DeepMethod should have overrides in leaf classes"
        );
        let paths: HashSet<String> = locations
            .iter()
            .map(|(url, _)| url.path().to_string())
            .collect();
        assert!(paths.iter().any(|p| p.ends_with("deep-leaf-one.cls")));
        assert!(paths.iter().any(|p| p.ends_with("deep-leaf-two.cls")));
    }

    #[tokio::test]
    async fn test_goto_implementation_no_overrides_returns_empty() {
        let project_root = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("implementation");
        let (backend, uri) = setup_backend_and_workspace(project_root).await;
        let project_state = backend.get_project(&uri).expect("missing project state");
        let project_data = project_state.data.read();

        let method_ref = project_data
            .method_defs
            .get("Demo.NoOverrides")
            .and_then(|m| m.get("Unique"))
            .expect("Demo.NoOverrides.Unique should exist");

        let locations = project_data.get_method_overrides(method_ref);
        assert!(
            locations.is_empty(),
            "method with no subclasses should have no overrides"
        );
    }

    #[tokio::test]
    async fn test_goto_implementation_class_subclasses() {
        let project_root = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("implementation");
        let (backend, uri) = setup_backend_and_workspace(project_root).await;
        let project_state = backend.get_project(&uri).expect("missing project state");
        let project_data = project_state.data.read();

        let class_id = project_data
            .classes
            .get("Demo.DeepSuper")
            .expect("Demo.DeepSuper should exist");

        let locations = project_data.get_class_implementations(class_id);
        assert!(
            !locations.is_empty(),
            "Demo.DeepSuper should have subclass implementations"
        );
        let paths: HashSet<String> = locations
            .iter()
            .map(|(url, _)| url.path().to_string())
            .collect();
        assert!(paths.iter().any(|p| p.ends_with("deep-mid.cls")));
    }

    #[tokio::test]
    async fn test_goto_implementation_class_with_no_subclasses() {
        let project_root = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("implementation");
        let (backend, uri) = setup_backend_and_workspace(project_root).await;
        let project_state = backend.get_project(&uri).expect("missing project state");
        let project_data = project_state.data.read();

        let class_id = project_data
            .classes
            .get("Demo.NoOverrides")
            .expect("Demo.NoOverrides should exist");

        let locations = project_data.get_class_implementations(class_id);
        assert!(
            locations.is_empty(),
            "class with no subclasses should return no implementations"
        );
    }

    // =========================================================================
    // DIAGNOSTICS (Test Suite 3.1, 3.2)
    // =========================================================================

    #[test]
    fn test_diagnostics_clean_cls_has_no_errors() {
        use objectscript_core::common::collect_error_nodes;
        use tree_sitter_objectscript::LANGUAGE_OBJECTSCRIPT_UDL;

        let content = std::fs::read_to_string(
            env::current_dir()
                .unwrap()
                .join("objectscript-tests")
                .join("diagnostics")
                .join("clean.cls"),
        )
        .unwrap();

        let mut parser = Parser::new();
        parser
            .set_language(&LANGUAGE_OBJECTSCRIPT_UDL.into())
            .unwrap();
        let tree = parser.parse(&content, None).unwrap();
        let errors = collect_error_nodes(tree.root_node());
        assert!(errors.is_empty(), "clean .cls should have no parse errors");
    }

    #[test]
    fn test_diagnostics_syntax_error_cls_has_errors() {
        use objectscript_core::common::collect_error_nodes;
        use tree_sitter_objectscript::LANGUAGE_OBJECTSCRIPT_UDL;

        let content = std::fs::read_to_string(
            env::current_dir()
                .unwrap()
                .join("objectscript-tests")
                .join("diagnostics")
                .join("syntax-error.cls"),
        )
        .unwrap();

        let mut parser = Parser::new();
        parser
            .set_language(&LANGUAGE_OBJECTSCRIPT_UDL.into())
            .unwrap();
        let tree = parser.parse(&content, None).unwrap();
        let errors = collect_error_nodes(tree.root_node());
        assert!(
            !errors.is_empty(),
            "syntax-error.cls should produce parse errors"
        );
    }

    #[test]
    fn test_diagnostics_clean_routine_has_no_errors() {
        let content = std::fs::read_to_string(
            env::current_dir()
                .unwrap()
                .join("objectscript-tests")
                .join("diagnostics")
                .join("clean.mac"),
        )
        .unwrap();

        let tree = parse_routine(&content);
        let errors = objectscript_core::common::collect_error_nodes(tree.root_node());
        assert!(errors.is_empty(), "clean .mac should have no parse errors");
    }

    #[test]
    fn test_diagnostics_multiple_errors_routine() {
        let content = std::fs::read_to_string(
            env::current_dir()
                .unwrap()
                .join("objectscript-tests")
                .join("diagnostics")
                .join("multiple-errors.mac"),
        )
        .unwrap();

        let tree = parse_routine(&content);
        let errors = objectscript_core::common::collect_error_nodes(tree.root_node());
        assert!(
            errors.len() >= 2,
            "multiple-errors.mac should have at least 2 parse errors, got {}",
            errors.len()
        );
    }

    #[test]
    fn test_diagnostics_xml_injected_clean_has_no_errors() {
        use objectscript_core::common::{
            collect_error_nodes, xml_objectscript_implementation_ranges,
        };
        use tree_sitter_objectscript_playground::LANGUAGE_OBJECTSCRIPT;

        let content = std::fs::read_to_string(
            env::current_dir()
                .unwrap()
                .join("objectscript-tests")
                .join("diagnostics")
                .join("injected-clean.xml"),
        )
        .unwrap();

        // Use ProjectState to parse XML (it owns the XML parser)
        let state = ProjectState::new();
        let uri = Url::parse("file:///tmp/injected-clean.xml").unwrap();
        state.handle_document_opened(uri.clone(), content.clone(), FileType::Xml, 1);

        let data = state.data.read();
        let doc = data.documents.get(&uri).expect("xml doc should be tracked");
        let xml_tree = &doc.tree;

        let ranges = xml_objectscript_implementation_ranges(xml_tree.root_node(), &content);
        let mut total_errors = 0;
        for range in ranges {
            let text = &content[range.start_byte..range.end_byte];
            if text.trim().is_empty() {
                continue;
            }
            let mut os_parser = Parser::new();
            os_parser
                .set_language(&LANGUAGE_OBJECTSCRIPT.into())
                .unwrap();
            os_parser.set_included_ranges(&[range]).ok();
            if let Some(tree) = os_parser.parse(&content, None) {
                total_errors += collect_error_nodes(tree.root_node()).len();
            }
        }
        assert_eq!(
            total_errors, 0,
            "injected-clean.xml should have no ObjectScript errors"
        );
    }

    #[test]
    fn test_diagnostics_xml_injected_error_has_errors() {
        use objectscript_core::common::{
            collect_error_nodes, xml_objectscript_implementation_ranges,
        };
        use tree_sitter_objectscript_playground::LANGUAGE_OBJECTSCRIPT;

        let content = std::fs::read_to_string(
            env::current_dir()
                .unwrap()
                .join("objectscript-tests")
                .join("diagnostics")
                .join("injected-error.xml"),
        )
        .unwrap();

        let state = ProjectState::new();
        let uri = Url::parse("file:///tmp/injected-error.xml").unwrap();
        state.handle_document_opened(uri.clone(), content.clone(), FileType::Xml, 1);

        let data = state.data.read();
        let doc = data.documents.get(&uri).expect("xml doc should be tracked");
        let xml_tree = &doc.tree;

        let ranges = xml_objectscript_implementation_ranges(xml_tree.root_node(), &content);
        let mut total_errors = 0;
        for range in ranges {
            let text = &content[range.start_byte..range.end_byte];
            if text.trim().is_empty() {
                continue;
            }
            let mut os_parser = Parser::new();
            os_parser
                .set_language(&LANGUAGE_OBJECTSCRIPT.into())
                .unwrap();
            os_parser.set_included_ranges(&[range]).ok();
            if let Some(tree) = os_parser.parse(&content, None) {
                total_errors += collect_error_nodes(tree.root_node()).len();
            }
        }
        assert!(
            total_errors > 0,
            "injected-error.xml should have ObjectScript errors in CDATA"
        );
    }

    // =========================================================================
    // ORDERING / TIMING (Test Suite 6.1)
    // =========================================================================

    #[tokio::test]
    async fn test_ordering_child_opened_before_parent() {
        let project_root = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("ordering");

        let state = ProjectState::new();
        state.project_root_path.set(Some(project_root.clone())).ok();
        let backend = BackendTester::new();
        let uri = Url::from_file_path(project_root.clone()).unwrap();
        backend.add_project(uri.clone(), state);
        let project_state = backend.get_project(&uri).expect("missing project state");

        let child_url = Url::from_file_path(project_root.join("child.cls")).unwrap();
        let parent_url = Url::from_file_path(project_root.join("parent.cls")).unwrap();

        // Open child FIRST (before parent is known)
        let child_content = std::fs::read_to_string(project_root.join("child.cls")).unwrap();
        project_state.handle_document_opened(child_url.clone(), child_content, FileType::Cls, 1);

        // Now open parent
        let parent_content = std::fs::read_to_string(project_root.join("parent.cls")).unwrap();
        project_state.handle_document_opened(parent_url.clone(), parent_content, FileType::Cls, 1);

        let project_data = project_state.data.read();

        let child_class_id = project_data
            .classes
            .get("Demo.OrderChild")
            .expect("Demo.OrderChild should be indexed");

        // goto def on Greet in child should resolve to parent's Greet
        let locations = project_data.get_method_superclass("Greet".to_string(), child_class_id);
        assert!(
            !locations.is_empty(),
            "child opened before parent: goto def should still resolve after parent loads"
        );
        assert!(locations[0].0.path().ends_with("parent.cls"));
    }

    #[tokio::test]
    async fn test_ordering_parent_opened_before_child() {
        let project_root = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("ordering");

        let state = ProjectState::new();
        state.project_root_path.set(Some(project_root.clone())).ok();
        let backend = BackendTester::new();
        let uri = Url::from_file_path(project_root.clone()).unwrap();
        backend.add_project(uri.clone(), state);
        let project_state = backend.get_project(&uri).expect("missing project state");

        let child_url = Url::from_file_path(project_root.join("child.cls")).unwrap();
        let parent_url = Url::from_file_path(project_root.join("parent.cls")).unwrap();

        // Open parent FIRST
        let parent_content = std::fs::read_to_string(project_root.join("parent.cls")).unwrap();
        project_state.handle_document_opened(parent_url.clone(), parent_content, FileType::Cls, 1);

        // Then child
        let child_content = std::fs::read_to_string(project_root.join("child.cls")).unwrap();
        project_state.handle_document_opened(child_url.clone(), child_content, FileType::Cls, 1);

        let project_data = project_state.data.read();

        let child_class_id = project_data
            .classes
            .get("Demo.OrderChild")
            .expect("Demo.OrderChild should be indexed");

        let locations = project_data.get_method_superclass("Greet".to_string(), child_class_id);
        assert!(
            !locations.is_empty(),
            "parent opened before child: goto def should resolve"
        );
        assert!(locations[0].0.path().ends_with("parent.cls"));
    }

    #[tokio::test]
    async fn test_ordering_missing_class_reference_returns_empty() {
        let project_root = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("ordering");
        let (backend, uri) = setup_backend_and_workspace(project_root).await;
        let project_state = backend.get_project(&uri).expect("missing project state");
        let project_data = project_state.data.read();

        // Reference a class that doesn't exist in the workspace
        let locations = project_data.get_class_definition("NonExistent.Class");
        assert!(
            locations.is_empty(),
            "referencing a class not in workspace should return empty, not crash"
        );
    }

    #[tokio::test]
    async fn test_ordering_duplicate_open_is_idempotent() {
        let project_root = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("ordering");

        let state = ProjectState::new();
        state.project_root_path.set(Some(project_root.clone())).ok();
        let backend = BackendTester::new();
        let uri = Url::from_file_path(project_root.clone()).unwrap();
        backend.add_project(uri.clone(), state);
        let project_state = backend.get_project(&uri).expect("missing project state");

        let parent_url = Url::from_file_path(project_root.join("parent.cls")).unwrap();
        let parent_content = std::fs::read_to_string(project_root.join("parent.cls")).unwrap();

        // Open same file twice
        project_state.handle_document_opened(
            parent_url.clone(),
            parent_content.clone(),
            FileType::Cls,
            1,
        );
        project_state.handle_document_opened(parent_url.clone(), parent_content, FileType::Cls, 2);

        let project_data = project_state.data.read();
        // Should still have exactly 1 class (not duplicated)
        assert!(
            project_data.classes.contains_key("Demo.OrderParent"),
            "class should still be indexed after duplicate open"
        );
        let doc = project_data
            .documents
            .get(&parent_url)
            .expect("document should exist");
        assert_eq!(doc.version, Some(2), "version should be updated to latest");
    }

    // =========================================================================
    // GOTO DEFINITION — EDGE CASES (Test Suite 1.8)
    // =========================================================================

    #[tokio::test]
    async fn test_goto_def_undefined_symbol_returns_empty() {
        let project_root = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("gotodef")
            .join("class-method-call");
        let (backend, uri) = setup_backend_and_workspace(project_root.clone()).await;
        let project_state = backend.get_project(&uri).expect("missing project state");
        let project_data = project_state.data.read();

        let document_url = Url::from_file_path(project_root.join("caller.cls")).unwrap();

        // Try to resolve a variable that was never defined
        let point = Point { row: 4, column: 10 };
        let locations = project_data.get_variable_definition(
            &document_url,
            point,
            "nonexistent_var".to_string(),
        );
        assert!(
            locations.is_empty(),
            "undefined variable should return empty, not crash"
        );
    }

    #[tokio::test]
    async fn test_goto_def_nonexistent_class_returns_empty() {
        let project_root = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("gotodef")
            .join("class-method-call");
        let (backend, uri) = setup_backend_and_workspace(project_root).await;
        let project_state = backend.get_project(&uri).expect("missing project state");
        let project_data = project_state.data.read();

        let locations = project_data.get_class_definition("Does.Not.Exist");
        assert!(
            locations.is_empty(),
            "class not in workspace should return empty"
        );
    }

    // =========================================================================
    // DOCUMENT SYNC — didOpen behavior (Test Suite 5.1)
    // =========================================================================

    #[test]
    fn test_did_open_cls_populates_class_id_and_name() {
        let state = ProjectState::new();
        let uri = Url::parse("file:///tmp/Demo.Hello.cls").unwrap();
        let content = "Class Demo.Hello\n{\nClassMethod Run()\n{\n    Write \"hi\"\n}\n}\n";
        state.handle_document_opened(uri.clone(), content.to_string(), FileType::Cls, 1);

        let data = state.data.read();
        let doc = data
            .documents
            .get(&uri)
            .expect("document should be tracked");
        assert_eq!(doc.file_type, FileType::Cls);
        assert!(doc.class_id.is_some(), "class_id should be set for .cls");
        assert_eq!(&doc.class_name, "Demo.Hello");
    }

    #[test]
    fn test_did_open_routine_populates_class_name_as_routine() {
        let state = ProjectState::new();
        let uri = Url::parse("file:///tmp/mytest.mac").unwrap();
        let content = "ROUTINE mytest\n\nmain\n set x = 1\n quit\n";
        state.handle_document_opened(uri.clone(), content.to_string(), FileType::Routine, 1);

        let data = state.data.read();
        let doc = data
            .documents
            .get(&uri)
            .expect("document should be tracked");
        assert_eq!(doc.file_type, FileType::Routine);
        assert!(
            doc.class_id.is_some(),
            "class_id should be set for routines"
        );
        assert_eq!(&doc.class_name, "mytest");
    }

    #[test]
    fn test_did_open_xml_no_class_id() {
        let state = ProjectState::new();
        let uri = Url::parse("file:///tmp/export.xml").unwrap();
        let content = r#"<?xml version="1.0" encoding="UTF-8"?>
<Export generator="IRIS" version="26">
<Class name="Demo.Xml">
<Method name="Test">
<Implementation><![CDATA[
 set x = 1
]]></Implementation>
</Method>
</Class>
</Export>"#;
        state.handle_document_opened(uri.clone(), content.to_string(), FileType::Xml, 1);

        let data = state.data.read();
        let doc = data
            .documents
            .get(&uri)
            .expect("document should be tracked");
        assert_eq!(doc.file_type, FileType::Xml);
        assert!(doc.class_id.is_none(), "XML docs should not have class_id");
        assert!(
            &doc.class_name == "XML",
            "XML docs should not have class_name"
        );
    }

    // =========================================================================
    // DOCUMENT SYNC — update_document (Test Suite 5.2)
    // =========================================================================

    #[tokio::test]
    async fn test_update_document_rebuilds_semantics() {
        let project_root = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("ordering");
        let (backend, uri) = setup_backend_and_workspace(project_root.clone()).await;
        let project_state = backend.get_project(&uri).expect("missing project state");

        let parent_url = Url::from_file_path(project_root.join("parent.cls")).unwrap();

        // Get the current tree, then simulate an update
        let (file_type, content, version, tree) = {
            let data = project_state.data.read();
            data.get_document_info(&parent_url).unwrap()
        };

        project_state.update_document(
            parent_url.clone(),
            &tree,
            file_type,
            version + 1,
            content.as_str(),
            vec![],
        );

        // Verify the document is still consistent
        let data = project_state.data.read();
        let doc = data.documents.get(&parent_url).unwrap();
        assert_eq!(doc.version, Some(version + 1));
        assert!(data.classes.contains_key("Demo.OrderParent"));
        assert!(data.method_defs.contains_key("Demo.OrderParent"));
    }

    // =========================================================================
    // REFACTORING (Test Suite 4.1, 4.2)
    // =========================================================================

    #[tokio::test]
    async fn test_refactor_no_changes_returns_none() {
        let state = ProjectState::new();
        let uri = Url::parse("file:///tmp/modern.mac").unwrap();
        let content = "ROUTINE modern\n\nmain\n    set x = 1\n    quit\n";
        state.handle_document_opened(uri.clone(), content.to_string(), FileType::Routine, 1);

        let data = state.data.read();
        let result = data.refactor_document(&uri, RefactorLevel::DoCommands);
        assert!(
            result.is_none(),
            "document with no legacy syntax should return None"
        );
    }

    #[tokio::test]
    async fn test_refactor_workspace_excludes_xml() {
        let project_root = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("diagnostics");
        let (backend, uri) = setup_backend_and_workspace(project_root).await;
        let project_state = backend.get_project(&uri).expect("missing project state");
        let project_data = project_state.data.read();

        let changes = project_data.refactor(RefactorLevel::All);
        for (_, url) in &changes {
            assert!(
                !url.path().ends_with(".xml"),
                "XML files should not be included in workspace refactor results"
            );
        }
    }

    #[tokio::test]
    async fn test_same_scope_last_definition_wins() {
        let project_root = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("variables");
        let document_url = Url::from_file_path(
            env::current_dir()
                .unwrap()
                .join("objectscript-tests")
                .join("variables")
                .join("scope-resolution.cls"),
        )
        .unwrap();
        let (backend, uri) = setup_backend_and_workspace(project_root).await;
        let project_state = backend.get_project(&uri).unwrap();
        let project_data = project_state.data.read();

        let document = project_data
            .documents
            .get(&document_url)
            .expect("missing scope-resolution document");
        let content = document.content.as_str();

        // In sameScope: two `set z` in the same scope, reference `w z` after both.
        // The last definition before the reference should win.
        let z_use_point = point_for_substring(content, "w z");
        let z_def_point = point_for_substring(content, "set z = 20");
        let z_locations =
            project_data.get_variable_definition(&document_url, z_use_point, "z".to_string());
        assert_eq!(z_locations.len(), 1);
        assert_eq!(z_locations[0].0, document_url);
        assert_eq!(z_locations[0].1.start_point.row, z_def_point.row);
    }

    #[tokio::test]
    async fn test_conditional_creates_new_scope() {
        let project_root = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("variables");
        let document_url = Url::from_file_path(
            env::current_dir()
                .unwrap()
                .join("objectscript-tests")
                .join("variables")
                .join("scope-resolution.cls"),
        )
        .unwrap();
        let (backend, uri) = setup_backend_and_workspace(project_root).await;
        let project_state = backend.get_project(&uri).unwrap();
        let project_data = project_state.data.read();

        let document = project_data
            .documents
            .get(&document_url)
            .expect("missing scope-resolution document");
        let content = document.content.as_str();

        // In conditionalScope: set y=1, then set y=2 inside if block (child scope),
        // then set y=3 after if block (same scope as y=1), then w y.
        // Two results: one per scope (method scope picks last def = "set y = 3",
        // if scope picks "set y = 2"). Both are before the reference.
        let y_use_point = point_for_substring(content, "w y");
        let y_def_method_scope = point_for_substring(content, "set y = 3");
        let y_def_if_scope = point_for_substring(content, "set y = 2");
        let y_locations =
            project_data.get_variable_definition(&document_url, y_use_point, "y".to_string());
        assert_eq!(y_locations.len(), 2);
        let y_rows: Vec<usize> = y_locations.iter().map(|(_, r)| r.start_point.row).collect();
        assert!(y_rows.contains(&y_def_method_scope.row));
        assert!(y_rows.contains(&y_def_if_scope.row));
    }

    #[tokio::test]
    async fn test_public_variable_resolution_via_call_path() {
        let project_root = env::current_dir()
            .unwrap()
            .join("objectscript-tests")
            .join("variables");
        let document_url = Url::from_file_path(
            env::current_dir()
                .unwrap()
                .join("objectscript-tests")
                .join("variables")
                .join("scope-resolution.cls"),
        )
        .unwrap();
        let (backend, uri) = setup_backend_and_workspace(project_root).await;
        let project_state = backend.get_project(&uri).unwrap();
        let project_data = project_state.data.read();

        let document = project_data
            .documents
            .get(&document_url)
            .expect("missing scope-resolution document");
        let content = document.content.as_str();

        // m1 uses x but doesn't define it. m2 calls m1 and defines x. m3 calls m2 and defines x.
        // Resolving x in m1 should find the definition in m2 (closest ancestor in call path).
        let x_use_point = point_for_substring(content, "w x");
        let x_def_point = point_for_substring(content, "set x = 200");

        let m1_ref = *project_data
            .method_defs
            .get("ScopeResolution")
            .unwrap()
            .get("m1")
            .unwrap();
        let m2_ref = *project_data
            .method_defs
            .get("ScopeResolution")
            .unwrap()
            .get("m2")
            .unwrap();
        let m3_ref = *project_data
            .method_defs
            .get("ScopeResolution")
            .unwrap()
            .get("m3")
            .unwrap();
        eprintln!("m1_ref: {:?}", m1_ref);
        eprintln!("m2_ref: {:?}", m2_ref);
        eprintln!("m3_ref: {:?}", m3_ref);

        if let Some(&m1_node) = project_data.dependency_graph.get_node(m1_ref) {
            eprintln!("m1 node index: {:?}", m1_node);
            let ancestors = project_data.dependency_graph.all_ancestors(m1_node);
            eprintln!("all_ancestors of m1 count: {:?}", ancestors.len());
            for (mref, range, depth) in &ancestors {
                eprintln!(
                    "  ancestor: {:?}, range: {:?}, depth: {:?}",
                    mref, range, depth
                );
            }
        } else {
            eprintln!("m1 NOT in dependency graph!");
        }

        eprintln!(
            "graph edge count: {:?}",
            project_data.dependency_graph.graph.edge_count()
        );
        eprintln!(
            "graph node count: {:?}",
            project_data.dependency_graph.graph.node_count()
        );
        for edge in project_data.dependency_graph.graph.raw_edges() {
            eprintln!(
                "edge: {:?} -> {:?} (weight: {:?})",
                edge.source(),
                edge.target(),
                edge.weight
            );
        }

        eprintln!(
            "is_variable_public for m1/x: {:?}",
            project_data.is_variable_public(m1_ref, "x".to_string())
        );

        let x_locations =
            project_data.get_variable_definition(&document_url, x_use_point, "x".to_string());
        assert_eq!(x_locations.len(), 1);
        assert_eq!(x_locations[0].0, document_url);
        assert_eq!(x_locations[0].1.start_point.row, x_def_point.row);
    }
}
