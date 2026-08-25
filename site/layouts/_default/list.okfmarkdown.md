{{- /*
  The byte-identical source of this page, served beside its HTML.

  `os.ReadFile .Filename` rather than `.RawContent`, because `.RawContent`
  strips front matter and would lose `type`, the one field OKF requires. Every
  build `cmp`s each pair, so a change here fails loudly rather than quietly
  handing an agent a lossy copy.
*/ -}}
{{- with .File -}}{{ os.ReadFile .Filename }}{{- else -}}{{ $.RawContent }}{{- end -}}
