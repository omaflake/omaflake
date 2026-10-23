// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of ImageMigration.swift – work package F1. Skeleton (`todo!`). Methods on `ImageStore`.
#![allow(unused_variables, dead_code)]

use std::path::Path;

use crate::image_store::ImageStore;
use crate::instance_store::InstanceStore;
use crate::models::GuestKind;

impl ImageStore {
    /// Older layouts (before image.json, 2026-10-17): NixOS images were symlinks into the Nix store and recipes were
    /// called "minimal", "mac-minimal", "nixos-minimal", … Turns them into image folders with image.json (recipe
    /// "omaflake.<kind>.<name>"), renames `current/` links and points instances at the new paths (qcow2 overlays are
    /// rebased). Then everything becomes a package (plan B108, `migrate_packages`). Idempotent and cheap when there is
    /// nothing to do; returns what it changed. Starts with `Settings::migrate_recipes_folder`.
    pub fn migrate_legacy(&self) -> Vec<String> {
        todo!("port from ImageMigration.swift")
    }

    /// Plan B110/B111: recipe IDs carry their content hash as 12 words and images their recipe's source (the line).
    /// Downloaded recipes move to `recipes/<id>/`; images, `current/` links and VMs of older IDs (none, or fewer
    /// words) get the ID and source of the recipe there is now – the one whose words they start with, else the first of
    /// that base; an image built from an older state is then outdated by its store path, as before. Recipes that are
    /// gone stay as they are. VMs get an identity of 12 words (a shorter one is extended, so it starts the same).
    pub(crate) fn migrate_recipe_ids(&self, store: &InstanceStore) -> Vec<String> {
        todo!("port from ImageMigration.swift")
    }

    /// Plan B110/B111: image packages are named `<base>@<12 words of the store path>` – older names are renamed; the VMs on
    /// them follow (their `image`, and NixOS overlays their relative backing file). An image a running VM uses waits.
    pub(crate) fn rename_images(&self, store: &InstanceStore) -> Vec<String> {
        todo!("port from ImageMigration.swift")
    }

    /// The backing file a qcow2 names (as written in it: `qemu-img info -U --output=json`, "backing-filename"), None
    /// without one
    pub(crate) fn backing_file(disk: &Path, qemu_img: &str) -> Option<String> {
        todo!("port from ImageMigration.swift")
    }

    /// Plan B108: image folders → `<name>.omaflake-image` (NixOS: the qcow2 is copied out of the Nix store, the GC
    /// root goes), `current/` links follow, instance folders → `<name>.omaflake-vm` with their runtime files moved to
    /// `run/<name>/`. Skipped as a whole while a VM of the old layout runs (its files are open) – next start then.
    pub(crate) fn migrate_packages(&self, qemu_img: &str) -> Vec<String> {
        todo!("port from ImageMigration.swift")
    }

    /// "minimal", "nixos-minimal", "mac-minimal", "macos-minimal" → "omaflake.<kind>.minimal"; IDs stay as they are
    pub(crate) fn legacy_recipe_id(recipe: &str, kind: GuestKind) -> String {
        todo!("port from ImageMigration.swift")
    }

    /// "<recipe>-<rev>[-dirty]-<hash>" → "<rev>[-dirty]"; "unknown" when the name does not start with the recipe
    pub(crate) fn legacy_revision(name: &str, recipe: &str) -> String {
        todo!("port from ImageMigration.swift")
    }
}
