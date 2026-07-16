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
    use tree_sitter::{Parser, Point};
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

        assert_eq!(before_classes.len(), 3);
        assert!(before_classes.contains_key("SuperClass"));
        assert!(before_classes.contains_key("SubClassOne"));
        assert!(before_classes.contains_key("SubClassTwo"));

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
        assert_eq!(before_y.len(), 2);
        assert_eq!(superclass_count, 2);

        project_state.update_document(document_url, tree, FileType::Cls, 1, content.as_str());

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

        assert_eq!(after_classes.len(), 3);
        assert!(after_classes.contains_key("SuperClass"));
        assert!(after_classes.contains_key("SubClassOne"));
        assert!(after_classes.contains_key("SubClassTwo"));

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
        assert_eq!(after_y.len(), 2);
        assert_eq!(superclass_count, 2);
        let Some(sub_one_class_inherited) = gsm_classes.get(&sub_one_class_id) else {
            panic!("Error: subclass one DNE in classes");
        };
        let Some(sub_two_class_inherited) = gsm_classes.get(&sub_two_class_id) else {
            panic!("Error: subclass two DNE in classes");
        };

        assert_eq!(
            sub_one_class_inherited.inherited_classes,
            vec![super_class_id]
        );
        assert_eq!(
            sub_two_class_inherited.inherited_classes,
            vec![super_class_id]
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
        for (_class_name, class_id) in classes {
            let Some(class) = &gsm.classes.get(&class_id) else {
                panic!("Class DNE");
            };
            assert_eq!(class.is_procedure_block, Some(false));
            assert_eq!(class.default_language, Some(Language::Objectscript));
            assert_eq!(class.inheritance_direction, "right");
            // get methods
            for (_, method_ref) in class.methods.clone() {
                let method = gsm.methods.get(&method_ref).unwrap();
                if method.name == "newVarChange" {
                    assert_eq!(method.variables.len(), 1);
                    let variable_refs = method.variables.get("x").unwrap();
                    assert_eq!(variable_refs.len(), 1);
                    for variable_ref in variable_refs {
                        assert!(variable_ref.pub_id.is_none());
                        assert!(variable_ref.priv_id.is_some());
                    }
                    assert_eq!(method.is_procedure_block, Some(true));
                    assert_eq!(method.language, Some(Language::Objectscript));
                } else {
                    let all_var_refs: Vec<Vec<VariableRef>> =
                        method.variables.values().cloned().collect();
                    for variable_refs in all_var_refs {
                        for variable_ref in variable_refs {
                            assert!(variable_ref.pub_id.is_some());
                            assert!(variable_ref.priv_id.is_none());
                        }
                    }
                    assert_eq!(method.is_procedure_block, Some(false));
                    assert_eq!(method.language, Some(Language::Objectscript));
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

checkSubroutine1
    set x = 1.2
    set y = 1.2
    w !,"goodbye"
    do checkSubroutine2
    quit

checkSubroutine2
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
        // eprintln!("METHOD: {:#?}", method);
        // eprintln!("OVERRIDE INDEX: {:#?}", project_data.override_index);
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
                tree,
                file_type,
                version,
                sub_public_content.as_str(),
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
        let document = project_data
            .documents
            .get(&routine_url)
            .expect("routine doc exists");

        eprintln!("class_id: {:?}", document.class_id);
        eprintln!("class_name: {:?}", document.class_name);
        eprintln!("scope_tree: {:#?}", document.scope_tree);
        // "w x" is on line 8 (0-indexed row=7), x is at column 3
        let point = Point { row: 7, column: 3 };

        let method_name = document.scope_tree.get_method_name(point);
        eprintln!("method_name at point {:?}: {:?}", point, method_name);

        let locations = project_data.get_variable_definition(&routine_url, point, "x".to_string());
        eprintln!("variable locations: {:?}", locations);
        assert!(
            !locations.is_empty(),
            "should find x definition in gotosubroutine"
        );
    }
}
