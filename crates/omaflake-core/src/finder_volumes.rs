// SPDX-License-Identifier: AGPL-3.0-or-later
//! The volumes in the Mac's Finder (plan B200): a folder holding a link to each volume's folder, with omaflake's icon
//! and once in the Finder's sidebar – so nobody has to go into `~/.omaflake/share`.
//!
//! Where: `volumes.localized` in the data folder by default (it moves with it), or in a folder the user chose
//! (`volumesFolder`), with a link `volumes` next to it for the Terminal. Its name in the Finder: macOS's own way of
//! localized folder names (as /Applications is "Programme" in German) – the folder ends in `.localized` and its
//! `.localized/<language>.strings` name it: "omaflake volumes", in German "omaflake Platten", or the user's own name
//! (`volumesName`). A link would not do: macOS translates only a real folder's name (checked 2026-11-27).
//!
//! The core service keeps it in step; the sidebar entry is added once (taken out by the user, it stays out) and moves
//! with the folder. Off (`volumesInFinder`), the folder, the link and the entry go again – never the user's files. The
//! sidebar goes through `LSSharedFileList` – deprecated, but the only way without an app extension; the macOS guests do
//! the same (darwin-guest.nix, plan B189). With the Developer ID a File Provider or FSKit can follow (OPEN, M12).

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::instance_store::InstanceStore;
use crate::paths::{Paths, Settings};
use crate::volumes::Volumes;

/// The folder's name on disk; macOS shows it by `.localized/<language>.strings`
pub const FOLDER: &str = "volumes.localized";
/// The link next to it, for the Terminal (`cd ~/.omaflake/volumes`)
pub const LINK: &str = "volumes";
/// omaflake's mark in the folder: it is ours
const MARK: &str = ".omaflake-volumes";
/// In the data folder: where the folder was last made, and whether its sidebar entry was added
const STATE: &str = "finder-volumes.json";
/// The languages it names the folder in (more fall back to the system's choice among them)
const NAMES: [(&str, &str); 2] = [("en", "omaflake volumes"), ("de", "omaflake Platten")];

#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    /// The folder as last made
    #[serde(default, skip_serializing_if = "Option::is_none")]
    folder: Option<PathBuf>,
    /// Its sidebar entry was added – not again after the user took it out
    #[serde(default)]
    favorite: bool,
}

/// JXA: `icon <folder> <icns>` gives the folder an icon; `favorite <folder>` adds it to the Finder's sidebar unless it is
/// there; `unfavorite <folder>` takes it out ("removed", or "not there")
const SCRIPT: &str = r#"
ObjC.import('AppKit');
ObjC.import('CoreServices');
function run(argv) {
  const [what, dir, icns] = argv;
  if (what === 'icon') {
    const image = $.NSImage.alloc.initWithContentsOfFile(icns);
    return $.NSWorkspace.sharedWorkspace.setIconForFileOptions(image, dir, 0) ? 'icon set' : 'icon not set';
  }
  const real = p => $(p).stringByResolvingSymlinksInPath.js;
  const want = real(dir);
  const list = $.LSSharedFileListCreate($(), $.kLSSharedFileListFavoriteItems, $());
  const items = ObjC.castRefToObject($.LSSharedFileListCopySnapshot(list, null));
  for (let i = 0; i < items.count; i++) {
    const item = items.objectAtIndex(i);
    let path = null;
    try { path = real(ObjC.castRefToObject($.LSSharedFileListItemCopyResolvedURL(item, 3, null)).path.js); } catch (e) {}
    if (path === want) {
      if (what === 'unfavorite') { $.LSSharedFileListItemRemove(list, item); return 'removed'; }
      return 'there';
    }
  }
  if (what === 'unfavorite') return 'not there';
  $.LSSharedFileListInsertItemURL(list, $.kLSSharedFileListItemLast, $(), $(), $.NSURL.fileURLWithPath(dir), $(), $());
  return 'added';
}
"#;

/// Where the folder goes: the chosen folder (`volumesFolder`), else the data folder
pub fn place(paths: &Paths, settings: &Settings) -> PathBuf {
    match settings.volumes_folder.as_deref().filter(|f| !f.is_empty()) {
        Some(f) => PathBuf::from(crate::platform::expand_tilde(f)),
        None => paths.root.clone(),
    }
}

/// The folder itself
pub fn folder(paths: &Paths, settings: &Settings) -> PathBuf {
    place(paths, settings).join(FOLDER)
}

/// omaflake's own icon, from the app this core runs in (`omaflake.app/Contents/Resources/omaflake.icns`); None for an
/// omaf outside the app
fn app_icon() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let icns = exe.parent()?.parent()?.join("Resources/omaflake.icns");
    icns.exists().then_some(icns)
}

fn script(args: &[&str]) -> Option<String> {
    let mut all = vec!["-l", "JavaScript", "-e", SCRIPT];
    all.extend_from_slice(args);
    let out = std::process::Command::new("/usr/bin/osascript")
        .args(all)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn read_state(paths: &Paths) -> State {
    let text = fs::read_to_string(paths.root.join(STATE)).unwrap_or_default();
    serde_json::from_str(&text).unwrap_or_default()
}

fn write_state(paths: &Paths, state: &State) {
    if let Ok(text) = serde_json::to_string(state) {
        let _ = fs::write(paths.root.join(STATE), text);
    }
}

/// Brings the folder in step with the volumes and the settings (macOS only; best effort – what it did, for a log).
/// Only for the user's data folder: a development or test data folder (`VM_HOME`, a temporary one) never touches the
/// Finder. One at a time (a second call while one runs does nothing): the sidebar entry is added once.
pub fn sync(paths: &Paths) -> Vec<String> {
    use std::sync::atomic::{AtomicBool, Ordering};
    static RUNNING: AtomicBool = AtomicBool::new(false);
    let own_folder = std::env::var_os("VM_HOME").is_none_or(|v| v.is_empty());
    if !cfg!(target_os = "macos") || !own_folder || paths.root != Paths::standard().root {
        return Vec::new();
    }
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Vec::new();
    }
    // A first build of B200 made ~/omaflake: noted as the old place, so it goes and its sidebar entry moves
    if read_state(paths).folder.is_none()
        && let Some(old) = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("omaflake"))
        && old.join(MARK).exists()
    {
        write_state(paths, &State { folder: Some(old), favorite: true });
    }
    let did = sync_in(paths, &Settings::load(paths), true);
    RUNNING.store(false, Ordering::SeqCst);
    did
}

/// `sync` for `paths`; `finder`: give it an icon and a sidebar entry (off in tests)
pub(crate) fn sync_in(paths: &Paths, settings: &Settings, finder: bool) -> Vec<String> {
    let mut did = Vec::new();
    let mut state = read_state(paths);
    let target = settings.volumes_in_finder.unwrap_or(true).then(|| folder(paths, settings));
    // Off, or moved: what omaflake made at the old place goes; its sidebar entry moves along unless the user took it out
    if let Some(old) = state.folder.clone().filter(|o| Some(o) != target.as_ref()) {
        let moved = finder && script(&["unfavorite", &old.to_string_lossy()]).as_deref() == Some("removed");
        remove_ours(&old);
        did.push(format!("removed {}", old.display()));
        state = State { folder: None, favorite: state.favorite && !moved };
    }
    let Some(dir) = target else {
        write_state(paths, &state);
        return did;
    };
    // A folder of that name that is not ours is left alone
    if !dir.join(MARK).exists() && fs::read_dir(&dir).is_ok_and(|mut e| e.next().is_some()) {
        return did;
    }
    if fs::create_dir_all(&dir).is_err() {
        return did;
    }
    let _ = fs::write(dir.join(MARK), "");
    name_it(&dir, settings.volumes_name.as_deref().filter(|n| !n.trim().is_empty()));
    link_volumes(paths, &dir, &mut did);
    // The link for the Terminal, next to it
    if let Some(parent) = dir.parent() {
        let link = parent.join(LINK);
        if fs::symlink_metadata(&link).is_err() && std::os::unix::fs::symlink(FOLDER, &link).is_ok() {
            did.push(format!("linked {}", link.display()));
        }
    }
    if finder {
        if !dir.join("Icon\r").exists()
            && let Some(icns) = app_icon()
            && let Some(r) = script(&["icon", &dir.to_string_lossy(), &icns.to_string_lossy()])
        {
            did.push(r);
        }
        if !state.favorite
            && let Some(r) = script(&["favorite", &dir.to_string_lossy()])
        {
            state.favorite = true;
            did.push(format!("sidebar: {r}"));
        }
    }
    state.folder = Some(dir);
    write_state(paths, &state);
    did
}

/// The folder's names in the Finder: `.localized/<language>.strings` with "volumes" = the name – the user's in every
/// language, else omaflake's
fn name_it(dir: &Path, own: Option<&str>) {
    let names = dir.join(".localized");
    let _ = fs::create_dir_all(&names);
    for (language, name) in NAMES {
        let name = own.unwrap_or(name).replace('\\', "\\\\").replace('"', "\\\"");
        let text = format!("\"{LINK}\" = \"{name}\";\n");
        let file = names.join(format!("{language}.strings"));
        if fs::read_to_string(&file).ok().as_deref() != Some(text.as_str()) {
            let _ = fs::write(&file, text);
        }
    }
}

/// A link to each volume's folder; links of volumes that are gone go – nothing else in the folder is touched
fn link_volumes(paths: &Paths, dir: &Path, did: &mut Vec<String>) {
    let store = InstanceStore::new(paths.clone(), "qemu-img");
    let volumes = Volumes::new(paths.clone()).list(&store.list());
    for v in &volumes {
        let link = dir.join(&v.name);
        let target = fs::read_link(&link).ok();
        if target.as_deref() == Some(v.folder.as_path()) {
            continue;
        }
        if target.is_some() {
            let _ = fs::remove_file(&link);
        } else if link.exists() {
            continue; // something of the user's of that name
        }
        if std::os::unix::fs::symlink(&v.folder, &link).is_ok() {
            did.push(format!("linked {}", v.name));
        }
    }
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let is_link = e.file_type().is_ok_and(|t| t.is_symlink());
            if is_link && !volumes.iter().any(|v| v.name == name) && fs::remove_file(e.path()).is_ok() {
                did.push(format!("unlinked {name}"));
            }
        }
    }
}

/// Takes out what omaflake put there – its links, mark, names, icon, the Terminal link next to it – and the folder
/// when nothing else is left
fn remove_ours(dir: &Path) {
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            let name = e.file_name();
            let link = e.file_type().is_ok_and(|t| t.is_symlink());
            if link || name == MARK || name == "Icon\r" || name == ".DS_Store" {
                let _ = fs::remove_file(e.path());
            } else if name == ".localized" {
                let _ = fs::remove_dir_all(e.path());
            }
        }
    }
    if let Some(parent) = dir.parent() {
        let link = parent.join(LINK);
        if fs::read_link(&link).is_ok_and(|t| parent.join(&t) == dir) {
            let _ = fs::remove_file(&link);
        }
    }
    let _ = fs::remove_dir(dir);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::tests::temp_paths;

    #[test]
    fn the_folder_links_each_volume_and_is_named() {
        let (_t, p) = temp_paths();
        fs::create_dir_all(p.share("music")).unwrap();
        fs::create_dir_all(p.share("test")).unwrap();
        let s = Settings::default();
        let did = sync_in(&p, &s, false);
        let dir = p.root.join(FOLDER);
        assert!(did.contains(&"linked music".to_string()) && did.contains(&"linked test".to_string()));
        assert_eq!(fs::read_link(dir.join("music")).unwrap(), p.share("music"));
        assert_eq!(fs::read_link(p.root.join(LINK)).unwrap(), PathBuf::from(FOLDER));
        let de = fs::read_to_string(dir.join(".localized/de.strings")).unwrap();
        assert_eq!(de, "\"volumes\" = \"omaflake Platten\";\n");
        // Again: nothing to do
        assert!(sync_in(&p, &s, false).is_empty());
        // A volume gone: its link goes, a file of the user's stays
        fs::remove_dir(p.share("test")).unwrap();
        fs::write(dir.join("notes.txt"), "mine").unwrap();
        assert_eq!(sync_in(&p, &s, false), ["unlinked test"]);
        // The user's own name, in every language
        let named = Settings { volumes_name: Some("VM-Platten".into()), ..Settings::default() };
        sync_in(&p, &named, false);
        assert!(fs::read_to_string(dir.join(".localized/en.strings")).unwrap().contains("\"VM-Platten\""));
        // Off: omaflake's links, names and the Terminal link go; the user's file and so the folder stay
        let off = Settings { volumes_in_finder: Some(false), ..Settings::default() };
        sync_in(&p, &off, false);
        assert!(!dir.join("music").exists() && !dir.join(".localized").exists() && !p.root.join(LINK).exists());
        assert!(dir.join("notes.txt").exists());
    }

    #[test]
    fn a_chosen_place_moves_the_folder() {
        let (t, p) = temp_paths();
        fs::create_dir_all(p.share("music")).unwrap();
        sync_in(&p, &Settings::default(), false);
        assert!(p.root.join(FOLDER).join("music").exists());
        let elsewhere = t.path().join("Documents");
        fs::create_dir_all(&elsewhere).unwrap();
        let moved = Settings { volumes_folder: Some(elsewhere.to_string_lossy().into()), ..Settings::default() };
        sync_in(&p, &moved, false);
        assert!(!p.root.join(FOLDER).exists() && !p.root.join(LINK).exists());
        assert_eq!(fs::read_link(elsewhere.join(FOLDER).join("music")).unwrap(), p.share("music"));
        assert!(elsewhere.join(LINK).exists());
    }

    #[test]
    fn a_folder_that_is_not_ours_is_left_alone() {
        let (_t, p) = temp_paths();
        let dir = p.root.join(FOLDER);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("project.txt"), "x").unwrap();
        fs::create_dir_all(p.share("music")).unwrap();
        assert!(sync_in(&p, &Settings::default(), false).is_empty());
        assert!(!dir.join("music").exists() && !dir.join(MARK).exists());
    }
}
