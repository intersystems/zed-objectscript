; Indentation rules for ObjectScript UDL (Zed)

; --- Class Definition ---
; Indent everything inside the class_body’s braces.
(class_body "{" @start "}" @end) @indent

; If individual class statements can wrap, indent their interior.
(class_body (class_statement) @indent)

; --- Command arguments that may wrap across lines ---
(command_write (keyword_write) (write_argument) @indent)
(command_set   (keyword_set)   (set_argument)   @indent)
(command_do    (keyword_do)    (do_parameter)   @indent)
(command_kill  (keyword_kill)  (kill_argument)  @indent)
(command_lock  (keyword_lock)  (command_lock_argument) @indent)
(command_read  (keyword_read)  (read_argument)  @indent)
(command_open  (keyword_open)  (open_parameter) @indent)
(command_close (keyword_close) (close_parameter) @indent)
(command_use   (keyword_use)   (use_parameter)  @indent)

; --- Block-style commands delimited by braces ---
(command_while "{" @start "}" @end) @indent
(command_for   "{" @start "}" @end) @indent
(command_if    "{" @start "}" @end) @indent
(elseif_block  "{" @start "}" @end) @indent
(else_block    "{" @start "}" @end) @indent

; --- Old-style FOR / IF / ELSE ---
(command_for (for_parameter) @indent)
(command_for (statement)     @indent)
(command_if  (expression)    @indent)
(command_if  (statement)     @indent)
(command_else (statement)    @indent)

; --- Generic fallbacks for braces/parentheses spanning multiple lines ---
(_ "{" "}" @end) @indent
(_ "(" ")" @end) @indent

