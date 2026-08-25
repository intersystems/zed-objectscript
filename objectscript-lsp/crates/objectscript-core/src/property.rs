use crate::common::{find_return_type, get_keyword_and_value, get_string_at_byte_range};
use crate::parse_structures::{Property, TypeName};
use tree_sitter::{Language as TsLanguage, Node, Query, QueryCursor, StreamingIterator};
use tree_sitter_objectscript::LANGUAGE_OBJECTSCRIPT_UDL;
impl Property {
    pub fn new(name: String) -> Self {
        Self {
            name,
            required: false,
            is_public: true,
            multidimensional: false,
            return_type: None,
            is_final: Some(false),
        }
    }

    /// Given a property node, queries the keywords and assigns
    /// the return type and the keywords
    /// Returns (bool, bool) representing (is_public_changed, is_final_changed)
    pub fn build_keywords(
        &mut self,
        node: Node,
        content: &str,
        old_class_is_final: Option<bool>,
        class_is_final: Option<bool>,
    ) -> (bool, bool) {
        let query_str = r#"
            [(property_keyword) @keyword
            (return_type (typename (identifier) @returntype ))
            ]"#;
        let language: &TsLanguage = &LANGUAGE_OBJECTSCRIPT_UDL.into();
        let mut is_final_changed = false;
        let mut privacy_changed = false;
        if let Ok(query) = Query::new(language, query_str) {
            let mut cursor = QueryCursor::new();
            let mut iter = cursor.matches(&query, node, content.as_bytes());
            let keyword_idx = query.capture_index_for_name("keyword");
            let returntype_idx = query.capture_index_for_name("returntype");
            let old_is_public = self.is_public.clone();
            let old_is_final = self.is_final.clone();
            let mut return_type_parameters = Vec::new();
            let mut saw_first_return_type = false;
            let mut return_type_id = None;
            while let Some(query_match) = iter.next() {
                let mut i = 0;
                while i < query_match.captures.len() {
                    let capture = &query_match.captures[i];
                    if keyword_idx == Some(capture.index) {
                        if let Some(keyword_str) =
                            get_string_at_byte_range(content, capture.node.byte_range())
                        {
                            let (not, keyword_name, _) =
                                get_keyword_and_value(keyword_str.as_str());
                            if keyword_name == "final" {
                                if not {
                                    self.is_final = Some(false);
                                } else {
                                    self.is_final = Some(true);
                                }
                            } else if keyword_name == "private" {
                                if not {
                                    self.is_public = true;
                                } else {
                                    self.is_public = false;
                                }
                            } else if keyword_name == "required" {
                                if not {
                                    self.required = true;
                                } else {
                                    self.required = false;
                                }
                            } else if keyword_name == "multidimensional" {
                                if not {
                                    self.multidimensional = true;
                                } else {
                                    self.multidimensional = false;
                                }
                            }
                        }
                        i += 1;
                        continue;
                    } else if returntype_idx == Some(capture.index) {
                        let return_type_node = capture.node;
                        let Some(typename) =
                            get_string_at_byte_range(content, return_type_node.byte_range())
                        else {
                            i += 1;
                            continue;
                        };
                        if !saw_first_return_type {
                            return_type_id = Some(find_return_type(typename));
                            saw_first_return_type = true;
                        } else {
                            return_type_parameters.push(typename);
                        }
                        i += 1;
                        continue;
                    }
                }
            }
            if let Some(typename_id) = return_type_id {
                let typename = TypeName {
                    ret_type: typename_id,
                    parameters: return_type_parameters,
                };
                self.return_type = Some(typename);
            }
            let old_final_keyword_res = old_is_final.unwrap_or(old_class_is_final.unwrap_or(false));
            let new_final_keyword = self.is_final.unwrap_or(class_is_final.unwrap_or(false));
            if old_final_keyword_res != new_final_keyword {
                is_final_changed = true;
            }
            if old_is_public != self.is_public {
                privacy_changed = true;
            }
        }
        (is_final_changed, privacy_changed)
    }
}
