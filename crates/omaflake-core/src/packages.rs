// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of Packages.swift.
//!
//! omaflake's portable packages (spec §11.10, plan B108): a VM, an image or recipes as one folder with an extension –
//! the Finder shows it as one file; copying it is a backup, double-clicking it imports it. The data folder keeps VMs
//! and images in exactly this form, so `~/.omaflake/instances/research.omaflake-vm` already is the backup.

use crate::vm_err;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::{IoContext, Result};
use crate::file_format::{FileFormat, Step, write_atomic};
use crate::models::GuestKind;
use crate::platform::{is_writable, resolve_symlinks, standardize};

/// Swift `Package.Kind`; serialized as its extension
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PackageKind {
    #[serde(rename = "omaflake-vm")]
    Vm,
    #[serde(rename = "omaflake-image")]
    Image,
    #[serde(rename = "omaflake-recipe")]
    Recipe,
}

impl PackageKind {
    pub const ALL: [PackageKind; 3] = [PackageKind::Vm, PackageKind::Image, PackageKind::Recipe];

    /// The extension ("omaflake-vm", …) – Swift `rawValue`
    pub fn extension(self) -> &'static str {
        match self {
            PackageKind::Vm => Package::VM,
            PackageKind::Image => Package::IMAGE,
            PackageKind::Recipe => Package::RECIPE,
        }
    }

    pub fn from_extension(ext: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.extension() == ext)
    }
}

/// Swift `enum Package` (namespace)
pub enum Package {}

impl Package {
    pub const VM: &'static str = "omaflake-vm";
    pub const IMAGE: &'static str = "omaflake-image";
    pub const RECIPE: &'static str = "omaflake-recipe";

    /// "research.omaflake-vm" → "research" (None for another kind). Swift `name(of:kind:)`.
    pub fn name_of(path: &Path, kind: PackageKind) -> Option<String> {
        let last = standardize(path).file_name()?.to_string_lossy().into_owned();
        let suffix = format!(".{}", kind.extension());
        if last.len() > suffix.len() && last.ends_with(&suffix) {
            Some(last[..last.len() - suffix.len()].to_string())
        } else {
            None
        }
    }

    /// The kind of a package by its extension. Swift `kind(of:)`.
    pub fn kind_of(path: &Path) -> Option<PackageKind> {
        let last = standardize(path).file_name()?.to_string_lossy().into_owned();
        // Like Foundation's pathExtension: the part after the last dot, not for a name that is only an extension
        let dot = last.rfind('.').filter(|i| *i > 0)?;
        PackageKind::from_extension(&last[dot + 1..])
    }

    /// Path of `file` relative to the folder `from` ("../../images/x.omaflake-image/disk.qcow2") – for qcow2 backing
    /// files, which QEMU resolves relative to the overlay's folder, so a package moved with what it points to still works
    pub fn relative_path(from: &Path, file: &Path) -> String {
        let a: Vec<String> = components(&resolve_symlinks(from));
        let b: Vec<String> = components(&resolve_symlinks(file));
        let mut common = 0;
        while common < a.len().min(b.len()) && a[common] == b[common] {
            common += 1;
        }
        let mut parts: Vec<String> = vec!["..".into(); a.len() - common];
        parts.extend(b[common..].iter().cloned());
        parts.join("/")
    }

    /// Copies a folder: on the same APFS volume as clones (instant, no extra space), elsewhere as files that stay
    /// sparse (an 80 GB macOS disk takes on the stick what it uses, not 80 GB). Symlinks are copied as links.
    /// OPEN (Linux): files are copied with `std::fs::copy` – no clones, not sparse.
    pub fn copy(from: &Path, to: &Path) -> Result<()> {
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).with_path(parent)?;
        }
        Self::copy_item(from, to)
    }

    /// Folders by hand, their permissions last: copyfile's own recursion creates a read-only folder (a macOS image)
    /// read-only first and then cannot fill it
    pub(crate) fn copy_item(from: &Path, to: &Path) -> Result<()> {
        let meta = fs::symlink_metadata(from).with_path(from)?;
        let ft = meta.file_type();
        if ft.is_dir() {
            fs::create_dir(to).with_path(to)?;
            let mut names: Vec<_> = fs::read_dir(from).with_path(from)?.flatten().map(|e| e.file_name()).collect();
            names.sort();
            for name in names {
                Self::copy_item(&from.join(&name), &to.join(&name))?;
            }
            fs::set_permissions(to, fs::Permissions::from_mode(meta.permissions().mode() & 0o7777)).with_path(to)?;
        } else if ft.is_symlink() {
            let target = fs::read_link(from).with_path(from)?;
            std::os::unix::fs::symlink(target, to).with_path(to)?;
        } else {
            copy_file(from, to)?;
        }
        Ok(())
    }

    /// Writes into a package that may be read-only (macOS images): opened for the write and closed again
    pub fn writable<T>(dir: &Path, body: impl FnOnce() -> Result<T>) -> Result<T> {
        let read_only = !is_writable(dir);
        if read_only {
            fs::set_permissions(dir, fs::Permissions::from_mode(0o755)).with_path(dir)?;
        }
        let result = body();
        if read_only {
            let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o555));
        }
        result
    }
}

fn components(p: &Path) -> Vec<String> {
    p.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect()
}

/// One file: APFS clone where possible, else a sparse copy with metadata (`copyfile(COPYFILE_ALL | COPYFILE_CLONE |
/// COPYFILE_DATA_SPARSE | COPYFILE_NOFOLLOW_SRC)`)
#[cfg(target_os = "macos")]
fn copy_file(from: &Path, to: &Path) -> Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let src = CString::new(from.as_os_str().as_bytes()).map_err(|_| vm_err!("path contains NUL"))?;
    let dst = CString::new(to.as_os_str().as_bytes()).map_err(|_| vm_err!("path contains NUL"))?;
    let flags = libc::COPYFILE_METADATA
        | libc::COPYFILE_DATA
        | libc::COPYFILE_CLONE
        | libc::COPYFILE_DATA_SPARSE
        | libc::COPYFILE_NOFOLLOW_SRC;
    // SAFETY: valid NUL-terminated paths, no copyfile state
    let rc = unsafe { libc::copyfile(src.as_ptr(), dst.as_ptr(), std::ptr::null_mut(), flags) };
    if rc != 0 {
        let e = std::io::Error::last_os_error();
        return Err(vm_err!("Cannot copy {} to {}: {}", from.display(), to.display(), crate::error::os_text(&e)));
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn copy_file(from: &Path, to: &Path) -> Result<()> {
    fs::copy(from, to)
        .map(|_| ())
        .map_err(|e| vm_err!("Cannot copy {} to {}: {}", from.display(), to.display(), crate::error::os_text(&e)))
}

/// manifest.json, schema `manifest` (plan B114). 0 → 1: `format` (always 1, plan B108) gave way to `$schema`
fn manifest_v0_to_v1(o: &mut Map<String, Value>) -> Result<()> {
    o.remove("format");
    Ok(())
}

static MANIFEST_STEPS: [Step; 1] = [manifest_v0_to_v1];

/// What a package says about itself (`manifest.json`): written on export, informational for the user and for
/// a newer omaflake (format version). The content is self-describing too (instance.json, image.json, recipe.toml).
/// Swift `Package.Manifest`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub kind: PackageKind,
    pub name: String,
    #[serde(with = "crate::json::iso8601")]
    pub exported: DateTime<Utc>,
    /// omaflake's version that wrote it
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub omaflake: Option<String>,
    #[serde(rename = "guestKind", default, skip_serializing_if = "Option::is_none")]
    pub guest_kind: Option<GuestKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipe: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// NixOS VM: its image is inside the package (`image.omaflake-image`)
    #[serde(rename = "containsImage", default, skip_serializing_if = "Option::is_none")]
    pub contains_image: Option<bool>,
}

impl Manifest {
    /// manifest.json (see `manifest_v0_to_v1`)
    pub const FILE_FORMAT: FileFormat = FileFormat::new("manifest", "manifest.json", 0, &MANIFEST_STEPS);

    /// Exported now, by this omaflake (Swift took the version from the app's Info.plist; here it is the crate's,
    /// which is kept equal to CFBundleShortVersionString)
    pub fn new(
        kind: PackageKind,
        name: &str,
        guest_kind: Option<GuestKind>,
        recipe: Option<String>,
        image: Option<String>,
        contains_image: Option<bool>,
    ) -> Self {
        Manifest {
            kind,
            name: name.into(),
            exported: crate::json::now(),
            omaflake: Some(crate::VERSION.to_string()),
            guest_kind,
            recipe,
            image,
            contains_image,
        }
    }

    pub fn path(package: &Path) -> PathBuf {
        package.join("manifest.json")
    }

    pub fn read(package: &Path) -> Option<Manifest> {
        let data = fs::read(Self::path(package)).ok()?;
        Self::FILE_FORMAT.decode::<Manifest>(&data).ok().map(|r| r.0)
    }

    pub fn write(&self, package: &Path) -> Result<()> {
        write_atomic(&Self::path(package), &Self::FILE_FORMAT.encode(self)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_relative_paths() {
        assert_eq!(
            Package::name_of(Path::new("/a/research.omaflake-vm"), PackageKind::Vm).as_deref(),
            Some("research")
        );
        assert_eq!(Package::name_of(Path::new("/a/research.omaflake-vm"), PackageKind::Image), None);
        assert_eq!(Package::name_of(Path::new("/a/.omaflake-vm"), PackageKind::Vm), None);
        assert_eq!(Package::kind_of(Path::new("/a/x.omaflake-recipe")), Some(PackageKind::Recipe));
        assert_eq!(Package::kind_of(Path::new("research")), None);
        assert_eq!(Package::kind_of(Path::new("x.omaflake-vm/")), Some(PackageKind::Vm));
        let rel = Package::relative_path(
            Path::new("/d/instances/a.omaflake-vm"),
            Path::new("/d/images/x.omaflake-image/disk.qcow2"),
        );
        assert_eq!(rel, "../../images/x.omaflake-image/disk.qcow2");
        assert_eq!(
            Package::relative_path(
                Path::new("/v/a.omaflake-vm"),
                Path::new("/v/a.omaflake-vm/image.omaflake-image/disk.qcow2")
            ),
            "image.omaflake-image/disk.qcow2"
        );
    }

    #[test]
    fn relative_paths_through_temporary_folders() {
        // /var → /private/var on macOS: both sides are resolved the same way
        let dir = tempfile::tempdir().unwrap();
        let vm = dir.path().join("instances/a.omaflake-vm");
        let img = dir.path().join("images/x.omaflake-image/disk.qcow2");
        fs::create_dir_all(&vm).unwrap();
        assert_eq!(Package::relative_path(&vm, &img), "../../images/x.omaflake-image/disk.qcow2");
    }

    #[test]
    fn copies_read_only_folders_and_links() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.omaflake-image");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("sub/disk.img"), "data").unwrap();
        std::os::unix::fs::symlink("sub/disk.img", src.join("link")).unwrap();
        fs::set_permissions(src.join("sub"), fs::Permissions::from_mode(0o555)).unwrap();
        fs::set_permissions(&src, fs::Permissions::from_mode(0o555)).unwrap();
        let dst = dir.path().join("out/copy.omaflake-image");
        Package::copy(&src, &dst).unwrap();
        assert_eq!(fs::read_to_string(dst.join("sub/disk.img")).unwrap(), "data");
        assert_eq!(fs::read_link(dst.join("link")).unwrap(), PathBuf::from("sub/disk.img"));
        assert_eq!(fs::metadata(&dst).unwrap().permissions().mode() & 0o777, 0o555);
        // writable() opens a read-only package for one write
        Package::writable(&dst, || fs::write(dst.join("x"), "y").map_err(crate::Error::from)).unwrap();
        assert_eq!(fs::metadata(&dst).unwrap().permissions().mode() & 0o777, 0o555);
        for d in [src.join("sub"), src.clone(), dst.join("sub"), dst.clone()] {
            fs::set_permissions(&d, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[test]
    fn manifest_round_trip_and_old_format() {
        let dir = tempfile::tempdir().unwrap();
        let m = Manifest::new(PackageKind::Vm, "a", Some(GuestKind::Nixos), None, None, Some(true));
        m.write(dir.path()).unwrap();
        let text = fs::read_to_string(Manifest::path(dir.path())).unwrap();
        assert!(text.contains("\"kind\" : \"omaflake-vm\"") && text.contains("\"containsImage\" : true"));
        assert_eq!(Manifest::read(dir.path()).unwrap(), m);
        fs::write(
            Manifest::path(dir.path()),
            r#"{"format":1,"kind":"omaflake-image","name":"x","exported":"2026-10-01T00:00:00Z"}"#,
        )
        .unwrap();
        let old = Manifest::read(dir.path()).unwrap();
        assert_eq!(old.kind, PackageKind::Image);
        assert_eq!(old.omaflake, None);
    }
}
