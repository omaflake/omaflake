// SPDX-License-Identifier: AGPL-3.0-or-later
//! OpenGL for QEMU from Nix on a Linux host that is not NixOS (plan B127).
//!
//! Nix programs load their GL driver from `/run/opengl-driver`, which only NixOS provides; elsewhere QEMU's GTK window
//! with `gl=on` aborts ("Couldn't find current GLX or EGL context", checked on Ubuntu 26.04). The distribution's Mesa
//! cannot fill in: it is linked against another glibc. So omaflake gives QEMU Mesa from QEMU's own nixpkgs – the one
//! the Nix profile installed QEMU from (its locked flake URL in the profile's manifest), else `nixpkgs` – built once
//! into `<data>/cache/gl/<QEMU's store name>` (a GC root), and points glvnd, the DRI loader and GBM at it.
//! On NixOS (`/run/opengl-driver` exists) and with a QEMU outside the Nix store nothing changes.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::paths::Paths;
use crate::shell::{RunOptions, Shell};

/// Where NixOS puts the GL driver for Nix programs
pub const OPENGL_DRIVER: &str = "/run/opengl-driver";

/// Whether QEMU at `qemu` (symlinks resolved) needs Mesa from Nix: it lives in the Nix store and the host has no
/// `opengl_driver` folder
pub fn needs_mesa(qemu: &Path, opengl_driver: &Path) -> bool {
    store_path(qemu).is_some() && !opengl_driver.exists()
}

/// The store path a file belongs to: `/nix/store/<hash>-<name>`
pub fn store_path(path: &Path) -> Option<PathBuf> {
    let rest = path.strip_prefix("/nix/store").ok()?;
    let first = rest.components().next()?;
    Some(Path::new("/nix/store").join(first))
}

/// The locked flake URL the Nix profile installed `store` from (manifest.json, versions 2 and 3); None when the
/// profile does not hold it
pub fn profile_flake(manifest: &Value, store: &Path) -> Option<String> {
    let elements: Vec<&Value> = match manifest.get("elements")? {
        Value::Object(map) => map.values().collect(),
        Value::Array(list) => list.iter().collect(),
        _ => return None,
    };
    let store = store.to_string_lossy();
    elements
        .into_iter()
        .find(|e| e.get("storePaths").and_then(Value::as_array).is_some_and(|p| p.iter().any(|s| s == store.as_ref())))
        .and_then(|e| e.get("url").and_then(Value::as_str))
        .map(str::to_string)
}

/// The environment that makes QEMU load `mesa` (a Mesa output: lib/dri, lib/gbm, share/glvnd), in front of an
/// `LD_LIBRARY_PATH` already set
pub fn environment(mesa: &Path, ld_library_path: Option<&str>) -> Vec<(String, String)> {
    let lib = mesa.join("lib").to_string_lossy().into_owned();
    let ld = match ld_library_path.filter(|l| !l.is_empty()) {
        Some(old) => format!("{lib}:{old}"),
        None => lib,
    };
    vec![
        (
            "__EGL_VENDOR_LIBRARY_FILENAMES".into(),
            mesa.join("share/glvnd/egl_vendor.d/50_mesa.json").to_string_lossy().into_owned(),
        ),
        ("__GLX_VENDOR_LIBRARY_NAME".into(), "mesa".into()),
        ("LIBGL_DRIVERS_PATH".into(), mesa.join("lib/dri").to_string_lossy().into_owned()),
        ("GBM_BACKENDS_PATH".into(), mesa.join("lib/gbm").to_string_lossy().into_owned()),
        ("LD_LIBRARY_PATH".into(), ld),
    ]
}

/// Whether `mesa` looks like a Mesa output with what `environment` points at
pub fn is_mesa(mesa: &Path) -> bool {
    mesa.join("share/glvnd/egl_vendor.d/50_mesa.json").is_file() && mesa.join("lib/dri").is_dir()
}

/// Mesa for QEMU at `qemu` (symlinks resolved): the cached build for this QEMU, else built now with `nix` from QEMU's
/// nixpkgs (the profile's flake, else `nixpkgs`). Err: why there is none (QEMU then starts without it).
pub fn mesa_for(paths: &Paths, nix: &str, qemu: &Path) -> Result<PathBuf, String> {
    let store = store_path(qemu).ok_or_else(|| format!("{} is not in the Nix store", qemu.display()))?;
    let name = store.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let link = paths.cache().join("gl").join(&name);
    if let Ok(target) = std::fs::canonicalize(&link) {
        if is_mesa(&target) {
            return Ok(target);
        }
    }
    let flake = profile_manifest().and_then(|m| profile_flake(&m, &store)).unwrap_or_else(|| "nixpkgs".into());
    if let Some(parent) = link.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let installable = format!("{flake}#mesa");
    let link_text = link.to_string_lossy().into_owned();
    let args = ["build", "--out-link", link_text.as_str(), "--print-out-paths", installable.as_str()];
    let result = Shell::run(nix, &args, &RunOptions::default()).map_err(|e| e.to_string())?;
    if result.status != 0 {
        let last = result.stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
        return Err(format!("nix build {installable} failed: {last}"));
    }
    let mesa = result.stdout.lines().next().map(PathBuf::from).unwrap_or_default();
    if is_mesa(&mesa) {
        Ok(mesa)
    } else {
        Err(format!("{installable} has no GL driver ({})", mesa.display()))
    }
}

/// The user's Nix profile manifest: ~/.nix-profile, else $XDG_STATE_HOME/nix/profiles/profile
fn profile_manifest() -> Option<Value> {
    let home = crate::platform::home_dir();
    let state = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/state"));
    [home.join(".nix-profile/manifest.json"), state.join("nix/profiles/profile/manifest.json")]
        .iter()
        .find_map(|p| std::fs::read_to_string(p).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const QEMU: &str = "/nix/store/cnwc2zbiwjn3mw9ds7bkr0f8vyyswj01-qemu-11.1.2";

    #[test]
    fn store_paths() {
        let bin = Path::new(QEMU).join("bin/qemu-system-x86_64");
        assert_eq!(store_path(&bin), Some(PathBuf::from(QEMU)));
        assert_eq!(store_path(Path::new("/usr/bin/qemu-system-x86_64")), None);
    }

    #[test]
    fn mesa_only_for_nix_qemu_without_the_driver_folder() {
        let dir = tempfile::tempdir().unwrap();
        let bin = Path::new(QEMU).join("bin/qemu-system-x86_64");
        assert!(needs_mesa(&bin, &dir.path().join("missing")));
        assert!(!needs_mesa(&bin, dir.path()), "NixOS: /run/opengl-driver exists");
        assert!(!needs_mesa(Path::new("/usr/bin/qemu-system-x86_64"), &dir.path().join("missing")));
    }

    #[test]
    fn flake_of_the_profile_element() {
        let url = "https://releases.nixos.org/nixpkgs/nixpkgs-26.11pre1100332.901c54c08bdc/nixexprs.tar.zst";
        let v3 = json!({"version": 3, "elements": {
            "omaflake-gui": {"storePaths": ["/nix/store/aaa-omaflake-gui-0.3.1"], "url": "github:omaflake/omaflake/abc"},
            "qemu": {"storePaths": [QEMU], "url": url}
        }});
        assert_eq!(profile_flake(&v3, Path::new(QEMU)).as_deref(), Some(url));
        let v2 = json!({"version": 2, "elements": [{"storePaths": [QEMU], "url": url}]});
        assert_eq!(profile_flake(&v2, Path::new(QEMU)).as_deref(), Some(url));
        assert_eq!(profile_flake(&v3, Path::new("/nix/store/zzz-qemu-10.0.0")), None);
    }

    #[test]
    fn environment_points_at_mesa() {
        let mesa = Path::new("/nix/store/jg8f97fhfmkpccrm3lsxlavkivpjbs72-mesa-26.2.5");
        let env = environment(mesa, Some("/opt/lib"));
        let get = |k: &str| env.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
        assert_eq!(
            get("__EGL_VENDOR_LIBRARY_FILENAMES"),
            Some(&*format!("{}/share/glvnd/egl_vendor.d/50_mesa.json", mesa.display()))
        );
        assert_eq!(get("LIBGL_DRIVERS_PATH"), Some(&*format!("{}/lib/dri", mesa.display())));
        assert_eq!(get("GBM_BACKENDS_PATH"), Some(&*format!("{}/lib/gbm", mesa.display())));
        assert_eq!(get("__GLX_VENDOR_LIBRARY_NAME"), Some("mesa"));
        assert_eq!(get("LD_LIBRARY_PATH"), Some(&*format!("{}/lib:/opt/lib", mesa.display())));
        let fresh = environment(mesa, Some(""));
        assert!(fresh.iter().any(|(k, v)| k == "LD_LIBRARY_PATH" && *v == format!("{}/lib", mesa.display())));
    }

    #[test]
    fn a_cached_mesa_is_used_without_nix() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("data"), None);
        let mesa = dir.path().join("mesa");
        std::fs::create_dir_all(mesa.join("share/glvnd/egl_vendor.d")).unwrap();
        std::fs::write(mesa.join("share/glvnd/egl_vendor.d/50_mesa.json"), "{}").unwrap();
        std::fs::create_dir_all(mesa.join("lib/dri")).unwrap();
        let link = paths.cache().join("gl").join(Path::new(QEMU).file_name().unwrap());
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&mesa, &link).unwrap();
        let qemu = Path::new(QEMU).join("bin/qemu-system-x86_64");
        let found = mesa_for(&paths, "/nonexistent/nix", &qemu).unwrap();
        assert_eq!(found, std::fs::canonicalize(&mesa).unwrap());
    }
}
