# Zed ObjectScript

An [ObjectScript](https://docs.intersystems.com/latest/csp/docbook/DocBook.UI.Page.cls?KEY=GCOS_intro) extension for [Zed](https://zed.dev) to support development for the InterSystems IRIS product.

[![Conventional Commits](https://img.shields.io/badge/Conventional%20Commits-1.0.0-%23FE5196?logo=conventionalcommits6logoColor=white)](https://conventionalcommits.org)

# Introduction

This Zed extension uses the [tree-sitter-objectscript](https://github.com/intersystems/tree-sitter-objectscript) grammar to provide syntax highlighting and code injections for `.cls` files containing ObjectScript.  Since ObjectScripts supports a number of embedded languages, you should install grammars for the following languages otherwise you may see areas that appear to lack syntax coloring.

- SQL
- HTML
- Python
- JavaScript
- CSS
- XML
- Markdown

**NOTE**: The ObjectScript `.cls` syntax supports some sophisticated constructs and as such it can take 15-60 seconds for Zed's WASM machinery to build the parser before syntax coloring becomes available.

Currently this extension only provides syntax coloring support.

## Reporting Issues

Please report issues via [GitHub Issues](https://github.com/intersystems/zed-objectscript/issues).

## Contributing

Contributions are welcome. Please submit changes via Pull Requests. Our preference is to use [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/) for commit messages in order to keep the summaries terse, but allowing for more detail on the subsequent lines of the commit message.

### Development

To develop this extension, see the [Developing Extensions](https://zed.dev/docs/extensions/developing-extensions) section of the Zed docs.

#### Notes

To enable log output for Zed, set `RUST_LOG` as follows before starting `zed` from the command line:

```ps
$env:RUST_LOG = "language,extension=trace"
```

```bash
RUST_LOG = "language,extension=trace"
```

Cloning and the building a debug Zed with these `RUST_LOG` settings gives fairly detailed log output including diagnosing
bad `.scm` rules.

