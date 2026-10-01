use crate::common::{get_node_children, get_string_at_byte_range, get_tracked_keywords};
use crate::parse_structures::{CodeMode, Language, Trigger, TriggerFire, TriggerForEach};
use tree_sitter::Node;
impl Default for Trigger {
    fn default() -> Self {
        Self {
            name: "TODO".to_string(),
            code_mode: CodeMode::Code,
            is_final: None,
            language: Language::Objectscript,
            delete: false,
            update: false,
            insert: false,
            time: TriggerFire::BEFORE,
            for_each: TriggerForEach::Row,
        }
    }
}

pub fn build_trigger_struct(trigger_node: Node, content: &str) -> Option<Trigger> {
    if trigger_node.kind() != "trigger" {
        eprintln!(
            "Error: build_trigger_struct was called for node {:?}, but it can only be called for trigger nodes",
            trigger_node.kind()
        );
        return None;
    }
    let mut trigger = Trigger::default();
    let trigger_children = get_node_children(trigger_node);
    for trigger_child in trigger_children {
        match trigger_child.kind() {
            "trigger_name" => {
                if let Some(trigger_name_node) = trigger_child.named_child(0)
                    && let Some(name) =
                        get_string_at_byte_range(content, trigger_name_node.byte_range())
                {
                    trigger.name = name;
                } else {
                    eprintln!("Error: failed to get trigger name.");
                    return None;
                }
            }
            "trigger_keywords" | "external_trigger_keywords" => {
                let tracked_keywords = get_tracked_keywords(trigger_child, content);
                trigger.update = tracked_keywords.trigger_update;
                trigger.delete = tracked_keywords.trigger_delete;
                trigger.insert = tracked_keywords.trigger_insert;
                trigger.for_each = tracked_keywords.trigger_for_each;
                trigger.is_final = tracked_keywords.is_final;
                trigger.language = tracked_keywords.language.unwrap_or(Language::Objectscript);
                trigger.time = tracked_keywords.trigger_time;
                trigger.code_mode = tracked_keywords.code_mode;
            }
            _ => {
                // eprintln!(
                //     "Error: Unrecognized trigger child node {:?}",
                //     trigger_child.kind()
                // );
                continue;
            }
        }
    }
    if &trigger.name == "TODO" {
        eprintln!("error: failed to parse trigger name");
        return None;
    }
    Some(trigger)
}
