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
