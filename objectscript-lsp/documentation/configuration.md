# Configuration

ObjectScript LSP accepts configuration from the LSP client in two places:

- `initialize.initializationOptions`, for startup configuration
- `workspace/didChangeConfiguration`, for runtime configuration updates
- `workspace/configuration`, when the client supports server-initiated configuration requests

The canonical Rust field is `enable_strict_mode`. The canonical JSON setting is `enableStrictMode`.

```json
{
  "enableStrictMode": true
}
```

The server also accepts `enable_strict_mode`, `strictMode`, `strict_mode`, and `STRICT_MODE` as compatibility aliases.

## Settings

| Rust field | JSON key | Default | Meaning |
|---|---|---:|---|
| `enable_snippets` | `enableSnippets` | `true` | Enable snippet-style completion items. |
| `enable_formatting` | `enableFormatting` | `true` | Advertise document formatting support during initialize. |
| `enable_lint` | `enableLint` | `true` | Enable diagnostics. When `false`, diagnostic requests return no diagnostics. |
| `enable_strict_mode` | `enableStrictMode` | `true` | Include all available diagnostics. When `false`, diagnostics are limited to syntax and XML-injected ObjectScript syntax checks. |

`enableFormatting` is negotiated during `initialize`, so changing it at runtime does not currently re-register formatting capabilities. Diagnostic settings take effect through `workspace/didChangeConfiguration`.

Changing an editor `settings.json` file only affects the language server if the active editor extension forwards that setting to the LSP process. The server accepts direct ObjectScript settings, an `objectscript` wrapper, flat VS Code-style keys, and Zed-style `lsp.objectscript-lsp.initialization_options`.

## Runtime Update Behavior

At startup, the server reads `initialize.initializationOptions` and stores the parsed config in each workspace's `ProjectData.config`.

After startup, `workspace/didChangeConfiguration` updates the same in-memory config. If the client supports `workspace/configuration`, the server first requests current settings from the client and uses that response. If the client does not support `workspace/configuration`, the server falls back to the `settings` payload included in the `didChangeConfiguration` notification.

Runtime updates are only applied when the payload contains ObjectScript config keys such as `enableStrictMode`, `enableLint`, or `objectscript.enableStrictMode`. Empty settings payloads and unrelated LSP settings are ignored. This prevents an editor notification with no ObjectScript settings from resetting `enableStrictMode` to its default value of `true`.

When `enableStrictMode` changes, the server refreshes workspace diagnostics if the client supports diagnostic refresh. A semantic diagnostic such as `"Method referenced has either not yet been indexed or does not exist"` is filtered out when `enableStrictMode` is `false`.

## Zed

Set startup and runtime options in Zed `settings.json` under the language server id used by the extension:

```json
{
  "lsp": {
    "objectscript-lsp": {
      "initialization_options": {
        "enableStrictMode": false
      },
      "settings": {
        "enableStrictMode": false
      }
    }
  }
}
```

`initialization_options` covers server startup. `settings` covers runtime `workspace/configuration` / `workspace/didChangeConfiguration` updates.

The server also accepts a runtime-only Zed shape:

```json
{
  "lsp": {
    "objectscript-lsp": {
      "settings": {
        "enableStrictMode": false
      }
    }
  }
}
```

If the Zed extension sends runtime settings, use the same JSON shape in the extension code and send it through `workspace/didChangeConfiguration`:

```json
{
  "objectscript": {
    "enableStrictMode": false
  }
}
```

## Neovim

For startup configuration, pass `init_options`:

```lua
vim.lsp.config('objectscript_lsp', {
  cmd = { '/path/to/objectscript-lsp' },
  filetypes = { 'objectscript', 'objectscript-class', 'objectscript-routine' },
  root_markers = { '.git' },
  init_options = {
    enableStrictMode = false,
  },
})

vim.lsp.enable('objectscript_lsp')
```

For runtime updates, send `workspace/didChangeConfiguration` with `settings`:

```lua
for _, client in ipairs(vim.lsp.get_clients({ name = 'objectscript_lsp' })) do
  client.notify('workspace/didChangeConfiguration', {
    settings = {
      objectscript = {
        enableStrictMode = false,
      },
    },
  })
end
```

## VS Code

Expose a user-facing setting from the VS Code extension `package.json`:

```json
{
  "contributes": {
    "configuration": {
      "title": "ObjectScript",
      "properties": {
        "objectscript.enableStrictMode": {
          "type": "boolean",
          "default": true,
          "scope": "resource",
          "description": "Enable all ObjectScript diagnostics."
        }
      }
    }
  }
}
```

Users set it in VS Code `settings.json`:

```json
{
  "objectscript.enableStrictMode": false
}
```

The VS Code extension should pass the setting at startup:

```ts
const objectscriptConfig = vscode.workspace.getConfiguration("objectscript");

const clientOptions: LanguageClientOptions = {
  documentSelector: [{ scheme: "file", language: "objectscript" }],
  initializationOptions: {
    enableStrictMode: objectscriptConfig.get("enableStrictMode", true),
  },
};
```

To apply changes without restarting the language server, listen for VS Code configuration changes and send `workspace/didChangeConfiguration`:

```ts
context.subscriptions.push(
  vscode.workspace.onDidChangeConfiguration((event) => {
    if (!event.affectsConfiguration("objectscript")) {
      return;
    }

    const objectscriptConfig = vscode.workspace.getConfiguration("objectscript");
    client.sendNotification("workspace/didChangeConfiguration", {
      settings: {
        objectscript: {
          enableStrictMode: objectscriptConfig.get("enableStrictMode", true),
        },
      },
    });
  }),
);
```
