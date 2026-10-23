// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of RecipeSources.swift – work package F3. Done: `RecipeSource` (type, `id`, `tag`, flags), `RecipeRef`,
//! `Settings::recipe_sources`, the `installed` file format. Skeleton: everything else (`todo!`).
//!
//! Recipe sources (spec §11.3, plan B107): omaflake ships no recipes. It reads them from sources – omaflake/recipes on
//! GitHub by default, other GitHub repositories (also private ones, with a fine-grained token), any HTTPS folder with a
//! catalog.json, or a local folder. Recipes from a remote source are downloaded (every file checked against the
//! catalog's SHA-256) into <data folder>/recipes/<id>/; a local folder is used as it is, so a recipe being written
//! there is built as it is on disk. Before every Nix call the recipes are put together into one folder that the image
//! flake gets as its input `omaflake-recipes`.
//!
//! Swift's `async` functions (fetching, installing) are blocking here (ureq).
#![allow(unused_variables, dead_code)]

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::error::Result;
use crate::file_format::{FileFormat, Step, UnreadableFile};
use crate::models::GuestKind;
use crate::paths::{Paths, Settings};

/// Swift `RecipeSource.Kind`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecipeSourceKind {
    Github,
    Url,
    Folder,
}

/// Where recipes come from. Kept in config.json (`sources`); a token for a private repository lives in the Keychain.
/// JSON keys: kind, location, branch, private, enabled (None fields left out).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RecipeSource {
    pub kind: RecipeSourceKind,
    /// github: "owner/repo"; url: the HTTPS URL of catalog.json; folder: an absolute path
    pub location: String,
    /// github: the branch (None = main)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// github: the repository is private – requests carry the token from the Keychain
    #[serde(rename = "private", default, skip_serializing_if = "Option::is_none")]
    pub is_private: Option<bool>,
    /// None = on
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

impl RecipeSource {
    pub fn new(
        kind: RecipeSourceKind,
        location: &str,
        branch: Option<String>,
        is_private: Option<bool>,
        enabled: Option<bool>,
    ) -> Self {
        RecipeSource { kind, location: location.into(), branch, is_private, enabled }
    }

    /// The default source: omaflake's own recipes
    pub fn official() -> Self {
        Self::new(RecipeSourceKind::Github, "omaflake/recipes", None, None, None)
    }

    /// "github:<owner/repo lowercased>@<branch>", "url:<location>", "folder:<path>"
    pub fn id(&self) -> String {
        match self.kind {
            RecipeSourceKind::Github => {
                format!("github:{}@{}", self.location.to_lowercase(), self.branch.as_deref().unwrap_or("main"))
            }
            RecipeSourceKind::Url => format!("url:{}", self.location),
            RecipeSourceKind::Folder => format!("folder:{}", self.location),
        }
    }

    /// Short, file-name-safe key of the source in recipe IDs (plan B110): the first 8 hex digits of SHA-256 of `id`
    pub fn tag(&self) -> String {
        Self::tag_for_key(&self.id())
    }

    /// Swift `tag(forKey:)`
    pub fn tag_for_key(key: &str) -> String {
        hex::encode(&Sha256::digest(key.as_bytes())[..4])
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }

    pub fn is_private_repository(&self) -> bool {
        self.kind == RecipeSourceKind::Github && self.is_private.unwrap_or(false)
    }

    /// omaflake/recipes on GitHub – its recipes by `omaflake` are marked as official (an author name alone is not enough)
    pub fn is_official(&self) -> bool {
        self.kind == RecipeSourceKind::Github && self.location.to_lowercase() == "omaflake/recipes"
    }

    /// Short name for lists: "owner/repo (branch)", "<host><folder of catalog.json>", "~/…"
    pub fn display_name(&self) -> String {
        todo!("port from RecipeSources.swift")
    }

    /// What the user typed: a GitHub repository ("https://github.com/owner/repo", "github.com/owner/repo/tree/branch",
    /// "owner/repo"), an HTTPS URL of a catalog.json (or of its folder), or a folder on the Mac.
    pub fn parse(input: &str, is_private: bool) -> Result<RecipeSource> {
        todo!("port from RecipeSources.swift")
    }
}

impl Settings {
    /// The sources in their order (earlier ones win when two have the same recipe); none set = omaflake/recipes
    pub fn recipe_sources(&self) -> Vec<RecipeSource> {
        self.sources.clone().unwrap_or_else(|| vec![RecipeSource::official()])
    }
}

// MARK: Tokens

/// Fine-grained GitHub tokens for private sources, in the login Keychain – never in config.json or a file.
/// macOS: `security-framework` (generic password, service `SERVICE`, account = `RecipeSource::id`). Elsewhere:
/// `Error::unsupported` (OPEN: Secret Service on Linux).
pub enum SourceTokens {}

impl SourceTokens {
    pub const SERVICE: &'static str = "app.omaflake.recipe-source";

    pub fn token(source: &RecipeSource) -> Option<String> {
        todo!("port from RecipeSources.swift")
    }

    /// Stores the token (None or "" removes it); the Keychain item's label is "omaflake recipe source <display name>"
    pub fn set(token: Option<&str>, source: &RecipeSource) -> Result<()> {
        todo!("port from RecipeSources.swift")
    }
}

// MARK: Catalog

/// catalog.json's `source`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogSource {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
}

/// One file of a recipe in catalog.json
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

/// One recipe in catalog.json (Swift `RecipeCatalog.Recipe`)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogRecipe {
    pub id: String,
    pub author: String,
    pub kind: GuestKind,
    pub name: String,
    pub path: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub descriptions: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lab: Option<bool>,
    /// What it can be built for (`Platform`, schema 2)
    pub systems: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    pub version: String,
    pub files: Vec<CatalogFile>,
}

impl CatalogRecipe {
    pub fn description_in(&self, language: Option<&str>) -> &str {
        todo!("port from RecipeSources.swift")
    }

    /// It can be built and run on this host
    pub fn runs_here(&self) -> bool {
        todo!("port from RecipeSources.swift")
    }

    /// The recipe's content hash from the catalog's file list – the same as `RecipeLibrary::content_hash` of its
    /// folder, so whether a recipe is here already is known before it is downloaded (plan B110): SHA-256 over the
    /// files sorted by path, each as `path` + NUL + its SHA-256 bytes
    pub fn content_hash(&self) -> String {
        todo!("port from RecipeSources.swift")
    }

    /// Hex → bytes, stopping at the first pair that is not hex
    pub(crate) fn bytes(hex: &str) -> Vec<u8> {
        todo!("port from RecipeSources.swift")
    }
}

/// catalog.json, version 2 (omaflake/recipes, tools/catalog.py). Everything in it comes from the source – untrusted:
/// paths and names are checked before anything is written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecipeCatalog {
    pub source: CatalogSource,
    pub recipes: Vec<CatalogRecipe>,
}

impl RecipeCatalog {
    /// The catalog's version this omaflake knows – `$schema` https://omaflake.dev/schema/catalog/v2.json (2: `systems`
    /// per recipe, plan B114) – and the oldest it reads: version 1 (`"schema": 1`, before B114; its recipes ran on Apple
    /// silicon). A catalog comes from its source and is only read, never rewritten.
    pub const VERSION: usize = 2;
    pub const MINIMUM: usize = 1;

    /// Reads catalog.json: version from `$schema` (type must be "catalog") or the old `"schema": n`; checked with
    /// `FileFormat::check(…, file: "catalog.json")`; version 1 recipes get `systems = [legacy system of their kind]`.
    /// Errors read "Not a recipe catalog (catalog.json): …".
    pub fn decode(data: &[u8]) -> Result<RecipeCatalog> {
        todo!("port from RecipeSources.swift")
    }
}

/// (author, kind, name) of a recipe reference
pub type RefParts = (String, GuestKind, String);

/// "<author>/<kind>/<name>" or "<author>.<kind>.<name>" – both forms a recipe is named by
pub enum RecipeRef {}

impl RecipeRef {
    /// Lowercase letters, digits and "-", not starting with "-"
    pub fn valid_name(s: &str) -> bool {
        match s.chars().next() {
            None | Some('-') => false,
            Some(_) => s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
        }
    }

    /// (author, kind, name) if the reference is well formed
    pub fn parts(r: &str) -> Option<RefParts> {
        let sep = if r.contains('/') { '/' } else { '.' };
        let p: Vec<&str> = r.split(sep).collect();
        if p.len() != 3 || !Self::valid_name(p[0]) || !Self::valid_name(p[2]) {
            return None;
        }
        let kind = GuestKind::parse(p[1])?;
        Some((p[0].to_string(), kind, p[2].to_string()))
    }

    /// "<author>.<kind>.<name>" of a reference
    pub fn id(r: &str) -> Option<String> {
        Self::parts(r).map(|(a, k, n)| format!("{a}.{}.{n}", k.as_str()))
    }

    /// For the CLI and the app: the internal ID of "<author>/<kind>/<name>" (or of an ID), None if not well formed
    pub fn public_id(r: &str) -> Option<String> {
        Self::id(r)
    }
}

// MARK: Fetching

/// Reads from a remote source: public GitHub repositories over raw.githubusercontent.com, private ones over the
/// GitHub API with the token, other sources from their HTTPS folder. Only when asked (opening the Recipes tab,
/// installing) – never in the background. Requests: 30 s timeout, `User-Agent: omaflake`; private repositories add
/// `Authorization: Bearer <token>`, `Accept: application/vnd.github.raw`, `X-GitHub-Api-Version: 2022-11-28`.
#[derive(Debug, Clone)]
pub struct SourceClient {
    pub source: RecipeSource,
    pub(crate) token: Option<String>,
}

impl SourceClient {
    /// `token` None = from the Keychain for a private repository
    pub fn new(source: RecipeSource, token: Option<String>) -> Self {
        todo!("port from RecipeSources.swift")
    }

    /// The URL (or, for a folder, the path) of a file relative to the source's catalog.json
    pub(crate) fn url(&self, relative: &str) -> Result<String> {
        todo!("port from RecipeSources.swift")
    }

    /// One file of the source, relative to its catalog.json (HTTP status texts as in Swift: 401 token not valid,
    /// 403/404 private no access, 404 not found, other "HTTP <status>")
    pub fn fetch(&self, relative: &str) -> Result<Vec<u8>> {
        todo!("port from RecipeSources.swift")
    }

    pub fn catalog(&self) -> Result<RecipeCatalog> {
        todo!("port from RecipeSources.swift")
    }

    /// Checks that the source can be read: a private repository with its token (the repository first, then the
    /// catalog), a public one or a URL by its catalog, a folder by its recipes. Returns a short description.
    pub fn verify(&self) -> Result<String> {
        todo!("port from RecipeSources.swift")
    }
}

// MARK: Library

/// A downloaded recipe in `.installed.json`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Installed {
    /// RecipeSource::id it came from
    pub source: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires: Option<Vec<String>>,
    /// With fractions of a second (versions of a line come in order, plan B111); older files without
    #[serde(with = "crate::json::iso8601_fractional")]
    pub installed: DateTime<Utc>,
}

/// `.installed.json` at its current version: `{"$schema": …, "recipes": {<id>: Installed}}`
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct InstalledFile {
    pub recipes: BTreeMap<String, Installed>,
}

/// .installed.json 0 → 1: the map was the whole file
fn installed_v0_to_v1(o: &mut Map<String, Value>) -> Result<()> {
    let old = std::mem::take(o);
    o.insert("recipes".into(), Value::Object(old));
    Ok(())
}

static INSTALLED_STEPS: [Step; 1] = [installed_v0_to_v1];

/// A recipe omaflake can build, and where it comes from (Swift's `(id, folder, source)` tuple of `available`)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableRecipe {
    /// "<author>.<kind>.<name>@<words>"
    pub id: String,
    pub folder: PathBuf,
    /// `RecipeSource::id`, or `RecipeLibrary::IMPORTED_SOURCE`
    pub source: String,
}

/// One version of a recipe's line (plan B111)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub id: String,
    pub source: String,
    /// When it came (downloaded, imported, kept at a build); None for the recipe a local folder has now
    pub added: Option<DateTime<Utc>>,
    /// The line's newest: a local folder's recipe as it is now, otherwise the last one that came
    pub is_newest: bool,
    /// It is the recipe in a local folder source (changed there, not removed here)
    pub is_live: bool,
    /// Images built from it – the version is kept while there are any
    pub images: Vec<String>,
}

impl Version {
    pub fn line(&self) -> String {
        crate::models::RecipeId::line_key(&self.id, &self.source)
    }
}

/// The recipes omaflake can build: those downloaded from remote sources (<data folder>/recipes) and those in the
/// enabled local folders. `stage` puts them together for the image flake.
#[derive(Debug, Clone)]
pub struct RecipeLibrary {
    pub paths: Paths,
}

impl RecipeLibrary {
    /// Limits for downloaded files (as tools/catalog.py checks them in the source)
    pub const MAX_FILE: u64 = 1024 * 1024;
    pub const MAX_RECIPE: u64 = 5 * 1024 * 1024;

    /// .installed.json, schema `installed` (plan B114). 0 → 1: the map was the whole file
    pub const FILE_FORMAT: FileFormat = FileFormat::new("installed", ".installed.json", 0, &INSTALLED_STEPS);

    /// Source name of recipes that came from a package or a folder (no catalog, no updates)
    pub const IMPORTED_SOURCE: &'static str = "import";

    /// recipe.toml's version this omaflake knows – `"$schema" = "https://omaflake.dev/schema/recipe/v1.json"` – and the
    /// oldest it still builds (version 0: no `$schema`, the systems of Apple silicon). The image flake reads the same
    /// range (plan B114). A recipe is never migrated here: a newer version comes from its author.
    pub const RECIPE_VERSION: usize = 1;
    pub const RECIPE_MINIMUM: usize = 0;

    pub fn new(paths: Paths) -> Self {
        RecipeLibrary { paths }
    }

    pub fn manifest_file(&self) -> PathBuf {
        self.paths.recipes().join(".installed.json")
    }

    /// Downloaded recipes by ID (dates read with or without fractions; migrated files are written back)
    pub fn installed(&self) -> BTreeMap<String, Installed> {
        todo!("port from RecipeSources.swift")
    }

    pub(crate) fn save_manifest(&self, m: &BTreeMap<String, Installed>) -> Result<()> {
        todo!("port from RecipeSources.swift")
    }

    /// Where a downloaded or imported recipe lives: `<data>/recipes/<id>/` (plan B110; one folder per ID, so versions
    /// of one base sit side by side). None for an ID without words or with a malformed base.
    pub fn folder(&self, id: &str) -> Option<PathBuf> {
        todo!("port from RecipeSources.swift")
    }

    /// <author>/<kind>/<name> folders with a recipe.toml under `root` – a source's checkout (with recipes/) or a
    /// folder of recipe folders: (base ID, folder)
    pub fn recipe_folders(root: &Path) -> Vec<(String, PathBuf)> {
        todo!("port from RecipeSources.swift")
    }

    /// Every recipe omaflake can build and where it comes from: the enabled local folders first (in their order),
    /// then the downloaded ones. The first one with an ID wins.
    pub fn available(&self, settings: &Settings) -> Vec<AvailableRecipe> {
        todo!("port from RecipeSources.swift")
    }

    /// The recipe a recipe's `requires` means: the one of that base from the same source, otherwise the first one
    pub(crate) fn requirement<'a>(
        &self,
        base: &str,
        source: &str,
        available: &'a [AvailableRecipe],
    ) -> Option<&'a AvailableRecipe> {
        todo!("port from RecipeSources.swift")
    }

    /// A source's name for lists: the source's own, "imported" recipes by where they are kept
    pub fn source_name(&self, source: &str, settings: &Settings) -> String {
        todo!("port from RecipeSources.swift")
    }

    /// What tells each available recipe apart from others of its base (`RecipeId::distinction`): ID → text
    pub fn distinctions(&self, settings: &Settings) -> HashMap<String, String> {
        todo!("port from RecipeSources.swift")
    }

    // MARK: Install and remove

    /// Downloads a recipe and those it requires (from the same catalog), checking every file's size and SHA-256.
    /// A recipe whose content is here already (any source) is not downloaded again; a new version replaces the older
    /// one of the same source (its line), images built from that stay. Returns the IDs written.
    pub fn install(&self, id: &str, catalog: &RecipeCatalog, client: &SourceClient) -> Result<Vec<String>> {
        todo!("port from RecipeSources.swift")
    }

    /// One recipe into `target` via a `.download-<uuid>` work folder; the catalog entry is checked first (path, sizes,
    /// recipe.toml and default.nix present, file paths inside the folder)
    pub(crate) fn download(&self, r: &CatalogRecipe, client: &SourceClient, target: &Path) -> Result<()> {
        todo!("port from RecipeSources.swift")
    }

    // MARK: Versions (plan B111)

    /// Every line with its versions, newest first (line key → versions)
    pub fn versions(&self, settings: &Settings) -> HashMap<String, Vec<Version>> {
        todo!("port from RecipeSources.swift")
    }

    /// Without `keepRecipeVersions`: the versions of each line that are not its newest and no image was built from are
    /// removed (downloaded, imported and kept copies – a local folder's recipe stays). Returns the removed IDs.
    pub fn prune_versions(&self) -> Vec<String> {
        todo!("port from RecipeSources.swift")
    }

    /// Keeps the version an image is built from (plan B111): a recipe from a local folder – and those it requires – is
    /// copied to `recipes/<id>/`, so it stays when the folder changes. Downloaded and imported ones are kept already.
    pub fn keep(&self, id: &str, settings: &Settings) {
        todo!("port from RecipeSources.swift")
    }

    /// Removes every version of a line that can go (not a local folder's recipe, no image built from it, not required
    /// by another recipe); returns the removed ones and why the others stay
    pub fn remove_line(&self, line: &str) -> (Vec<String>, Vec<String>) {
        todo!("port from RecipeSources.swift")
    }

    /// Removes one version of a recipe (downloaded, imported or kept). Refused while an image is built from it (it is
    /// kept for that image), for a local folder's recipe as it is now, and while another recipe requires it and no other
    /// version of it is left; images built from it stay.
    pub fn remove(&self, id: &str) -> Result<()> {
        todo!("port from RecipeSources.swift")
    }

    // MARK: Packages (plan B108)

    /// The regular files of a recipe folder (relative paths, sorted), within the download limits; symbolic links,
    /// Finder files, a top-level manifest.json and Nix `result` links are left out
    pub fn recipe_files(folder: &Path) -> Result<Vec<String>> {
        todo!("port from RecipeSources.swift")
    }

    /// What a recipe is, as one SHA-256 over its files (sorted relative paths and each file's SHA-256; the same files
    /// `recipe_files` takes) – hex; its words are part of the recipe's ID (`RecipeId`, plan B110)
    pub fn content_hash(folder: &Path) -> Result<String> {
        todo!("port from RecipeSources.swift")
    }

    /// A recipe version this omaflake builds
    pub fn readable(version: usize) -> bool {
        (Self::RECIPE_MINIMUM..=Self::RECIPE_VERSION).contains(&version)
    }

    /// The version a recipe.toml's `"$schema"` names (0 without one or with another schema, written before B114)
    pub fn version_of(folder: &Path) -> usize {
        todo!("port from RecipeSources.swift")
    }

    /// Available recipes of a version this omaflake does not build (too old or too new): left out of what the image
    /// flake sees – one of them would stop every build – and named with the reason
    pub fn unreadable(&self, settings: &Settings) -> Vec<UnreadableFile> {
        todo!("port from RecipeSources.swift")
    }

    /// `requires = ["author/kind/name", …]` of a recipe.toml → recipe IDs
    pub fn requires_of(folder: &Path) -> Vec<String> {
        todo!("port from RecipeSources.swift")
    }

    /// Recipes and the ones they require → `<folder>/<base>.omaflake-recipe` (`recipes/<author>/<kind>/<name>/` +
    /// manifest.json) – what people share
    pub fn export_recipe(&self, id: &str, folder: &Path, settings: &Settings) -> Result<PathBuf> {
        todo!("port from RecipeSources.swift")
    }

    /// The recipes in a package, a checkout, a folder of recipe folders or one recipe folder (whose place says who it
    /// is: …/<author>/<kind>/<name>); an error when there are none. (base ID, folder)
    pub fn recipes_in(&self, url: &Path) -> Result<Vec<(String, PathBuf)>> {
        todo!("port from RecipeSources.swift")
    }

    /// Recipes from a package, a checkout or a folder of recipe folders (`<author>/<kind>/<name>/recipe.toml`), or
    /// one recipe folder → the downloaded recipes (source "import"). Same limits as a download; a recipe it requires
    /// must come along or be available already. A recipe whose content is here already is skipped; other content is a
    /// recipe of its own next to the ones there are (plan B110) – nothing is replaced. Returns the IDs written.
    pub fn import_recipes(&self, from: &Path) -> Result<Vec<String>> {
        todo!("port from RecipeSources.swift")
    }

    // MARK: Staging

    /// Puts every available recipe into one folder for the image flake's input `omaflake-recipes` (plan B110):
    /// `<cache>/recipes/<hash of the content>/<id>/<author>/<kind>/<name>/`, each recipe in a tree of its own with the
    /// recipes it requires next to it (same source first) – recipes import those by relative paths (`../minimal`), so
    /// each sees its own. `recipes.json` tells the flake which folder is which recipe (ID → path). The same content
    /// gives the same folder, so a change in a local folder is picked up by the next Nix call and nothing is copied
    /// twice. Symbolic links and Finder files are left out. Keeps the 8 most recently used staged folders.
    pub fn stage(&self, settings: &Settings) -> Result<PathBuf> {
        todo!("port from RecipeSources.swift")
    }

    /// Keeps the `keep` most recently used staged folders
    pub(crate) fn prune(&self, root: &Path, keep: usize) {
        todo!("port from RecipeSources.swift")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_ids_and_tags() {
        let s = RecipeSource::official();
        assert_eq!(s.id(), "github:omaflake/recipes@main");
        assert_eq!(s.tag().len(), 8);
        assert_eq!(s.tag(), RecipeSource::tag_for_key("github:omaflake/recipes@main"));
        assert!(s.is_official() && s.is_enabled() && !s.is_private_repository());
        let json =
            serde_json::to_string(&RecipeSource::new(RecipeSourceKind::Github, "a/b", None, Some(true), None)).unwrap();
        assert_eq!(json, r#"{"kind":"github","location":"a/b","private":true}"#);
    }

    #[test]
    fn references() {
        assert_eq!(RecipeRef::id("omaflake/nixos/minimal").as_deref(), Some("omaflake.nixos.minimal"));
        assert_eq!(RecipeRef::id("omaflake.macos.minimal").as_deref(), Some("omaflake.macos.minimal"));
        assert!(RecipeRef::id("Omaflake/nixos/minimal").is_none());
        assert!(RecipeRef::id("-a/nixos/b").is_none());
        assert!(RecipeRef::id("a/windows/b").is_none());
        assert!(RecipeRef::id("a/nixos").is_none());
    }

    #[test]
    fn installed_index_is_wrapped() {
        let old =
            br#"{"omaflake.nixos.minimal@abcd1234":{"source":"s","version":"v","installed":"2026-10-01T00:00:00Z"}}"#;
        let (file, migrated) = RecipeLibrary::FILE_FORMAT.decode::<InstalledFile>(old).unwrap();
        assert!(migrated);
        assert_eq!(file.recipes["omaflake.nixos.minimal@abcd1234"].version, "v");
        let text = String::from_utf8(RecipeLibrary::FILE_FORMAT.encode(&file).unwrap()).unwrap();
        assert!(text.contains("\"installed\" : \"2026-10-01T00:00:00.000Z\""));
    }
}
