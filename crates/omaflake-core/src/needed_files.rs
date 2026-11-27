// SPDX-License-Identifier: AGPL-3.0-or-later
//! Files a module needs to be built (plan B198) – module.toml `[[files]]` (module/v3), such as Xcode, which nobody may
//! pass on and so no cache has. The module's Nix asks for a fixed-output path (`requireFile`: a name and a hash); this
//! is what omaflake does about it before a build: the path is known from the name and the hash (no Nix needed to know
//! whether it is there), a file the module names is looked for in the places it names (`~/Downloads/Xcode_27.2.xip`),
//! and a file found or chosen is unpacked (`.xip`, `.zip`), added to the Nix Store (`nix-store --add-fixed`) and
//! checked to land at that path – any other content lands elsewhere and is refused.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::cancellation::Cancellation;
use crate::error::{IoContext, Result};
use crate::shell::{RunOptions, Shell};
use crate::vm_err;
use omaflake_sdk::codes;

/// The Nix Store's folder (the path is part of every store path's hash)
const STORE: &str = "/nix/store";

/// One `[[files]]` entry of module.toml
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NeededFile {
    /// Its name in the Nix Store – the `name` of the module's `requireFile` ("Xcode.app")
    pub name: String,
    /// Its hash as Nix writes it: SRI ("sha256-…", base64) – the `hash` of the module's `requireFile`
    pub hash: String,
    /// "recursive" (a folder or file as a NAR, the default – `nix-store --add-fixed --recursive`) or "flat" (a file)
    #[serde(default = "recursive", skip_serializing_if = "is_recursive")]
    pub mode: String,
    /// What it is, for people ("Xcode 27.2"); the name when empty
    #[serde(default)]
    pub title: String,
    /// The file names it may come as on this computer, first the one a download gives ("Xcode_27.2.xip",
    /// "Xcode.app")
    #[serde(default)]
    pub from: Vec<String>,
    /// Where to look for them ("~/Downloads", "/Applications")
    #[serde(default)]
    pub search: Vec<String>,
    /// Where to download it
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

fn recursive() -> String {
    "recursive".into()
}

fn is_recursive(mode: &str) -> bool {
    mode == "recursive"
}

impl NeededFile {
    pub fn title(&self) -> &str {
        if self.title.is_empty() { &self.name } else { &self.title }
    }

    /// What is wrong with the entry (None: fine) – checked when the module is read
    pub fn problem(&self) -> Option<String> {
        if self.name.is_empty() || self.name.contains('/') {
            return Some(format!("files: name \"{}\" is not a Nix Store name", self.name));
        }
        if self.mode != "recursive" && self.mode != "flat" {
            return Some(format!("files: mode \"{}\" is recursive or flat", self.mode));
        }
        if sha256_of_sri(&self.hash).is_none() {
            return Some(format!("files: hash \"{}\" is not a sha256 hash in SRI form (sha256-…)", self.hash));
        }
        None
    }

    /// Its path in the Nix Store, as Nix computes a fixed-output path (`makeFixedOutputPath`): recursive sha256 is a
    /// "source" path of the hash; flat a path of type "output:out" of the hash of "fixed:out:sha256:<hash>:"
    pub fn store_path(&self) -> Option<String> {
        let hash = sha256_of_sri(&self.hash)?;
        let path_hash = if self.mode == "flat" {
            let inner = Sha256::digest(format!("fixed:out:sha256:{}:", crate::encoding::hex(hash)).as_bytes());
            store_path_hash("output:out", &inner, &self.name)
        } else {
            store_path_hash("source", &hash, &self.name)
        };
        Some(format!("{STORE}/{path_hash}-{}", self.name))
    }

    /// It is in the Nix Store
    pub fn present(&self) -> bool {
        self.store_path().is_some_and(|p| Path::new(&p).exists())
    }

    /// The first of its names found in the places it names (`~` is the home folder)
    pub fn find(&self) -> Option<PathBuf> {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        for dir in &self.search {
            let dir = match (dir.strip_prefix("~/"), &home) {
                (Some(rest), Some(h)) => h.join(rest),
                _ if dir == "~" => home.clone()?,
                _ => PathBuf::from(dir),
            };
            for name in &self.from {
                let p = dir.join(name);
                if p.exists() {
                    return Some(p);
                }
            }
        }
        None
    }

    /// Adds `source` – the file, a folder or app, or an archive of it (`.xip`, `.zip`) – to the Nix Store, unpacked in
    /// `work` (a folder of its own, removed afterwards); its path when it lands where the hash says, else an error
    /// (and what landed elsewhere is removed from the store again)
    pub fn add(
        &self,
        source: &Path,
        nix: &str,
        work: &Path,
        cancel: &Cancellation,
        log: &dyn Fn(&str),
    ) -> Result<String> {
        let expected = self.store_path().ok_or_else(|| vm_err!("{}: no valid hash", self.name))?;
        if !source.exists() {
            return Err(vm_err!(codes::NOT_FOUND, "{} is not there", source.display()));
        }
        if work.exists() {
            std::fs::remove_dir_all(work).with_path(work)?;
        }
        std::fs::create_dir_all(work).with_path(work)?;
        let result = self.add_in(source, nix, &expected, work, cancel, log);
        let _ = std::fs::remove_dir_all(work);
        result
    }

    fn add_in(
        &self,
        source: &Path,
        nix: &str,
        expected: &str,
        work: &Path,
        cancel: &Cancellation,
        log: &dyn Fn(&str),
    ) -> Result<String> {
        let opts = RunOptions { cancel: Some(cancel), ..Default::default() };
        let lower = source.to_string_lossy().to_lowercase();
        let unpacked = if lower.ends_with(".xip") || lower.ends_with(".zip") {
            let into = work.join("unpacked");
            std::fs::create_dir_all(&into).with_path(&into)?;
            log(&format!("Unpacking {} (a while for large files)", source.display()));
            let src = source.to_string_lossy().to_string();
            let r = if lower.ends_with(".xip") {
                // xip unpacks into the folder it runs in
                Shell::run("/usr/bin/xip", &["--expand", &src], &RunOptions { cwd: Some(&into), ..opts })?
            } else {
                Shell::run("/usr/bin/ditto", &["-x", "-k", &src, &into.to_string_lossy()], &opts)?
            };
            if r.status != 0 {
                return Err(vm_err!("Could not unpack {}: {}", source.display(), r.stderr.trim()));
            }
            // The name the module needs inside, else the archive's only entry
            let named = into.join(&self.name);
            if named.exists() {
                named
            } else {
                let entries: Vec<PathBuf> = std::fs::read_dir(&into)
                    .with_path(&into)?
                    .filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')))
                    .collect();
                match entries.as_slice() {
                    [one] => one.clone(),
                    _ => return Err(vm_err!("{} holds no {}", source.display(), &self.name)),
                }
            }
        } else {
            source.to_path_buf()
        };
        // The store names the path after the file: under another name it is cloned to the module's (APFS: no copy)
        let named = if unpacked.file_name().is_some_and(|n| n == self.name.as_str()) {
            unpacked
        } else {
            let stage = work.join("named");
            std::fs::create_dir_all(&stage).with_path(&stage)?;
            let to = stage.join(&self.name);
            let r = Shell::run(
                "/bin/cp",
                &["-cR".to_string(), unpacked.to_string_lossy().into(), to.to_string_lossy().into()],
                &opts,
            )?;
            if r.status != 0 {
                return Err(vm_err!("Could not copy {}: {}", unpacked.display(), r.stderr.trim()));
            }
            to
        };
        log(&format!("Adding {} to the Nix Store", self.title()));
        let nix_store = Path::new(nix).with_file_name("nix-store");
        let nix_store = nix_store.to_string_lossy().to_string();
        let mut args = vec!["--add-fixed".to_string()];
        if self.mode != "flat" {
            args.push("--recursive".into());
        }
        args.extend(["sha256".to_string(), named.to_string_lossy().into()]);
        let r = Shell::run(&nix_store, &args, &opts)?;
        if r.status != 0 {
            return Err(vm_err!("nix-store could not add {}: {}", self.title(), r.stderr.trim()));
        }
        let landed = r.stdout.trim().to_string();
        if landed != expected {
            // Not the file the module needs (another version, or changed): out of the store again
            let _ = Shell::run(&nix_store, &["--delete", &landed], &RunOptions::default());
            return Err(vm_err!(
                codes::PREREQUISITE,
                "{} is not {} – its content differs from what the module needs (another version?)",
                source.display(),
                self.title()
            ));
        }
        log(&format!("✓ {} is in the Nix Store", self.title()));
        Ok(landed)
    }
}

/// The 32 bytes of an SRI sha256 hash ("sha256-<base64>")
fn sha256_of_sri(sri: &str) -> Option<[u8; 32]> {
    let bytes = crate::encoding::from_base64(sri.strip_prefix("sha256-")?)?;
    bytes.try_into().ok()
}

/// Nix's `makeStorePath`: sha256 of "<type>:sha256:<hex>:/nix/store:<name>", folded to 20 bytes, in Nix's base32
fn store_path_hash(kind: &str, hash: &[u8], name: &str) -> String {
    let fingerprint = format!("{kind}:sha256:{}:{STORE}:{name}", crate::encoding::hex(hash));
    let full = Sha256::digest(fingerprint.as_bytes());
    let mut folded = [0u8; 20];
    for (i, b) in full.iter().enumerate() {
        folded[i % 20] ^= b;
    }
    nix32(&folded)
}

/// Nix's base32 (its own alphabet, the last byte first)
fn nix32(bytes: &[u8]) -> String {
    const CHARS: &[u8] = b"0123456789abcdfghijklmnpqrsvwxyz";
    let len = (bytes.len() * 8 - 1) / 5 + 1;
    (0..len)
        .rev()
        .map(|n| {
            let b = n * 5;
            let (i, j) = (b / 8, b % 8);
            let low = u16::from(bytes[i]) >> j;
            let high = bytes.get(i + 1).map_or(0, |&x| u16::from(x) << (8 - j));
            CHARS[usize::from((low | high) & 0x1f)] as char
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn xcode() -> NeededFile {
        NeededFile {
            name: "Xcode.app".into(),
            hash: "sha256-zs5OMdddog5+11qHlkkDMC7liJ4qllxmF4qiVy4zdf8=".into(),
            mode: "recursive".into(),
            title: "Xcode 27.2".into(),
            from: vec!["Xcode_27.2.xip".into(), "Xcode.app".into()],
            search: vec!["~/Downloads".into()],
            url: None,
        }
    }

    /// The path nix-store printed for Xcode 27.2 (`nix-store --add-fixed --recursive sha256 Xcode.app`, 2026-11-27)
    #[test]
    fn the_store_path_is_the_one_nix_gives() {
        assert_eq!(xcode().store_path().as_deref(), Some("/nix/store/dxy8jk1h5x4dkzxzr1j4286pxgpgc6zc-Xcode.app"));
        assert_eq!(xcode().problem(), None);
    }

    /// A flat file: what `nix-store --print-fixed-path sha256 <hex> hello.txt` gives for a file holding "hello\n"
    /// (`nix hash file`: sha256-WJG1…vgM=)
    #[test]
    fn a_flat_file_has_its_own_path() {
        let f = NeededFile {
            name: "hello.txt".into(),
            hash: "sha256-WJG1tSLV3whtD/CxEPvZ0hu0/HFjrzTQgoai6Eb2vgM=".into(),
            mode: "flat".into(),
            ..xcode()
        };
        assert_eq!(f.store_path().as_deref(), Some("/nix/store/gy454w1cxaq731grqwylhzf4pp9r5izh-hello.txt"));
    }

    #[test]
    fn bad_entries_are_named() {
        assert!(NeededFile { hash: "md5-abc".into(), ..xcode() }.problem().is_some());
        assert!(NeededFile { name: "a/b".into(), ..xcode() }.problem().is_some());
        assert!(NeededFile { mode: "zip".into(), ..xcode() }.problem().is_some());
    }

    #[test]
    fn it_is_found_in_the_places_it_names() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = xcode();
        f.search = vec![dir.path().join("nothing").to_string_lossy().into(), dir.path().to_string_lossy().into()];
        assert_eq!(f.find(), None);
        std::fs::create_dir(dir.path().join("Xcode.app")).unwrap();
        assert_eq!(f.find(), Some(dir.path().join("Xcode.app")));
        std::fs::write(dir.path().join("Xcode_27.2.xip"), b"x").unwrap();
        assert_eq!(f.find(), Some(dir.path().join("Xcode_27.2.xip")), "the download first");
    }
}
