use std::fs;

use zed_extension_api::{self as zed, settings::LspSettings, LanguageServerId, Result};

struct ObjectScriptBinary {
    path: String,
    args: Option<Vec<String>>,
}

struct ObjectScriptExtension {
    cached_binary_path: Option<String>,
}

impl ObjectScriptExtension {
    fn language_server_binary(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<ObjectScriptBinary> {
        // Read user/worktree LSP settings for this language server id ("objectscript-lsp")
        let settings = LspSettings::for_worktree(language_server_id.as_ref(), worktree);
        // Optional user override: explicit binary path + args from settings
        let binary = settings.ok().and_then(|settings| settings.binary);
        let args = binary.as_ref().and_then(|binary| binary.arguments.clone());
        // Binary resolution order:
        // 1) settings.binary.path
        // 2) PATH lookup in the worktree environment
        // 3) Zed-managed download from GitHub releases
        let path = if let Some(path) = binary
            .and_then(|b| b.path)
            .or_else(|| worktree.which("objectscript-lsp"))
        {
            path
        } else {
            self.zed_managed_binary_path(language_server_id)?
        };
        Ok(ObjectScriptBinary { path, args })
    }

    fn zed_managed_binary_path(&mut self, language_server_id: &LanguageServerId) -> Result<String> {
        if let Some(path) = &self.cached_binary_path {
            if fs::metadata(path).is_ok_and(|m| m.is_file()) {
                return Ok(path.clone());
            }
        }

        zed::set_language_server_installation_status(
            language_server_id,
            &zed::LanguageServerInstallationStatus::CheckingForUpdate,
        );

        let release = zed::latest_github_release(
            "intersystems/zed-objectscript",
            zed::GithubReleaseOptions {
                require_assets: true,
                pre_release: false,
            },
        )?;

        let (platform, arch) = zed::current_platform();
        let asset_name = format!(
            "objectscript-lsp-{version}-{os}-{arch}.{extension}",
            version = release.version,
            os = match platform {
                zed::Os::Mac => "darwin",
                zed::Os::Linux => "linux",
                zed::Os::Windows => "win32",
            },
            arch = match arch {
                zed::Architecture::Aarch64 => "arm64",
                zed::Architecture::X8664 => "x64",
                zed::Architecture::X86 => return Err("unsupported platform x86".into()),
            },
            extension = match platform {
                zed::Os::Mac | zed::Os::Linux => "tar.gz",
                zed::Os::Windows => "zip",
            },
        );

        let asset = release
            .assets
            .iter()
            .find(|asset| asset.name == asset_name)
            .ok_or_else(|| format!("no asset found matching {asset_name:?}"))?;

        let version_dir = format!("objectscript-lsp-{}", release.version);
        let binary_path = format!(
            "{version_dir}/bin/objectscript-lsp{extension}",
            extension = match platform {
                zed::Os::Mac | zed::Os::Linux => "",
                zed::Os::Windows => ".exe",
            },
        );

        if !fs::metadata(&binary_path).is_ok_and(|stat| stat.is_file()) {
            zed::set_language_server_installation_status(
                language_server_id,
                &zed::LanguageServerInstallationStatus::Downloading,
            );

            zed::download_file(
                &asset.download_url,
                &version_dir,
                match platform {
                    zed::Os::Mac | zed::Os::Linux => zed::DownloadedFileType::GzipTar,
                    zed::Os::Windows => zed::DownloadedFileType::Zip,
                },
            )
            .map_err(|e| format!("failed to download file: {e}"))?;

            let entries =
                fs::read_dir(".").map_err(|e| format!("failed to list working directory {e}"))?;
            for entry in entries {
                let entry = entry.map_err(|e| format!("failed to load directory entry {e}"))?;
                if entry.file_name().to_str() != Some(&version_dir) {
                    fs::remove_dir_all(entry.path()).ok();
                }
            }
        }

        self.cached_binary_path = Some(binary_path.clone());
        Ok(binary_path)
    }
}

impl zed::Extension for ObjectScriptExtension {
    fn new() -> Self {
        Self {
            cached_binary_path: None,
        }
    }

    fn language_server_command(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        let objectscript_binary = self.language_server_binary(language_server_id, worktree)?;
        Ok(zed::Command {
            command: objectscript_binary.path,
            args: objectscript_binary.args.unwrap_or_else(std::vec::Vec::new),
            env: vec![],
        })
    }

    //  This forwards Config (enable_snippets / enable_formatting / enable_lint / enable_strict_mode)
    // from Zed settings into the LSP initialize request as `initializationOptions`.
    fn language_server_initialization_options(
        &mut self,
        server_id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<Option<zed::serde_json::Value>> {
        LspSettings::for_worktree(server_id.as_ref(), worktree)
            .map(|lsp_settings| lsp_settings.initialization_options.clone())
    }

    // a separate channel of configuration that is NOT initializationOptions.
    // Many servers use this for workspace/didChangeConfiguration.
    fn language_server_workspace_configuration(
        &mut self,
        server_id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<Option<zed::serde_json::Value>> {
        LspSettings::for_worktree(server_id.as_ref(), worktree)
            .map(|lsp_settings| lsp_settings.settings.clone())
    }
}

zed::register_extension!(ObjectScriptExtension);

// TODO : I might want to set label_for_completion and label_for_symbol
