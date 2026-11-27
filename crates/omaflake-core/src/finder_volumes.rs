// SPDX-License-Identifier: AGPL-3.0-or-later
//! The volumes in the Mac's Finder (plan B200): a folder `~/omaflake` with omaflake's icon, holding a link to each
//! volume's folder, and that folder once in the Finder's sidebar – so nobody has to go into `~/.omaflake/share`. The
//! core service keeps the links in step with the volumes; the sidebar entry is added once (taken out by the user, it
//! stays out). Off in the settings (`volumesInFinder`), the folder and the entry go again.
//!
//! The sidebar goes through `LSSharedFileList` – deprecated, but the only way there is without an app extension; the
//! macOS guests do the same (darwin-guest.nix, plan B189). With the Developer ID a File Provider (a "Locations" entry
//! like Google Drive's) or FSKit (each volume a disk, as in unchained guests) can follow (OPEN, M12).

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::instance_store::InstanceStore;
use crate::paths::{Paths, Settings};
use crate::volumes::Volumes;

/// omaflake's mark in the folder: it is ours, and what was done once
const MARK: &str = ".omaflake-volumes";

/// What the mark remembers
#[derive(Debug, Default, Serialize, Deserialize)]
struct Mark {
    /// The sidebar entry was added once – not again after the user took it out
    #[serde(default)]
    favorite: bool,
}

/// JXA: `icon <folder> <icns>` gives the folder an icon; `favorite <folder>` adds it to the Finder's sidebar unless it is
/// there; `unfavorite <folder>` takes it out
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

/// The folder in the home folder
pub fn folder() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join("omaflake"))
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

fn read_mark(dir: &Path) -> Option<Mark> {
    let text = fs::read_to_string(dir.join(MARK)).ok()?;
    Some(serde_json::from_str(&text).unwrap_or_default())
}

fn write_mark(dir: &Path, mark: &Mark) {
    if let Ok(text) = serde_json::to_string(mark) {
        let _ = fs::write(dir.join(MARK), text);
    }
}

/// Brings `~/omaflake` in step with the volumes (macOS only; best effort – what it did, for a log). Only for the
/// user's data folder: a development or test data folder (`VM_HOME`, a temporary one) never touches `~/omaflake`. One
/// at a time (a second call while one runs does nothing): the sidebar entry is added once.
pub fn sync(paths: &Paths) -> Vec<String> {
    use std::sync::atomic::{AtomicBool, Ordering};
    static RUNNING: AtomicBool = AtomicBool::new(false);
    let own_folder = std::env::var_os("VM_HOME").is_none_or(|v| v.is_empty());
    if !cfg!(target_os = "macos") || !own_folder || paths.root != Paths::standard().root {
        return Vec::new();
    }
    let Some(dir) = folder() else { return Vec::new() };
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Vec::new();
    }
    let did = sync_in(paths, &dir, Settings::load(paths).volumes_in_finder.unwrap_or(true), true);
    RUNNING.store(false, Ordering::SeqCst);
    did
}

/// `sync` for a folder `dir`; `finder`: give it an icon and a sidebar entry (off in tests)
pub(crate) fn sync_in(paths: &Paths, dir: &Path, on: bool, finder: bool) -> Vec<String> {
    let mut did = Vec::new();
    let mark = read_mark(dir);
    if !on {
        // Off: our folder goes – its links, the mark, the icon, the sidebar entry; never anything of the user's
        if mark.is_some() {
            if finder {
                let _ = script(&["unfavorite", &dir.to_string_lossy()]);
            }
            remove_ours(dir);
            did.push(format!("removed {}", dir.display()));
        }
        return did;
    }
    // A folder of the same name that is not ours is left alone
    if mark.is_none() && fs::read_dir(dir).is_ok_and(|mut e| e.next().is_some()) {
        return did;
    }
    if fs::create_dir_all(dir).is_err() {
        return did;
    }
    let mut mark = mark.unwrap_or_default();
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
    // Links of volumes that are gone (only links – nothing else in the folder is touched)
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let is_link = e.file_type().is_ok_and(|t| t.is_symlink());
            if is_link && !volumes.iter().any(|v| v.name == name) && fs::remove_file(e.path()).is_ok() {
                did.push(format!("unlinked {name}"));
            }
        }
    }
    if finder {
        if !dir.join("Icon\r").exists()
            && let Some(icns) = app_icon()
            && let Some(r) = script(&["icon", &dir.to_string_lossy(), &icns.to_string_lossy()])
        {
            did.push(r);
        }
        if !mark.favorite
            && let Some(r) = script(&["favorite", &dir.to_string_lossy()])
        {
            mark.favorite = true;
            did.push(format!("sidebar: {r}"));
        }
    }
    write_mark(dir, &mark);
    did
}

/// Takes out what omaflake put into the folder, and the folder when nothing else is left
fn remove_ours(dir: &Path) {
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            let name = e.file_name();
            let ours =
                e.file_type().is_ok_and(|t| t.is_symlink()) || name == MARK || name == "Icon\r" || name == ".DS_Store";
            if ours {
                let _ = fs::remove_file(e.path());
            }
        }
    }
    let _ = fs::remove_dir(dir);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::tests::temp_paths;

    #[test]
    fn the_folder_links_each_volume_and_forgets_gone_ones() {
        let (t, p) = temp_paths();
        let dir = t.path().join("home-omaflake");
        fs::create_dir_all(p.share("music")).unwrap();
        fs::create_dir_all(p.share("test")).unwrap();
        let did = sync_in(&p, &dir, true, false);
        assert_eq!(did, ["linked music", "linked test"]);
        assert_eq!(fs::read_link(dir.join("music")).unwrap(), p.share("music"));
        assert!(dir.join(MARK).exists());
        // Again: nothing to do
        assert!(sync_in(&p, &dir, true, false).is_empty());
        // A volume gone: its link goes, a file of the user's stays
        fs::remove_dir(p.share("test")).unwrap();
        fs::write(dir.join("notes.txt"), "mine").unwrap();
        assert_eq!(sync_in(&p, &dir, true, false), ["unlinked test"]);
        assert!(dir.join("notes.txt").exists());
        // Off: our links and mark go, the user's file and so the folder stay
        sync_in(&p, &dir, false, false);
        assert!(!dir.join("music").exists() && !dir.join(MARK).exists());
        assert!(dir.join("notes.txt").exists());
    }

    #[test]
    fn a_folder_that_is_not_ours_is_left_alone() {
        let (t, p) = temp_paths();
        let dir = t.path().join("home-omaflake");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("project.txt"), "x").unwrap();
        fs::create_dir_all(p.share("music")).unwrap();
        assert!(sync_in(&p, &dir, true, false).is_empty());
        assert!(!dir.join("music").exists() && !dir.join(MARK).exists());
    }
}
