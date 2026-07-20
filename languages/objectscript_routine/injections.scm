(embedded_html
  (angled_bracket_fenced_text) @injection.content
  (#set! injection.language "html"))

(embedded_sql
  (_
    (paren_fenced_text) @injection.content)
  (#set! injection.language "sql"))

(embedded_js
  [
    (angled_bracket_fenced_text)
    (embedded_js_special_case)
  ] @injection.content
  (#set! injection.language "javascript"))

(embedded_xml
  (angled_bracket_fenced_text) @injection.content
  (#set! injection.language "xml"))
