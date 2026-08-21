# Zed ObjectScript

An [ObjectScript](https://docs.intersystems.com/latest/csp/docbook/DocBook.UI.Page.cls?KEY=GCOS_intro) extension for [Zed](https://zed.dev) to support development for the InterSystems IRIS product.

[![Conventional Commits](https://img.shields.io/badge/Conventional%20Commits-1.0.0-%23FE5196?logo=conventionalcommits6logoColor=white)](https://conventionalcommits.org)

# Introduction

This Zed extension uses the [tree-sitter-objectscript](https://github.com/intersystems/tree-sitter-objectscript) grammars and the `objectscript-lsp` crate (local to this repo) to provide syntax highlighting, code injections, and language support for `.cls`, `.mac`, `.rtn`, `.inc`, `.int` and `.xml` files containing ObjectScript.  Install the following extensions below to get syntax highlighting for any code injected into objectscript that is `sql`, `html`, or special comments (TODO, NOTE, etc). 

- [SQL](https://zed.dev/extensions/sql)
- [HTML](https://zed.dev/extensions/html)
- [Comment](https://zed.dev/extensions/comment)

The current features supported in the `ObjectScript language server` are `goto_definition`, `goto_implementation`, `refactor`, and `diagnostics`. These features are described in detail in the `objectscript-lsp/documentation/features` folder.

### Setup Dev Extension 
If you want the most up-to-date version of this extension (including experimental parts that have not yet been merged into [zed-industries/extensions](https://github.com/zed-industries/extensions), follow these steps:   
1. Make sure you have [rust](https://rust-lang.org/tools/install/) installed.  
2. Clone this repository into your local folders and cd into the objectscript-lsp folder (`cd zed-objectscript/objectscript-lsp`).  
3. Build a local copy of the binary: `cargo b && cargo install --path . --force`  
4. Go to Zed, and do `Cmd + Shift + P`, and then choose `zed:Extensions`. Then choose `Install Dev Extension` and point it at your local copy of this directory.
5. NOTE: If rebuilding, sometimes you have to quit out of Zed and re-enter it for the changes to show.

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
