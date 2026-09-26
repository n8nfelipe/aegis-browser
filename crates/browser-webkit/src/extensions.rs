use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use webkit2gtk::{
    UserContentInjectedFrames, UserContentManager, UserContentManagerExt, UserScript,
    UserScriptInjectionTime, UserStyleLevel, UserStyleSheet,
};

const EXTENSIONS_DIR_ENV: &str = "AEGIS_EXTENSIONS_DIR";

#[derive(Debug, Clone)]
pub(crate) struct InstalledExtension {
    pub id: String,
    pub name: String,
    pub version: String,
    pub icon_path: Option<PathBuf>,
    pub popup_path: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct Manifest {
    name: String,
    version: String,
    #[serde(rename = "manifest_version")]
    manifest_version: u32,
    #[serde(default)]
    icons: HashMap<String, String>,
    #[serde(default)]
    action: Option<ExtensionAction>,
    #[serde(default)]
    browser_action: Option<ExtensionAction>,
    #[serde(default)]
    #[serde(rename = "content_scripts")]
    content_scripts: Vec<ContentScript>,
}

#[derive(Debug, Deserialize)]
struct ExtensionAction {
    #[serde(default, rename = "default_popup")]
    default_popup: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ContentScript {
    #[serde(default)]
    matches: Vec<String>,
    #[serde(default)]
    #[serde(rename = "exclude_matches")]
    exclude_matches: Vec<String>,
    #[serde(default)]
    js: Vec<String>,
    #[serde(default)]
    css: Vec<String>,
    #[serde(default = "default_run_at", rename = "run_at")]
    run_at: String,
    #[serde(default)]
    #[serde(rename = "all_frames")]
    all_frames: bool,
}

fn default_run_at() -> String {
    "document_idle".to_owned()
}

/// Loads the safe subset of unpacked Chrome/WebExtension content scripts.
///
/// The browser deliberately does not execute extension background pages,
/// service workers, popups, native messaging, or `chrome.*` APIs here. Those
/// features need a full extension runtime and are outside WebKitGTK's user
/// content API.
pub(crate) fn load_content_scripts(manager: &UserContentManager) -> usize {
    let root = extensions_root();
    let Ok(root) = root.canonicalize() else {
        return 0;
    };

    let Ok(entries) = fs::read_dir(&root) else {
        return 0;
    };

    let mut loaded = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        match load_extension(&root, &path, manager) {
            Ok(count) => loaded += count,
            Err(error) => eprintln!("Aegis: extensão ignorada ({}): {error}", path.display()),
        }
    }
    loaded
}

pub(crate) fn extensions_root() -> PathBuf {
    if let Some(path) = std::env::var_os(EXTENSIONS_DIR_ENV) {
        return PathBuf::from(path);
    }

    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from(".local/share"));
    data_home.join("aegis-browser/extensions")
}

pub(crate) fn installed_extensions() -> Vec<InstalledExtension> {
    let root = extensions_root();
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };

    let mut extensions = entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            let path = entry.path();
            let manifest = read_manifest(&path).ok()?;
            let icon_path = manifest_icon_path(&path, &manifest);
            let popup_path = manifest_popup_path(&path, &manifest);
            Some(InstalledExtension {
                id: extension_id(&path).ok()?,
                name: manifest.name,
                version: manifest.version,
                icon_path,
                popup_path,
            })
        })
        .collect::<Vec<_>>();
    extensions.sort_by(|left, right| left.name.cmp(&right.name));
    extensions
}

pub(crate) fn import_unpacked_extension(source: &Path) -> Result<InstalledExtension, String> {
    let source = source
        .canonicalize()
        .map_err(|error| format!("não foi possível abrir a pasta: {error}"))?;
    if !source.is_dir() {
        return Err("selecione uma pasta de extensão".to_owned());
    }

    let manifest = read_manifest(&source)?;
    if !matches!(manifest.manifest_version, 2 | 3) {
        return Err(format!(
            "manifest_version {} não suportado",
            manifest.manifest_version
        ));
    }
    let id = extension_id(&source)?;
    let root = extensions_root();
    fs::create_dir_all(&root).map_err(|error| format!("diretório de extensões: {error}"))?;
    let destination = root.join(&id);
    if destination.exists() {
        return Err(format!("a extensão {id} já está instalada"));
    }
    copy_extension_tree(&source, &destination)?;
    let icon_path = manifest_icon_path(&destination, &manifest);
    let popup_path = manifest_popup_path(&destination, &manifest);

    Ok(InstalledExtension {
        id,
        name: manifest.name,
        version: manifest.version,
        icon_path,
        popup_path,
    })
}

pub(crate) fn uninstall_extension(id: &str) -> Result<(), String> {
    if id.is_empty()
        || !id.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '_' || character == '-'
        })
    {
        return Err("identificador de extensão inválido".to_owned());
    }

    let root = extensions_root()
        .canonicalize()
        .map_err(|error| format!("diretório de extensões: {error}"))?;
    let destination = root.join(id);
    let canonical_destination = destination
        .canonicalize()
        .map_err(|error| format!("extensão não encontrada: {error}"))?;
    if canonical_destination.parent() != Some(root.as_path()) || !canonical_destination.is_dir() {
        return Err("extensão fora do diretório permitido".to_owned());
    }

    fs::remove_dir_all(&canonical_destination)
        .map_err(|error| format!("não foi possível remover a extensão: {error}"))
}

fn manifest_icon_path(extension_dir: &Path, manifest: &Manifest) -> Option<PathBuf> {
    let mut candidates = manifest
        .icons
        .iter()
        .filter_map(|(size, path)| size.parse::<u32>().ok().map(|size| (size, path)))
        .collect::<Vec<_>>();
    candidates.sort_by_key(|(size, _)| (*size as i64 - 24).abs());

    let canonical_dir = extension_dir.canonicalize().ok()?;
    for (_, relative_path) in candidates {
        if let Some(path) = extension_asset_path(&canonical_dir, relative_path) {
            return Some(path);
        }
    }
    None
}

fn manifest_popup_path(extension_dir: &Path, manifest: &Manifest) -> Option<PathBuf> {
    let relative_path = manifest
        .action
        .as_ref()
        .and_then(|action| action.default_popup.as_deref())
        .or_else(|| {
            manifest
                .browser_action
                .as_ref()
                .and_then(|action| action.default_popup.as_deref())
        })?;
    let canonical_dir = extension_dir.canonicalize().ok()?;
    extension_asset_path(&canonical_dir, relative_path)
}

fn extension_asset_path(extension_dir: &Path, relative_path: &str) -> Option<PathBuf> {
    let relative = Path::new(relative_path);
    if relative.is_absolute() {
        return None;
    }
    let canonical = extension_dir.join(relative).canonicalize().ok()?;
    if canonical.starts_with(extension_dir) && canonical.is_file() {
        Some(canonical)
    } else {
        None
    }
}

fn read_manifest(extension_dir: &Path) -> Result<Manifest, String> {
    let manifest_path = extension_dir.join("manifest.json");
    let manifest_text =
        fs::read_to_string(&manifest_path).map_err(|error| format!("manifest.json: {error}"))?;
    serde_json::from_str(&manifest_text).map_err(|error| format!("manifest.json inválido: {error}"))
}

fn extension_id(extension_dir: &Path) -> Result<String, String> {
    let raw = extension_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let id = raw
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    if id.is_empty() || id == "." || id == ".." {
        return Err("nome de pasta de extensão inválido".to_owned());
    }
    Ok(id)
}

fn copy_extension_tree(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination)
        .map_err(|error| format!("não foi possível criar a extensão: {error}"))?;
    for entry in
        fs::read_dir(source).map_err(|error| format!("não foi possível ler a extensão: {error}"))?
    {
        let entry = entry.map_err(|error| format!("entrada de extensão inválida: {error}"))?;
        let source_path = entry.path();
        let target_path = destination.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|error| format!("tipo de recurso inválido: {error}"))?;
        if file_type.is_symlink() {
            return Err(format!(
                "links simbólicos não são permitidos: {}",
                source_path.display()
            ));
        }
        if file_type.is_dir() {
            copy_extension_tree(&source_path, &target_path)?;
        } else if file_type.is_file() {
            fs::copy(&source_path, &target_path).map_err(|error| {
                format!("não foi possível copiar {}: {error}", source_path.display())
            })?;
        }
    }
    Ok(())
}

fn load_extension(
    root: &Path,
    extension_dir: &Path,
    manager: &UserContentManager,
) -> Result<usize, String> {
    let manifest_path = extension_dir.join("manifest.json");
    let manifest_text =
        fs::read_to_string(&manifest_path).map_err(|error| format!("manifest.json: {error}"))?;
    let manifest: Manifest = serde_json::from_str(&manifest_text)
        .map_err(|error| format!("manifest.json inválido: {error}"))?;

    if !matches!(manifest.manifest_version, 2 | 3) {
        return Err(format!(
            "manifest_version {} não suportado",
            manifest.manifest_version
        ));
    }

    let world_name = format!("aegis-extension-{}", extension_world_name(extension_dir));
    let mut loaded = 0;
    for content_script in manifest.content_scripts {
        let allow_list = content_script
            .matches
            .iter()
            .flat_map(|pattern| expand_match_pattern(pattern))
            .collect::<Vec<_>>();
        let block_list = content_script
            .exclude_matches
            .iter()
            .flat_map(|pattern| expand_match_pattern(pattern))
            .collect::<Vec<_>>();

        if allow_list.is_empty() {
            continue;
        }

        let allow_refs = allow_list.iter().map(String::as_str).collect::<Vec<_>>();
        let block_refs = block_list.iter().map(String::as_str).collect::<Vec<_>>();
        let injected_frames = if content_script.all_frames {
            UserContentInjectedFrames::AllFrames
        } else {
            UserContentInjectedFrames::TopFrame
        };
        let injection_time = match content_script.run_at.as_str() {
            "document_start" => UserScriptInjectionTime::Start,
            // WebKitGTK exposes start/end only. `document_idle` is mapped to
            // end, which is the least surprising safe fallback.
            "document_end" | "document_idle" => UserScriptInjectionTime::End,
            other => {
                eprintln!(
                    "Aegis: {} usa run_at desconhecido {other:?}; usando document_idle",
                    manifest.name
                );
                UserScriptInjectionTime::End
            }
        };

        for relative_path in content_script.js {
            let source = read_extension_asset(root, extension_dir, &relative_path)?;
            let script = UserScript::for_world(
                &source,
                injected_frames,
                injection_time,
                &world_name,
                &allow_refs,
                &block_refs,
            );
            manager.add_script(&script);
            loaded += 1;
        }

        for relative_path in content_script.css {
            let source = read_extension_asset(root, extension_dir, &relative_path)?;
            let stylesheet = UserStyleSheet::new(
                &source,
                injected_frames,
                UserStyleLevel::Author,
                &allow_refs,
                &block_refs,
            );
            manager.add_style_sheet(&stylesheet);
            loaded += 1;
        }
    }

    if loaded > 0 {
        eprintln!(
            "Aegis: extensão {} {} carregou {loaded} recurso(s)",
            manifest.name, manifest.version
        );
    }
    Ok(loaded)
}

fn read_extension_asset(
    root: &Path,
    extension_dir: &Path,
    relative_path: &str,
) -> Result<String, String> {
    let relative = Path::new(relative_path);
    if relative.is_absolute() {
        return Err(format!("recurso absoluto recusado: {relative_path}"));
    }

    let path = extension_dir.join(relative);
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("recurso {relative_path:?}: {error}"))?;
    if !canonical.starts_with(root) || !canonical.starts_with(extension_dir) {
        return Err(format!(
            "recurso fora da extensão recusado: {relative_path}"
        ));
    }
    fs::read_to_string(&canonical).map_err(|error| format!("recurso {relative_path:?}: {error}"))
}

fn extension_world_name(extension_dir: &Path) -> String {
    extension_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("extension")
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
                character
            } else {
                '-'
            }
        })
        .collect()
}

fn expand_match_pattern(pattern: &str) -> Vec<String> {
    match pattern {
        "<all_urls>" => vec!["http://*/*".to_owned(), "https://*/*".to_owned()],
        pattern if pattern.starts_with("*://") => {
            let host_and_path = &pattern[4..];
            vec![
                format!("http://{host_and_path}"),
                format!("https://{host_and_path}"),
            ]
        }
        pattern if pattern.starts_with("http://") || pattern.starts_with("https://") => {
            vec![pattern.to_owned()]
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{expand_match_pattern, Manifest};

    #[test]
    fn parses_chrome_manifest_field_names() {
        let manifest: Manifest = serde_json::from_str(
            r#"{
                "name": "Teste",
                "version": "1.0",
                "manifest_version": 3,
                "content_scripts": [{
                    "matches": ["<all_urls>"],
                    "exclude_matches": ["https://private.example/*"],
                    "js": ["content.js"],
                    "run_at": "document_start",
                    "all_frames": true
                }]
            }"#,
        )
        .expect("manifesto Chrome válido");

        assert_eq!(manifest.manifest_version, 3);
        assert_eq!(manifest.content_scripts.len(), 1);
        assert!(manifest.content_scripts[0].all_frames);
    }

    #[test]
    fn expands_all_urls_without_local_file_access() {
        assert_eq!(
            expand_match_pattern("<all_urls>"),
            vec!["http://*/*", "https://*/*"]
        );
    }

    #[test]
    fn expands_wildcard_protocol() {
        assert_eq!(
            expand_match_pattern("*://*.example.com/*"),
            vec!["http://*.example.com/*", "https://*.example.com/*"]
        );
    }

    #[test]
    fn rejects_privileged_schemes() {
        assert!(expand_match_pattern("chrome://*/*").is_empty());
        assert!(expand_match_pattern("file:///*").is_empty());
    }
}
