// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of ImageStore.swift – work package F1. Done: the types, the image.json format, `ImageInfo::new`/`line`,
//! `info_file`, `image_name`. Skeleton: the rest (`todo!`).
//!
//! Images ("Vorlagen") under <data folder>/images: one package per image (`<name>.omaflake-image`, plan B108) with an
//! `image.json` that says what it is (recipe, author, guest kind, name, flake, revision, time). Nothing is derived
//! from folder names. Packages are self-contained, so they can be copied to a stick and back:
//!   NixOS: `image.json` + `disk.qcow2` (copied out of the Nix store, read-only)
//!   macOS: the VM folder (read-only) + `image.json`
//! `<name>` = `<author>.<kind>.<name>@<words>` (plan B110), the file form of its omaflake ID – the words come from the
//! store path, so the name is unique per content and several images of one recipe live side by side;
//! `current/<line>` points to the newest of a recipe's line.
#![allow(unused_variables, dead_code)]

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::cancellation::Cancellation;
use crate::error::Result;
use crate::file_format::{FileFormat, Step, UnreadableFile};
use crate::instance_store::InstanceStore;
use crate::models::{GuestKind, RecipeDefaults, RecipeId};
use crate::omaflake_id::OmaflakeId;
use crate::paths::Paths;
use crate::platform::Platform;
use crate::spawn::Pid;
use crate::Log;

/// image.json, schema `image` (plan B114). 0 → 1: "flavor" became "recipe" (plan B92), `system` added – every
/// image before it was built on a Mac with Apple silicon
fn image_v0_to_v1(o: &mut Map<String, Value>) -> Result<()> {
    FileFormat::rename_key("flavor", "recipe", o);
    let kind = o.get("kind").and_then(Value::as_str).and_then(GuestKind::parse).unwrap_or(GuestKind::Nixos);
    o.insert("system".into(), Value::String(Platform::legacy_system(kind).into()));
    Ok(())
}

static IMAGE_STEPS: [Step; 1] = [image_v0_to_v1];

/// Contents of `image.json` (Swift `ImageStore.Info`). Keys: recipe, recipeSource, author, kind, system, name,
/// source, revision, storePath, created (`.iso8601`); None fields left out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageInfo {
    /// Recipe ID "<author>.<kind>.<name>@<hash>" (plan B110; older images: without the hash)
    pub recipe: String,
    /// The recipe source it was built from (`RecipeSource::id`) – with the recipe's base its line (plan B110)
    #[serde(rename = "recipeSource", default, skip_serializing_if = "Option::is_none")]
    pub recipe_source: Option<String>,
    pub author: String,
    pub kind: GuestKind,
    /// What it was built for (`Platform`): "aarch64-linux", "x86_64-linux", "aarch64-darwin"
    pub system: String,
    pub name: String,
    /// Flake the image was built from (folder or URL)
    pub source: String,
    /// The recipe's state (plan B107): from a local folder its git revision ("-dirty" with uncommitted changes in
    /// the recipe's folder, "nogit" outside git), downloaded the start of its catalog version; images from before
    /// B107 carry the image flake's revision
    pub revision: String,
    /// NixOS: the store path of the image; macOS: the nix-darwin system it was activated with
    #[serde(rename = "storePath", default, skip_serializing_if = "Option::is_none")]
    pub store_path: Option<String>,
    #[serde(with = "crate::json::iso8601")]
    pub created: DateTime<Utc>,
}

impl ImageInfo {
    /// image.json (see `image_v0_to_v1`)
    pub const FILE_FORMAT: FileFormat = FileFormat::new("image", "image.json", 0, &IMAGE_STEPS);

    /// Author and name come from the recipe ID ("local" and the ID itself when it has no parts); pass
    /// `json::now()` for Swift's default `created: Date()`
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        recipe: &str,
        recipe_source: Option<String>,
        kind: GuestKind,
        system: &str,
        source: &str,
        revision: &str,
        store_path: Option<String>,
        created: DateTime<Utc>,
    ) -> Self {
        let p = RecipeId::parts(recipe);
        ImageInfo {
            recipe: recipe.into(),
            recipe_source,
            author: p.as_ref().map(|p| p.author.clone()).unwrap_or_else(|| "local".into()),
            kind,
            system: system.into(),
            name: p.map(|p| p.name).unwrap_or_else(|| recipe.to_string()),
            source: source.into(),
            revision: revision.into(),
            store_path,
            created,
        }
    }

    /// The recipe's line: images of one line replace each other (current, outdated, upgrade, retention)
    pub fn line(&self) -> String {
        RecipeId::line_key(&self.recipe, self.recipe_source.as_deref().unwrap_or(""))
    }
}

/// An image (Swift `ImageStore.Image`)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub name: String,
    pub info: ImageInfo,
    /// NixOS: the .qcow2 file; macOS: the VM folder (spec §5)
    pub path: PathBuf,
}

impl Image {
    pub fn recipe(&self) -> &str {
        &self.info.recipe
    }

    pub fn kind(&self) -> GuestKind {
        self.info.kind
    }
}

#[derive(Debug, Clone)]
pub struct ImageStore {
    pub paths: Paths,
}

impl ImageStore {
    /// Retention (plan B52): after a build, a recipe keeps its current image and the one before it
    pub const KEEP_PREVIOUS: usize = 1;

    pub fn new(paths: Paths) -> Self {
        ImageStore { paths }
    }

    /// `<dir>/image.json`
    pub fn info_file(dir: &Path) -> PathBuf {
        dir.join("image.json")
    }

    /// image.json, brought up to date (and written back) if it is older; None if there is none or it cannot be read
    pub fn read_info(dir: &Path) -> Option<ImageInfo> {
        todo!("port from ImageStore.swift")
    }

    /// image.json as `read_info`, but with the reason when it cannot be read (too old, too new, damaged); None if there
    /// is none
    pub(crate) fn checked_info(dir: &Path) -> Result<Option<ImageInfo>> {
        todo!("port from ImageStore.swift")
    }

    /// Image packages this omaflake cannot read – written by a much older or a newer omaflake, or damaged (plan B114)
    pub fn unreadable(&self) -> Vec<UnreadableFile> {
        todo!("port from ImageStore.swift")
    }

    /// Writes image.json (`ImageInfo::FILE_FORMAT.encode`); a read-only (macOS) folder is opened for it (0755) and
    /// closed again (0555), the file then 0444.
    pub fn write_info(info: &ImageInfo, dir: &Path) -> Result<()> {
        todo!("port from ImageStore.swift")
    }

    /// Recipe defaults from the flake (`nix eval --json <flakeRef>#lib.recipeDefaults` + `guest_arguments`).
    pub fn recipe_defaults(&self, flake: &str) -> Result<HashMap<String, RecipeDefaults>> {
        todo!("port from ImageStore.swift")
    }

    /// The store path a build of `recipe` would produce now – evaluation only, nothing is built. NixOS: the disk image,
    /// macOS: the darwin system; both are what image.json records as `storePath` (plan B82).
    pub fn expected_store_path(&self, recipe: &str, kind: GuestKind, flake: &str) -> Result<String> {
        todo!("port from ImageStore.swift")
    }

    /// What a build of each recipe (with an image) would give now – recipes that fail to evaluate are left out
    pub fn expected_store_paths(
        &self,
        recipes: &HashMap<String, RecipeDefaults>,
        flake: &str,
    ) -> HashMap<String, String> {
        todo!("port from ImageStore.swift")
    }

    /// Recipes whose current image no longer matches the recipe (its files, the modules it uses, nixpkgs, the guest
    /// settings): a build now would give another store path.
    pub fn outdated(&self, recipes: &HashMap<String, RecipeDefaults>, flake: &str) -> HashSet<String> {
        todo!("port from ImageStore.swift")
    }

    /// The flake output of a NixOS recipe's image for this host: `images.<system>."<recipe>"` (plan B114)
    pub(crate) fn image_attribute(recipe: &str) -> Result<String> {
        todo!("port from ImageStore.swift")
    }

    /// `nix build <flake>#images.<system>.<recipe>`, copy the disk out of the store, switch `current/<line>`.
    /// The build log goes line by line to `log`; `started` hands over the nix process for cancelling; `cancel` stops
    /// it. A build is run a second time when the Linux builder collected garbage during the first ("running
    /// auto-GC", plan B79). Exit status 15 or a cancelled token: "Build cancelled".
    pub fn build(
        &self,
        recipe: &str,
        flake: &str,
        started: Option<&dyn Fn(Pid)>,
        cancel: &Cancellation,
        log: &Log,
    ) -> Result<Image> {
        todo!("port from ImageStore.swift")
    }

    /// The qcow2 of a NixOS store path → `<package>/disk.qcow2` (read-only). The package then needs no GC root and
    /// no Nix store – it can be copied anywhere (plan B108). Written under a temporary name first, so an interrupted
    /// copy never looks like an image.
    pub(crate) fn copy_disk(from_store: &Path, into: &Path) -> Result<()> {
        todo!("port from ImageStore.swift")
    }

    /// Flake reference for `nix`: a folder without its own git repository (like `recipes/` in the
    /// checkout) becomes `path:<folder>`, so Nix also sees its git-ignored `local/` settings.
    /// A git reference would copy only tracked files. Flake URLs (`path:`, `github:`, …) stay as they are.
    pub fn flake_ref(flake: &str) -> String {
        todo!("port from ImageStore.swift")
    }

    /// "omaflake.nixos.minimal" + a store path → "omaflake.nixos.minimal@cedar-mango-lift-…" (12 words, plan B111; the
    /// revision is in image.json)
    pub fn image_name(recipe: &str, store_path: &str) -> String {
        format!("{}@{}", RecipeId::base(recipe), OmaflakeId::words_of(store_path, OmaflakeId::LENGTH))
    }

    /// The state of a recipe for the image's name and image.json: where the recipe comes from decides (plan B107);
    /// unknown recipes fall back to the image flake's revision
    pub fn recipe_revision(&self, recipe: &str, flake: &str) -> String {
        todo!("port from ImageStore.swift")
    }

    /// Short git hash of the folder's repository, "-dirty" with uncommitted changes in that folder, "nogit" without
    /// git (never /usr/bin/git without developer tools: `Prerequisites::locate_git`)
    pub(crate) fn git_rev(&self, folder: &str) -> String {
        todo!("port from ImageStore.swift")
    }

    /// The line of a recipe ID (plan B110): from the recipe there is, else from an image built from it
    pub fn line_of(&self, recipe: &str) -> String {
        todo!("port from ImageStore.swift")
    }

    /// Points `current/<line>` to an image – the newest of its recipe's line, used for new VMs (plan B56, B110).
    /// The link is relative: `../<name>.omaflake-image`.
    pub fn set_current(&self, image_name: &str) -> Result<()> {
        todo!("port from ImageStore.swift")
    }

    /// Name of the current image of a recipe's line, of any guest kind (macOS images are VM folders, not qcow2).
    pub fn current_name(&self, recipe: &str) -> Option<String> {
        todo!("port from ImageStore.swift")
    }

    /// Swift `currentName(line:)`
    pub fn current_name_in_line(&self, line: &str) -> Option<String> {
        todo!("port from ImageStore.swift")
    }

    /// Link target "../<name>.omaflake-image" → "<name>" (links from before B108 have no extension)
    pub(crate) fn image_name_of_link(dest: &str) -> String {
        todo!("port from ImageStore.swift")
    }

    /// Current image of a recipe (None if never built).
    pub fn current(&self, recipe: &str) -> Option<Image> {
        todo!("port from ImageStore.swift")
    }

    pub fn image(&self, name: &str) -> Result<Image> {
        todo!("port from ImageStore.swift")
    }

    /// An image package anywhere (in the data folder, inside a VM package, on a stick). Swift `image(at:)`.
    pub fn image_at(&self, dir: &Path) -> Result<Image> {
        todo!("port from ImageStore.swift")
    }

    /// Legacy folder names `<recipe>-<git-rev>[-dirty]-<hash>` → recipe part (used only to migrate old images).
    pub(crate) fn recipe_of_image(name: &str) -> String {
        todo!("port from ImageStore.swift")
    }

    /// Space the image takes on disk: allocated blocks (sparse disks count what they use; `st_blocks × 512`),
    /// symlinks followed. None when it can't be read.
    pub fn disk_usage(image: &Image) -> Option<i64> {
        todo!("port from ImageStore.swift")
    }

    /// Images of a recipe's line, newest first. Swift `images(of:)`.
    pub fn images_of(&self, recipe: &str) -> Vec<Image> {
        todo!("port from ImageStore.swift")
    }

    /// Swift `images(inLine:)`, newest first
    pub fn images_in_line(&self, line: &str) -> Vec<Image> {
        todo!("port from ImageStore.swift")
    }

    /// Deletes an image (NixOS: its GC root goes with it, the store path can then be collected). Refused while
    /// instances use it: NixOS overlays are built on it, macOS instances clone it again on reset (ephemeral ones on
    /// every start). If it was the recipe's current image, `current/` moves to the next newest image of that recipe.
    pub fn remove(&self, name: &str, instances: &InstanceStore) -> Result<()> {
        todo!("port from ImageStore.swift")
    }

    /// Retention (plan B52): after a build, a recipe keeps its current image, the one before it (to go back to) and
    /// every image a VM still uses; older ones are deleted. `used` must list the images of *all* instances – callers
    /// skip pruning when they cannot read them. Returns the deleted names.
    pub fn prune(&self, recipe: &str, used: &HashSet<String>) -> Vec<String> {
        todo!("port from ImageStore.swift")
    }

    /// All image packages with an image.json (not `current/` and not unfinished macOS builds `.build-*`), by name.
    pub fn list(&self) -> Vec<Image> {
        todo!("port from ImageStore.swift")
    }
}
