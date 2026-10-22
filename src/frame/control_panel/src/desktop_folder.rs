/*
The signed-in user's Desktop folder, part of the desktop UI state (`/api/desktop`).

Like Windows, the desktop shows a folder in the user's home, so Files shows the same items:

  data/home/{user}/Desktop        (cyfs:///home/{user}/Desktop)

Every device and UI session shows the folder's items; a session only keeps where they sit. Names starting
with `.` are hidden. Two kinds of small JSON files are desktop objects rather than documents:

- `*.lnk`     a shortcut, e.g. {"app": "messagehub"}
- `*.widget`  a widget, e.g. {"widget": "clock"} or {"widget": "notepad", "text": "..."}

Actions (POST /api/desktop, `{action, ...}`):
- `items.list {seed?: [{name, text}]}` → `{path, created, items: [{name, kind, size, mtime, text?}]}`.
  A missing folder is created and `seed` written into it (`created: true`), together with the shortcuts
  `system/install_settings` declares for pre-installed Apps (`desktop_shortcut`); a folder the user emptied
  stays empty. `kind` is `file` or `dir`; `text` is the content of a `.lnk`/`.widget` file of at most 64 KiB.
- `items.write {name, text, create?}` writes a `.lnk`/`.widget` file; with `create` an existing one is left
  alone and the call fails with `exists: {name}`.
- `items.delete {name}` removes a `.lnk`/`.widget` file. Documents and folders are changed in Files.
*/

use super::download_mgr::home_segment;
use buckyos_api::SystemInstallSettings;
use buckyos_kit::get_buckyos_root_dir;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

pub(super) const DESKTOP_FOLDER_NAME: &str = "Desktop";
const MAX_OBJECT_BYTES: usize = 64 * 1024;
const MAX_SEED_ITEMS: usize = 64;
const MAX_NAME_BYTES: usize = 255;
const OBJECT_EXTENSIONS: [&str; 2] = [".lnk", ".widget"];

/// `data/home/{user}/Desktop`, when the user name is safe as a folder name.
pub(super) fn desktop_dir(username: &str) -> Option<PathBuf> {
    let home = home_segment(username)?;
    Some(
        get_buckyos_root_dir()
            .join("data")
            .join("home")
            .join(home)
            .join(DESKTOP_FOLDER_NAME),
    )
}

/// The folder as Files shows it.
pub(super) fn desktop_dfs_path(username: &str) -> String {
    format!("/home/{}/{}", username.trim(), DESKTOP_FOLDER_NAME)
}

/// One visible entry name: a single path segment, not hidden.
pub(super) fn is_desktop_item_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_BYTES
        && !name.starts_with('.')
        && !name
            .chars()
            .any(|c| c == '/' || c == '\\' || c.is_control())
}

/// A shortcut or widget file, the only entries the desktop itself writes.
pub(super) fn is_desktop_object_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    is_desktop_item_name(name)
        && OBJECT_EXTENSIONS
            .iter()
            .any(|ext| lower.len() > ext.len() && lower.ends_with(ext))
}

pub(super) struct DesktopSeedItem {
    pub name: String,
    pub text: String,
}

/// `seed` of `items.list`; entries that are not desktop objects are refused, not skipped.
pub(super) fn parse_seed(value: Option<&Value>) -> Result<Vec<DesktopSeedItem>, String> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(Vec::new());
    };
    let entries = value.as_array().ok_or("seed must be an array")?;
    if entries.len() > MAX_SEED_ITEMS {
        return Err(format!("seed has more than {} items", MAX_SEED_ITEMS));
    }
    entries
        .iter()
        .map(|entry| {
            let name = entry.get("name").and_then(Value::as_str).unwrap_or("");
            let text = entry.get("text").and_then(Value::as_str).unwrap_or("");
            check_object(name, text)?;
            Ok(DesktopSeedItem {
                name: name.to_string(),
                text: text.to_string(),
            })
        })
        .collect()
}

fn check_object(name: &str, text: &str) -> Result<(), String> {
    if !is_desktop_object_name(name) {
        return Err(format!("not a .lnk or .widget name: {}", name));
    }
    if text.len() > MAX_OBJECT_BYTES {
        return Err(format!("{} is larger than 64 KiB", name));
    }
    Ok(())
}

/// A shortcut's file name from a display name: separators and control characters become spaces.
fn shortcut_file_name(label: &str) -> Option<String> {
    let cleaned = label
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || c.is_control() {
                ' '
            } else {
                c
            }
        })
        .collect::<String>();
    let stem = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let stem = stem.trim_start_matches('.');
    let name = format!("{}.lnk", stem);
    (!stem.is_empty() && is_desktop_object_name(&name)).then_some(name)
}

/// The shortcuts `system/install_settings` declares for pre-installed Apps, named apart from `seed`.
pub(super) fn declared_shortcuts(
    settings: &SystemInstallSettings,
    seed: &[DesktopSeedItem],
) -> Vec<DesktopSeedItem> {
    let mut taken: Vec<String> = seed.iter().map(|item| item.name.to_lowercase()).collect();
    let mut apps: Vec<_> = settings
        .pre_install_apps
        .iter()
        .filter_map(|(app_id, config)| {
            config
                .desktop_shortcut
                .as_ref()
                .map(|shortcut| (app_id, shortcut))
        })
        .collect();
    apps.sort_by(|a, b| a.0.cmp(b.0));
    let mut items = Vec::new();
    for (app_id, shortcut) in apps {
        let Some(name) = shortcut_file_name(&shortcut.name) else {
            log::warn!(
                "desktop: install_settings shortcut name {:?} for {} is not a file name",
                shortcut.name,
                app_id
            );
            continue;
        };
        let stem = &name[..name.len() - ".lnk".len()];
        let mut candidate = name.clone();
        let mut n = 2;
        while taken.contains(&candidate.to_lowercase()) {
            candidate = format!("{} ({}).lnk", stem, n);
            n += 1;
        }
        taken.push(candidate.to_lowercase());
        items.push(DesktopSeedItem {
            name: candidate,
            text: format!(
                "{}\n",
                serde_json::to_string_pretty(&json!({ "app": app_id })).unwrap_or_default()
            ),
        });
    }
    items
}

pub(super) async fn folder_exists(dir: &Path) -> bool {
    tokio::fs::metadata(dir).await.is_ok()
}

async fn write_atomic(dir: &Path, name: &str, text: &str) -> std::io::Result<()> {
    let partial = dir.join(format!(".{}.part", name));
    tokio::fs::write(&partial, text.as_bytes()).await?;
    tokio::fs::rename(&partial, dir.join(name)).await
}

/// Creates the folder with `seed` when it does not exist yet. True when this call created it.
async fn ensure_folder(dir: &Path, seed: &[DesktopSeedItem]) -> std::io::Result<bool> {
    if folder_exists(dir).await {
        return Ok(false);
    }
    if let Some(home) = dir.parent() {
        tokio::fs::create_dir_all(home).await?;
    }
    match tokio::fs::create_dir(dir).await {
        Ok(()) => {}
        // Another device got there first; it writes the seed.
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => return Ok(false),
        Err(error) => return Err(error),
    }
    for item in seed {
        write_atomic(dir, &item.name, &item.text).await?;
    }
    Ok(true)
}

fn unix_seconds(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

pub(super) async fn list_items(
    dir: &Path,
    seed: &[DesktopSeedItem],
) -> Result<(bool, Vec<Value>), String> {
    let created = ensure_folder(dir, seed)
        .await
        .map_err(|error| format!("prepare the Desktop folder: {}", error))?;
    let mut reader = tokio::fs::read_dir(dir)
        .await
        .map_err(|error| format!("read the Desktop folder: {}", error))?;
    let mut items = Vec::new();
    while let Ok(Some(entry)) = reader.next_entry().await {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if !is_desktop_item_name(&name) {
            continue;
        }
        // Not followed: a link shows as a plain file and is never read here.
        let Ok(meta) = entry.metadata().await else {
            continue;
        };
        let is_dir = meta.is_dir();
        let mut item = json!({
            "name": name,
            "kind": if is_dir { "dir" } else { "file" },
            "size": if is_dir { 0 } else { meta.len() },
            "mtime": unix_seconds(&meta),
        });
        if meta.is_file()
            && is_desktop_object_name(&name)
            && meta.len() as usize <= MAX_OBJECT_BYTES
        {
            if let Ok(bytes) = tokio::fs::read(entry.path()).await {
                item["text"] = Value::String(String::from_utf8_lossy(&bytes).into_owned());
            }
        }
        items.push(item);
    }
    items.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Ok((created, items))
}

pub(super) async fn write_item(
    dir: &Path,
    name: &str,
    text: &str,
    create: bool,
) -> Result<(), String> {
    check_object(name, text)?;
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|error| format!("prepare the Desktop folder: {}", error))?;
    if create && tokio::fs::symlink_metadata(dir.join(name)).await.is_ok() {
        return Err(format!("exists: {}", name));
    }
    write_atomic(dir, name, text)
        .await
        .map_err(|error| format!("write {}: {}", name, error))
}

pub(super) async fn delete_item(dir: &Path, name: &str) -> Result<(), String> {
    if !is_desktop_object_name(name) {
        return Err(format!("not a .lnk or .widget name: {}", name));
    }
    match tokio::fs::remove_file(dir.join(name)).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("delete {}: {}", name, error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_desktop() -> PathBuf {
        std::env::temp_dir()
            .join(format!("cp-desktop-{}", uuid::Uuid::new_v4()))
            .join("alice")
            .join(DESKTOP_FOLDER_NAME)
    }

    fn seed(items: &[(&str, &str)]) -> Vec<DesktopSeedItem> {
        items
            .iter()
            .map(|(name, text)| DesktopSeedItem {
                name: name.to_string(),
                text: text.to_string(),
            })
            .collect()
    }

    #[test]
    fn names_are_single_visible_segments() {
        assert!(is_desktop_item_name("Notes.widget"));
        assert!(is_desktop_item_name("照片 2026.jpg"));
        assert!(!is_desktop_item_name(""));
        assert!(!is_desktop_item_name(".hidden"));
        assert!(!is_desktop_item_name(".."));
        assert!(!is_desktop_item_name("a/b.lnk"));
        assert!(!is_desktop_item_name("a\\b.lnk"));
        assert!(!is_desktop_item_name("line\nbreak.lnk"));
        assert!(!is_desktop_item_name(&"a".repeat(256)));

        assert!(is_desktop_object_name("MessageHub.lnk"));
        assert!(is_desktop_object_name("Clock.WIDGET"));
        assert!(!is_desktop_object_name(".lnk"));
        assert!(!is_desktop_object_name("photo.jpg"));
        assert!(!is_desktop_object_name("../x.lnk"));
    }

    #[test]
    fn a_seed_only_holds_desktop_objects() {
        let ok = json!([{"name": "Clock.widget", "text": "{\"widget\":\"clock\"}"}]);
        assert_eq!(parse_seed(Some(&ok)).unwrap().len(), 1);
        assert!(parse_seed(None).unwrap().is_empty());
        assert!(parse_seed(Some(&Value::Null)).unwrap().is_empty());
        assert!(parse_seed(Some(&json!([{"name": "run.sh", "text": ""}]))).is_err());
        assert!(parse_seed(Some(
            &json!([{"name": "x.lnk", "text": "a".repeat(MAX_OBJECT_BYTES + 1)}])
        ))
        .is_err());
        assert!(parse_seed(Some(&json!({"name": "x.lnk"}))).is_err());
    }

    #[test]
    fn pre_installed_apps_declare_their_shortcuts() {
        let settings: SystemInstallSettings = serde_json::from_value(json!({
            "pre_install_apps": {
                "text-editor.buckyos.bns.did": {
                    "schema_version": 1, "pikg_path": "data/cache/a.pikg", "install_plan": {},
                    "desktop_shortcut": {"name": "Text Editor"}
                },
                "b.example.did": {
                    "schema_version": 1, "pikg_path": "data/cache/b.pikg", "install_plan": {},
                    "desktop_shortcut": {"name": " Files / Tools "}
                },
                "c.example.did": {"schema_version": 1, "pikg_path": "data/cache/c.pikg", "install_plan": {}},
                "d.example.did": {
                    "schema_version": 1, "pikg_path": "data/cache/d.pikg", "install_plan": {},
                    "desktop_shortcut": {"name": "..."}
                }
            }
        }))
        .unwrap();
        let seed = seed(&[("Files Tools.lnk", "{}")]);
        let items = declared_shortcuts(&settings, &seed);
        let names: Vec<_> = items.iter().map(|item| item.name.as_str()).collect();
        // Sorted by AppId; a taken name is numbered; an unusable name is skipped.
        assert_eq!(names, ["Files Tools (2).lnk", "Text Editor.lnk"]);
        let text: Value = serde_json::from_str(&items[1].text).unwrap();
        assert_eq!(text, json!({"app": "text-editor.buckyos.bns.did"}));
    }

    #[tokio::test]
    async fn the_seed_is_written_once_and_an_emptied_folder_stays_empty() {
        let dir = temp_desktop();
        let defaults = seed(&[
            ("Clock.widget", r#"{"widget":"clock"}"#),
            ("Files.lnk", r#"{"app":"files"}"#),
        ]);

        let (created, items) = list_items(&dir, &defaults).await.unwrap();
        assert!(created);
        let names: Vec<_> = items
            .iter()
            .map(|item| item["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["Clock.widget", "Files.lnk"]);
        assert_eq!(items[1]["text"], r#"{"app":"files"}"#);
        assert_eq!(items[1]["kind"], "file");

        delete_item(&dir, "Clock.widget").await.unwrap();
        delete_item(&dir, "Files.lnk").await.unwrap();
        let (created, items) = list_items(&dir, &defaults).await.unwrap();
        assert!(!created);
        assert!(items.is_empty());

        let _ = std::fs::remove_dir_all(dir.parent().unwrap().parent().unwrap());
    }

    #[tokio::test]
    async fn documents_and_folders_show_without_their_content() {
        let dir = temp_desktop();
        list_items(&dir, &[]).await.unwrap();
        std::fs::write(dir.join("photo.jpg"), b"\xFF\xD8\xFFjpeg").unwrap();
        std::fs::write(dir.join(".hidden.lnk"), b"{}").unwrap();
        std::fs::create_dir(dir.join("Projects")).unwrap();

        let (_, items) = list_items(&dir, &[]).await.unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["name"], "Projects");
        assert_eq!(items[0]["kind"], "dir");
        assert_eq!(items[1]["name"], "photo.jpg");
        assert_eq!(items[1]["size"], 7);
        assert!(items[1].get("text").is_none());

        // Only desktop objects are written or deleted through the desktop.
        assert!(delete_item(&dir, "photo.jpg").await.is_err());
        assert!(write_item(&dir, "photo.jpg", "x", false).await.is_err());
        assert!(dir.join("photo.jpg").exists());

        let _ = std::fs::remove_dir_all(dir.parent().unwrap().parent().unwrap());
    }

    #[tokio::test]
    async fn create_never_replaces_an_item() {
        let dir = temp_desktop();
        write_item(&dir, "Notes.widget", "one", true).await.unwrap();
        let error = write_item(&dir, "Notes.widget", "two", true)
            .await
            .unwrap_err();
        assert!(error.starts_with("exists:"));
        assert_eq!(
            std::fs::read_to_string(dir.join("Notes.widget")).unwrap(),
            "one"
        );
        write_item(&dir, "Notes.widget", "three", false)
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("Notes.widget")).unwrap(),
            "three"
        );
        // A missing item is already deleted.
        delete_item(&dir, "Gone.lnk").await.unwrap();

        let _ = std::fs::remove_dir_all(dir.parent().unwrap().parent().unwrap());
    }
}
