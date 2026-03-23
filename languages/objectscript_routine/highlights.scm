(pattern_expression) @string.regex

[
  (json_number_literal)
  (numeric_literal)
] @number

[
  (json_boolean_literal)
  (json_null_literal)
] @boolean

(json_object_literal_pair
  (json_string_literal) @string.special)

[
  (json_string_literal)
  (string_literal)
] @string

(string_literal) @string

[
  (keyword_pound_pound_super)
  (keyword_pound_pound_class)
] @keyword

(system_defined_variable) @variable.special
(system_defined_function) @function.builtin
(sql_field_modifier) @keyword

[
  (property_name)
  (parameter_name)
  (sql_field_identifier)
] @variant

(method_name) @function
[
  (routine_name)
  (class_name)
] @type

[
  (macro_function)
  (macro_constant)
] @constant

[
  (lvn)
  (gvn)
  (ssvn)
  (objectscript_identifier)
] @variable

(namespace) @namespace

[
  (objectscript_identifier_special)
  (instance_variable)
] @variant

(method_arg) @variable.parameter
; I didn't include ( or ) in this, because they are often grouped 
; as part of a sequence that gets turned into a single token, so they 
; don't get matched, and one ends up getting colored differently than the other.
[
  "_"
  ","
  ":"
  ".."
  "..."
  "'["
  "']"
  "']]"
  "\""
  "\"\""
  "["
  "]"
  "]]"
  "{"
  "}"
  "/"
  "\\"
  "#"
  "|"
  "||"
  "$$"
] @punctuation.delimiter

[
  "'&"
  "&"
  "&&"
  "'<"
  "'="
  "'>"
  "^"
  "-"
  "^$"
  "+"
  "<"
  "<="
  "="
  ">"
  ">="
  "@"
  "*"
  "**"
  "'"
  "'!"
  "'?"
  "!"
  "?"
] @operator

(bracket) @punctuation.bracket

; core
(macro_arg) @variant
(macro_value) @constant.builtin
(macro_def) @preproc

[
  (keyword_for)
  (keyword_while)
  (keyword_continue)
  (keyword_quit)
  (keyword_throw)
  (keyword_if)
  (keyword_elseif)
  (keyword_else)
  (keyword_oldelse)
  (keyword_try)
  (keyword_catch)
  (keyword_break)
  (keyword_return)
  (keyword_zbreak)
  (keyword_debug)
  (keyword_trace)
  (keyword_step)
  (keyword_nostep)
  (keyword_stepmethod)
  (keyword_errortrap)
  (keyword_interrupt)
  (keyword_normal)
  (keyword_zkill)
  (keyword_zn)
  (keyword_zsu)
  (keyword_ztrap)
  (keyword_zz)
  (keyword_print)
  (keyword_zprint)
  (keyword_set)
  (keyword_write)
  (keyword_zwrite)
  (keyword_do)
  (keyword_for)
  (keyword_while)
  (keyword_kill)
  (keyword_lock)
  (keyword_read)
  (keyword_open)
  (keyword_close)
  (keyword_use)
  (keyword_new)
  (keyword_job)
  (keyword_merge)
  (keyword_goto)
  (keyword_halt_or_hang)
  (keyword_halt)
  (keyword_hang)
  (keyword_tcommit)
  (keyword_trollback)
  (keyword_tstart)
  (keyword_xecute)
  (keyword_view)
] @type.builtin

[
  (keyword_pound_define)
  (keyword_pound_def1arg)
  (keyword_pound_if)
  (keyword_pound_elseif)
  (keyword_pound_else)
  (keyword_pound_endif)
  (keyword_pound_ifdef)
  (keyword_pound_ifndef)
  (keyword_dim)
  (keyword_pound_import)
  (keyword_pound_include)
  (keyword_pound_delay)
  (locktype)
] @preproc

[
  (keyword_as)
  (keyword_of)
  (keyword_public)
  (keyword_private)
  (keyword_methodimpl)
  (open_keywords)
  (use_keywords)
  (close_parameter_option_value)
  (keyword_clear)
  (keyword_on)
  (keyword_off)
  (keyword_all)
  (keyword_ext)
  (keyword_stepmethod)
  (keyword_destruct)
] @keyword

[
  (keyword_embedded_html)
  (keyword_embedded_xml)
  (keyword_embedded_sql_amp)
  (keyword_embedded_sql_hash)
  (keyword_js)
] @embedded

[
  (embedded_js_special_case_complete)
  (embedded_sql_marker)
  (embedded_sql_reverse_marker)
  (html_marker)
  (html_marker_reversed)
] @punctuation.special

[
  (line_comment_1)
  (line_comment_2)
  (line_comment_3)
  (line_comment_4)
  (block_comment)
] @comment

(tag) @label

; case where comments get eaten by scanner, this will still
; highlight just the comments of these commands as comments
[
  (command_quit)
  (command_else)
  (command_continue)
  (command_if)
  (command_do)
  (command_for)
  (command_lock)
  (command_return)
  (command_halt_or_hang)
  (command_break)
] @comment

;routine 
(routine_type) @type.builtin

(documatic_line) @comment.doc

(routine) @attribute