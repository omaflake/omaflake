// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of Trash.swift.
//!
//! Deleting what the user removes – an instance, its old state on reset/upgrade, an image, a failed build. The
//! product deletes for real; "never delete, move to _to_delete" is a rule for the developers' (and agents') own
//! actions during development, not for the app (spec §9).

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use crate::error::{Error, Result};
use crate::platform::is_writable;

/// Swift `enum Disposal` (namespace)
pub enum Disposal {}

impl Disposal {
    /// Deletes the items; read-only folders (macOS images and bases) are made writable first. Missing ones are
    /// skipped. A symbolic link is deleted, not what it points to.
    pub fn delete<P: AsRef<Path>>(items: &[P]) -> Result<()> {
        for item in items {
            let item = item.as_ref();
            let Ok(meta) = fs::symlink_metadata(item) else { continue };
            let result = if meta.file_type().is_symlink() || !meta.is_dir() {
                fs::remove_file(item)
            } else {
                Self::make_writable(item);
                fs::remove_dir_all(item)
            };
            result.map_err(|e| Error::from(e).context(format!("Cannot delete {}: ", item.display())))?;
        }
        Ok(())
    }

    /// u+w on a folder and everything in it (only what is needed to delete it)
    pub(crate) fn make_writable(path: &Path) {
        let Ok(meta) = fs::symlink_metadata(path) else { return };
        if !meta.is_dir() {
            return;
        }
        if !is_writable(path) {
            let mode = meta.permissions().mode() & 0o7777;
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode | 0o700));
        }
        let Ok(entries) = fs::read_dir(path) else { return };
        for entry in entries.flatten() {
            let child = entry.path();
            let is_link = fs::symlink_metadata(&child).map(|m| m.file_type().is_symlink()).unwrap_or(true);
            if !is_link {
                Self::make_writable(&child);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::tests::temp_paths;

    #[test]
    fn deletes_files_folders_and_links() {
        let (_t, p) = temp_paths();
        let dir = p.root.join("d");
        fs::create_dir_all(&dir).unwrap();
        let file = p.root.join("f");
        fs::write(&file, "x").unwrap();
        let link = p.root.join("l");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        Disposal::delete(&[link.clone(), dir.clone(), p.root.join("missing")]).unwrap();
        assert!(!dir.exists() && fs::symlink_metadata(&link).is_err());
        assert!(file.exists()); // a link is deleted, not what it points to
    }

    #[test]
    fn deletes_read_only_folders() {
        let (_t, p) = temp_paths();
        let dir = p.bases().join("mac-base-1/sub");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("disk.img"), "x").unwrap();
        for d in [dir.clone(), dir.parent().unwrap().to_path_buf()] {
            fs::set_permissions(&d, fs::Permissions::from_mode(0o555)).unwrap();
        }
        Disposal::delete(&[dir.parent().unwrap()]).unwrap();
        assert!(!p.bases().join("mac-base-1").exists());
    }
}
