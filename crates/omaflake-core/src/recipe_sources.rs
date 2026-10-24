// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of RecipeSources.swift – work package F3.
//!
//! Recipe sources (spec §11.3, plan B107): omaflake ships no recipes. It reads them from sources – omaflake/recipes on
//! GitHub by default, other GitHub repositories (also private ones, with a fine-grained token), any HTTPS folder with a
//! catalog.json, or a local folder. Recipes from a remote source are downloaded (every file checked against the
//! catalog's SHA-256) into <data folder>/recipes/<id>/; a local folder is used as it is, so a recipe being written
//! there is built as it is on disk. Before every Nix call the recipes are put together into one folder that the image
//! flake gets as its input `omaflake-recipes`.
//!
//! Swift's `async` functions (fetching, installing) are blocking here (ureq).
//!
//! Not 1:1 (no Foundation): URLs are taken apart by hand (`UrlParts`) instead of `URL(string:)` – enough for what
//! sources hold (https URLs of a catalog.json); an address with white space is refused instead of percent-encoded.
//! Work folders are named with a random UUID-like text like Swift's `UUID().uuidString`. A recipe folder is walked in
//! sorted order (Swift: the file system's), so of several files over the limit the first by name is named.

use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::error::{codes, Error, IoContext, Result};
use crate::file_format::{write_atomic, FileFormat, Step, UnreadableFile};
use crate::image_store::ImageStore;
use crate::models::{GuestKind, RecipeId};
use crate::omaflake_id::OmaflakeId;
use crate::packages::{Manifest, Package, PackageKind};
use crate::paths::{Paths, Settings};
use crate::platform::{abbreviate_tilde, expand_tilde, standardize, Platform};
use crate::trash::Disposal;
use crate::vm_err;

/// Swift `RecipeSource.Kind`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecipeSourceKind {
    Github,
    Url,
    Folder,
}

/// Where recipes come from. Kept in config.json (`sources`); a token for a private repository lives in the Keychain (Linux: the keyring, else a 0600 file – `SourceTokens`).
/// JSON keys: kind, location, branch, private, enabled (None fields left out).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RecipeSource {
    pub kind: RecipeSourceKind,
    /// github: "owner/repo"; url: the HTTPS URL of catalog.json; folder: an absolute path
    pub location: String,
    /// github: the branch (None = main)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// github: the repository is private – requests carry the token from the Keychain/keyring (`SourceTokens`)
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
        match self.kind {
            RecipeSourceKind::Github => match &self.branch {
                Some(b) => format!("{} ({b})", self.location),
                None => self.location.clone(),
            },
            RecipeSourceKind::Url => match UrlParts::parse(&self.location) {
                // Swift: host + deletingLastPathComponent().path
                Some(u) => format!("{}{}", u.host, percent_decode(&parent_path(u.path))),
                None => self.location.clone(),
            },
            RecipeSourceKind::Folder => abbreviate_tilde(&self.location),
        }
    }

    /// What the user typed: a GitHub repository ("https://github.com/owner/repo", "github.com/owner/repo/tree/branch",
    /// "owner/repo"), an HTTPS URL of a catalog.json (or of its folder), or a folder on the Mac.
    pub fn parse(input: &str, is_private: bool) -> Result<RecipeSource> {
        let text = input.trim();
        if text.is_empty() {
            return Err(vm_err!("Enter a GitHub repository, an HTTPS URL or a folder"));
        }
        if text.starts_with('/') || text.starts_with('~') || text.starts_with("file://") {
            let path = match text.strip_prefix("file://") {
                Some(_) => UrlParts::parse(text).map(|u| percent_decode(u.path)).unwrap_or_else(|| text.to_string()),
                None => expand_tilde(text),
            };
            let url = standardize(Path::new(&path));
            if !url.is_dir() {
                return Err(Error::with_code(codes::NOT_FOUND, format!("No such folder: {}", url.display())));
            }
            return Ok(RecipeSource::new(RecipeSourceKind::Folder, &url.to_string_lossy(), None, None, None));
        }
        let private = is_private.then_some(true);
        if let Some((owner, repo, branch)) = Self::github_url(text) {
            // Swift: replacingOccurrences(of: ".git", with: "")
            let repo = repo.replace(".git", "");
            return Ok(RecipeSource::new(
                RecipeSourceKind::Github,
                &format!("{owner}/{repo}"),
                branch.map(str::to_string),
                private,
                None,
            ));
        }
        if let Some((owner, repo)) = text.split_once('/') {
            if Self::github_name(owner) && Self::github_name(repo) {
                return Ok(RecipeSource::new(RecipeSourceKind::Github, text, None, private, None));
            }
        }
        let invalid = || vm_err!("Only HTTPS addresses, GitHub repositories and folders can be recipe sources");
        if text.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(invalid());
        }
        let Some(url) = UrlParts::parse(text) else { return Err(invalid()) };
        if url.scheme.to_lowercase() != "https" || url.host.is_empty() {
            return Err(invalid());
        }
        let last = url.path.split('/').rfind(|s| !s.is_empty()).unwrap_or("");
        if last == "catalog.json" {
            return Ok(RecipeSource::new(RecipeSourceKind::Url, text, None, None, None));
        }
        let path = format!("{}/catalog.json", url.path.trim_end_matches('/'));
        let location = format!("{}://{}{}{}", url.scheme, url.authority, path, url.suffix);
        Ok(RecipeSource::new(RecipeSourceKind::Url, &location, None, None, None))
    }

    /// `[A-Za-z0-9][A-Za-z0-9._-]*` – an owner or repository name as Swift's pattern took it
    fn github_name(s: &str) -> bool {
        let mut chars = s.chars();
        match chars.next() {
            Some(c) if c.is_ascii_alphanumeric() => {}
            _ => return false,
        }
        chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    }

    /// Swift's `^(?:https?://)?(?:www\.)?github\.com/(name)/(name)(?:\.git)?(?:/tree/([^/?#]+))?/?$` →
    /// (owner, repository, branch)
    fn github_url(text: &str) -> Option<(&str, &str, Option<&str>)> {
        let rest = text.strip_prefix("https://").or_else(|| text.strip_prefix("http://")).unwrap_or(text);
        let rest = rest.strip_prefix("www.").unwrap_or(rest);
        let rest = rest.strip_prefix("github.com/")?;
        let rest = rest.strip_suffix('/').unwrap_or(rest);
        let parts: Vec<&str> = rest.split('/').collect();
        let (owner, repo, branch) = match parts.as_slice() {
            [o, r] => (*o, *r, None),
            [o, r, "tree", b] => (*o, *r, Some(*b)),
            _ => return None,
        };
        if !Self::github_name(owner) || !Self::github_name(repo) {
            return None;
        }
        if let Some(b) = branch {
            if b.is_empty() || b.contains(['?', '#']) {
                return None;
            }
        }
        Some((owner, repo, branch))
    }
}

impl Settings {
    /// The sources in their order (earlier ones win when two have the same recipe); none set = omaflake/recipes
    pub fn recipe_sources(&self) -> Vec<RecipeSource> {
        self.sources.clone().unwrap_or_else(|| vec![RecipeSource::official()])
    }
}

/// An absolute URL taken apart: `<scheme>://<authority><path><suffix>` (suffix = `?query#fragment`)
struct UrlParts<'a> {
    scheme: &'a str,
    authority: &'a str,
    host: &'a str,
    path: &'a str,
    suffix: &'a str,
}

impl<'a> UrlParts<'a> {
    fn parse(s: &'a str) -> Option<UrlParts<'a>> {
        let (scheme, rest) = s.split_once("://")?;
        let mut sc = scheme.chars();
        if !sc.next().is_some_and(|c| c.is_ascii_alphabetic())
            || !sc.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        {
            return None;
        }
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let (authority, after) = rest.split_at(end);
        let pend = after.find(['?', '#']).unwrap_or(after.len());
        let (path, suffix) = after.split_at(pend);
        let hostport = authority.rsplit_once('@').map(|x| x.1).unwrap_or(authority);
        let host = if hostport.starts_with('[') {
            hostport.split_inclusive(']').next().unwrap_or(hostport)
        } else {
            hostport.split(':').next().unwrap_or("")
        };
        Some(UrlParts { scheme, authority, host, path, suffix })
    }
}

/// Swift `deletingLastPathComponent().path`: "/a/b/catalog.json" → "/a/b", "/catalog.json" → "/"
fn parent_path(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    let parent = match trimmed.rfind('/') {
        Some(i) => &trimmed[..i],
        None => "",
    };
    if parent.is_empty() {
        "/".into()
    } else {
        parent.to_string()
    }
}

/// "%20" → " " (invalid sequences stay as they are)
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let digit = |c: u8| (c as char).to_digit(16);
            if let (Some(h), Some(l)) = (digit(b[i + 1]), digit(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

/// Swift `addingPercentEncoding(withAllowedCharacters: .urlPathAllowed)` for one path segment
fn percent_encode_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        let c = b as char;
        if c.is_ascii_alphanumeric() || "!$&'()*+,-.:;=@_~".contains(c) {
            out.push(c);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A random UUID-like text (Swift `UUID().uuidString`) for work folder names
fn uuid() -> String {
    let mut b = [0u8; 16];
    let filled = fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b));
    if filled.is_err() {
        let seed = format!("{:?}{}", std::time::SystemTime::now(), std::process::id());
        b.copy_from_slice(&Sha256::digest(seed.as_bytes())[..16]);
    }
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = hex::encode_upper(b);
    format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
}

/// SHA-256 as lowercase hex
fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

/// Swift's `.whitespaces` (spaces and tabs, no line breaks)
fn is_blank(c: char) -> bool {
    c == '\t'
        || (c.is_whitespace() && !matches!(c, '\n' | '\r' | '\u{0B}' | '\u{0C}' | '\u{85}' | '\u{2028}' | '\u{2029}'))
}

// MARK: Tokens

/// Tokens of private sources: the Keychain on macOS, the Secret Service or a 0600 file on Linux (source_tokens.rs)
pub use crate::source_tokens::SourceTokens;

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
        language.and_then(|l| self.descriptions.as_ref()?.get(l)).map(String::as_str).unwrap_or(&self.description)
    }

    /// It can be built and run on this host
    pub fn runs_here(&self) -> bool {
        Platform::host_system(self.kind).map(|s| self.systems.iter().any(|x| x == s)).unwrap_or(false)
    }

    /// The recipe's content hash from the catalog's file list – the same as `RecipeLibrary::content_hash` of its
    /// folder, so whether a recipe is here already is known before it is downloaded (plan B110): SHA-256 over the
    /// files sorted by path, each as `path` + NUL + its SHA-256 bytes
    pub fn content_hash(&self) -> String {
        let mut files: Vec<&CatalogFile> = self.files.iter().collect();
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let mut hasher = Sha256::new();
        for f in files {
            hasher.update(f.path.as_bytes());
            hasher.update([0u8]);
            hasher.update(Self::bytes(&f.sha256.to_lowercase()));
        }
        hex::encode(hasher.finalize())
    }

    /// Hex → bytes, pair by pair (Swift `bytes(hex:)`): a pair that is not hex is skipped, a single last digit ignored
    pub(crate) fn bytes(hex: &str) -> Vec<u8> {
        let chars: Vec<char> = hex.chars().collect();
        let mut out = Vec::with_capacity(chars.len() / 2);
        for pair in chars.as_chunks::<2>().0 {
            let s: String = pair.iter().collect();
            // Swift's UInt8(_:radix:) also takes a sign: "+f" is 15, "-0" is 0
            let value = if s == "-0" { Some(0) } else { u8::from_str_radix(&s, 16).ok() };
            if let Some(b) = value {
                out.push(b);
            }
        }
        out
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
        let Ok(Value::Object(mut object)) = serde_json::from_slice::<Value>(data) else {
            return Err(vm_err!("Not a recipe catalog (catalog.json): not JSON"));
        };
        let found = match object.get("$schema").and_then(Value::as_str) {
            Some(schema) => match FileFormat::parse(schema) {
                Some((t, v)) if t == "catalog" => v,
                _ => return Err(vm_err!("Not a recipe catalog (catalog.json): {schema}")),
            },
            // before B114
            None => object.get("schema").and_then(Value::as_u64).unwrap_or(0) as usize,
        };
        FileFormat::check(found, Self::MINIMUM, Self::VERSION, "catalog.json")?;
        if found < 2 {
            if let Some(Value::Array(recipes)) = object.get_mut("recipes") {
                // Version 1 has no systems: its recipes were made for Apple silicon
                if recipes.iter().all(Value::is_object) {
                    for r in recipes.iter_mut() {
                        let kind = r.get("kind").and_then(Value::as_str).and_then(GuestKind::parse);
                        let system = Platform::legacy_system(kind.unwrap_or(GuestKind::Nixos));
                        r["systems"] = Value::Array(vec![Value::String(system.into())]);
                    }
                }
            }
        }
        serde_json::from_value::<RecipeCatalog>(Value::Object(object))
            .map_err(|e| vm_err!("Not a recipe catalog (catalog.json): {e}"))
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
    /// Request timeout (Swift `timeoutInterval: 30`)
    const TIMEOUT: Duration = Duration::from_secs(30);
    /// The most a response may have (a catalog; recipe files are at most `RecipeLibrary::MAX_FILE`)
    const MAX_RESPONSE: u64 = 64 * 1024 * 1024;

    /// `token` None = the stored one (`SourceTokens`) for a private repository
    pub fn new(source: RecipeSource, token: Option<String>) -> Self {
        let token = token.or_else(|| if source.is_private_repository() { SourceTokens::token(&source) } else { None });
        SourceClient { source, token }
    }

    /// The URL (or, for a folder, the path) of a file relative to the source's catalog.json
    pub(crate) fn url(&self, relative: &str) -> Result<String> {
        let branch = self.source.branch.as_deref().unwrap_or("main");
        let encoded =
            relative.split('/').filter(|s| !s.is_empty()).map(percent_encode_segment).collect::<Vec<_>>().join("/");
        let location = &self.source.location;
        match self.source.kind {
            RecipeSourceKind::Github if self.source.is_private_repository() => {
                Ok(format!("https://api.github.com/repos/{location}/contents/{encoded}?ref={branch}"))
            }
            RecipeSourceKind::Github => Ok(format!("https://raw.githubusercontent.com/{location}/{branch}/{encoded}")),
            RecipeSourceKind::Url => {
                let Some(u) = UrlParts::parse(location) else {
                    return Err(vm_err!("Not a valid address: {location}"));
                };
                let folder = match u.path.trim_end_matches('/').rfind('/') {
                    Some(i) => &u.path[..=i],
                    None => "/",
                };
                Ok(format!("{}://{}{folder}{encoded}{}", u.scheme, u.authority, u.suffix))
            }
            RecipeSourceKind::Folder => Ok(Path::new(location).join(relative).to_string_lossy().into_owned()),
        }
    }

    /// GET with the source's headers (`accept` overrides the private repository's); (status, body)
    fn get(&self, url: &str, accept: Option<&str>) -> Result<(u16, Vec<u8>)> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Self::TIMEOUT))
            .http_status_as_error(false)
            .build()
            .into();
        let mut request = agent.get(url);
        if self.source.is_private_repository() {
            if let Some(token) = &self.token {
                request = request.header("Authorization", &format!("Bearer {token}"));
            }
            request = request.header("Accept", accept.unwrap_or("application/vnd.github.raw"));
            request = request.header("X-GitHub-Api-Version", "2022-11-28");
        } else if let Some(a) = accept {
            request = request.header("Accept", a);
        }
        request = request.header("User-Agent", "omaflake");
        let network = |e: ureq::Error| Error::with_code(codes::NETWORK, format!("{}: {e}", self.source.display_name()));
        let mut response = request.call().map_err(network)?;
        let status = response.status().as_u16();
        let body = response.body_mut().with_config().limit(Self::MAX_RESPONSE).read_to_vec().map_err(network)?;
        Ok((status, body))
    }

    /// One file of the source, relative to its catalog.json (HTTP status texts as in Swift: 401 token not valid,
    /// 403/404 private no access, 404 not found, other "HTTP <status>")
    pub fn fetch(&self, relative: &str) -> Result<Vec<u8>> {
        let url = self.url(relative)?;
        let name = self.source.display_name();
        if self.source.kind == RecipeSourceKind::Folder {
            return fs::read(&url).map_err(|e| {
                let code = if e.kind() == std::io::ErrorKind::NotFound { codes::NOT_FOUND } else { codes::NETWORK };
                Error::with_code(code, format!("{url}: {}", crate::error::os_text(&e)))
            });
        }
        if self.source.is_private_repository() && self.token.is_none() {
            return Err(Error::with_code(
                codes::AUTH,
                format!("{name} is private – add a token in Settings → Recipe Sources"),
            ));
        }
        let (status, data) = self.get(&url, None)?;
        // Swift `case 403, 404 where private`: the `where` belongs to 404 only – a 403 is always "no access"
        match status {
            200 => Ok(data),
            401 => Err(Error::with_code(codes::AUTH, format!("{name}: the token is not valid (or has expired)"))),
            403 => Err(self.no_access(relative)),
            404 if self.source.is_private_repository() => Err(self.no_access(relative)),
            404 => Err(Error::with_code(
                codes::NOT_FOUND,
                format!("{name}: {relative} not found (is it a recipe source, and public?)"),
            )),
            _ => Err(Error::with_code(codes::NETWORK, format!("{name}: {relative} – HTTP {status}"))),
        }
    }

    fn no_access(&self, relative: &str) -> Error {
        Error::with_code(
            codes::AUTH,
            format!(
                "{}: no access to {relative} – the token needs read access to the repository's contents (Contents: \
                 Read-only), and the branch must exist",
                self.source.display_name()
            ),
        )
    }

    pub fn catalog(&self) -> Result<RecipeCatalog> {
        RecipeCatalog::decode(&self.fetch("catalog.json")?)
    }

    /// Checks that the source can be read: a private repository with its token (the repository first, then the
    /// catalog), a public one or a URL by its catalog, a folder by its recipes. Returns a short description.
    pub fn verify(&self) -> Result<String> {
        match self.source.kind {
            RecipeSourceKind::Folder => {
                let n = RecipeLibrary::recipe_folders(Path::new(&self.source.location)).len();
                if n == 0 {
                    return Err(Error::with_code(
                        codes::NOT_FOUND,
                        "No recipes in this folder – it needs recipes/<author>/<kind>/<name>/recipe.toml",
                    ));
                }
                Ok(format!("{n} recipes"))
            }
            RecipeSourceKind::Github if self.source.is_private_repository() => {
                if self.token.as_deref().is_none_or(str::is_empty) {
                    return Err(Error::with_code(
                        codes::AUTH,
                        "Enter a fine-grained token with read access to the repository",
                    ));
                }
                let url = format!("https://api.github.com/repos/{}", self.source.location);
                let (status, data) = self.get(&url, Some("application/vnd.github+json"))?;
                match status {
                    200 => {}
                    401 => return Err(Error::with_code(codes::AUTH, "The token is not valid (or has expired)")),
                    _ => {
                        return Err(Error::with_code(
                            codes::AUTH,
                            format!(
                                "The token has no access to {} – give it this repository with Contents: Read-only",
                                self.source.location
                            ),
                        ))
                    }
                }
                let repo = serde_json::from_slice::<Value>(&data).ok();
                let c = self.catalog()?;
                let public = repo.as_ref().and_then(|r| r.get("private")).and_then(Value::as_bool) == Some(false);
                let visibility = if public { "public" } else { "private" };
                Ok(format!("{}: {} recipes ({visibility} repository, token works)", c.source.name, c.recipes.len()))
            }
            _ => {
                let c = self.catalog()?;
                Ok(format!("{}: {} recipes", c.source.name, c.recipes.len()))
            }
        }
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

    /// How many staged folders `stage` keeps
    const STAGED_KEPT: usize = 8;

    pub fn new(paths: Paths) -> Self {
        RecipeLibrary { paths }
    }

    pub fn manifest_file(&self) -> PathBuf {
        self.paths.recipes().join(".installed.json")
    }

    /// Downloaded recipes by ID (dates read with or without fractions; migrated files are written back)
    pub fn installed(&self) -> BTreeMap<String, Installed> {
        let Ok(data) = fs::read(self.manifest_file()) else { return BTreeMap::new() };
        let Ok((file, migrated)) = Self::FILE_FORMAT.decode::<InstalledFile>(&data) else { return BTreeMap::new() };
        if migrated {
            let _ = self.save_manifest(&file.recipes);
        }
        file.recipes
    }

    pub(crate) fn save_manifest(&self, m: &BTreeMap<String, Installed>) -> Result<()> {
        let dir = self.paths.recipes();
        fs::create_dir_all(&dir).with_path(&dir)?;
        let file = self.manifest_file();
        Self::FILE_FORMAT.check_writable(&file)?;
        let data = Self::FILE_FORMAT.encode(&InstalledFile { recipes: m.clone() })?;
        write_atomic(&file, &data)
    }

    /// Where a downloaded or imported recipe lives: `<data>/recipes/<id>/` (plan B110; one folder per ID, so versions
    /// of one base sit side by side). None for an ID without words or with a malformed base.
    pub fn folder(&self, id: &str) -> Option<PathBuf> {
        if RecipeRef::parts(RecipeId::base(id)).is_none() || RecipeId::words(id).is_none() {
            return None;
        }
        Some(self.paths.recipes().join(id))
    }

    /// <author>/<kind>/<name> folders with a recipe.toml under `root` – a source's checkout (with recipes/) or a
    /// folder of recipe folders: (base ID, folder)
    pub fn recipe_folders(root: &Path) -> Vec<(String, PathBuf)> {
        let base = if root.join("recipes").exists() { root.join("recipes") } else { root.to_path_buf() };
        // Folders (links to folders too), not hidden, sorted
        fn dirs(u: &Path) -> Vec<String> {
            let Ok(entries) = fs::read_dir(u) else { return Vec::new() };
            let mut names: Vec<String> = entries
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| !n.starts_with('.') && u.join(n).is_dir())
                .collect();
            names.sort();
            names
        }
        let mut out = Vec::new();
        for author in dirs(&base).into_iter().filter(|a| RecipeRef::valid_name(a)) {
            for kind in GuestKind::ALL {
                let kind_dir = base.join(&author).join(kind.as_str());
                for name in dirs(&kind_dir).into_iter().filter(|n| RecipeRef::valid_name(n)) {
                    let folder = kind_dir.join(&name);
                    if folder.join("recipe.toml").exists() {
                        out.push((format!("{author}.{}.{name}", kind.as_str()), folder));
                    }
                }
            }
        }
        out
    }

    /// Every recipe omaflake can build and where it comes from: the enabled local folders first (in their order),
    /// then the downloaded ones. The first one with an ID wins.
    pub fn available(&self, settings: &Settings) -> Vec<AvailableRecipe> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        // The ID carries the content hash (12 words, unique worldwide): the same base with the same content is one
        // recipe – the first source's
        let mut add = |base: &str, folder: PathBuf, source: String| {
            let Ok(hash) = Self::content_hash(&folder) else { return };
            let id = RecipeId::make_with_hash(base, &hash, OmaflakeId::LENGTH);
            if seen.insert(id.clone()) {
                out.push(AvailableRecipe { id, folder, source });
            }
        };
        for s in settings.recipe_sources() {
            if s.is_enabled() && s.kind == RecipeSourceKind::Folder {
                for (base, folder) in Self::recipe_folders(Path::new(&s.location)) {
                    add(&base, folder, s.id());
                }
            }
        }
        for (id, entry) in self.installed() {
            let Some(folder) = self.folder(&id) else { continue };
            if !folder.exists() {
                continue;
            }
            add(RecipeId::base(&id), folder, entry.source);
        }
        out
    }

    /// The recipe a recipe's `requires` means: the one of that base from the same source, otherwise the first one
    pub(crate) fn requirement<'a>(
        &self,
        base: &str,
        source: &str,
        available: &'a [AvailableRecipe],
    ) -> Option<&'a AvailableRecipe> {
        available
            .iter()
            .find(|a| RecipeId::base(&a.id) == base && a.source == source)
            .or_else(|| available.iter().find(|a| RecipeId::base(&a.id) == base))
    }

    /// A source's name for lists: the source's own, "imported" recipes by where they are kept
    pub fn source_name(&self, source: &str, settings: &Settings) -> String {
        if source == Self::IMPORTED_SOURCE {
            return abbreviate_tilde(&self.paths.recipes().to_string_lossy());
        }
        settings
            .recipe_sources()
            .iter()
            .find(|s| s.id() == source)
            .map(RecipeSource::display_name)
            .unwrap_or_else(|| source.to_string())
    }

    /// What tells each available recipe apart from others of its base (`RecipeId::distinction`): ID → text
    pub fn distinctions(&self, settings: &Settings) -> HashMap<String, String> {
        let mut entries: HashMap<String, String> = HashMap::new();
        for a in self.available(settings) {
            entries.entry(a.id).or_insert(a.source);
        }
        let name = |s: &str| self.source_name(s, settings);
        let mut out = HashMap::new();
        for id in entries.keys() {
            if let Some(d) = RecipeId::distinction(id, &entries, &name) {
                out.insert(id.clone(), d);
            }
        }
        out
    }

    // MARK: Install and remove

    /// Downloads a recipe and those it requires (from the same catalog), checking every file's size and SHA-256.
    /// A recipe whose content is here already (any source) is not downloaded again; a new version replaces the older
    /// one of the same source (its line), images built from that stay. Returns the IDs written.
    pub fn install(&self, id: &str, catalog: &RecipeCatalog, client: &SourceClient) -> Result<Vec<String>> {
        fn collect<'a>(
            r: &str,
            catalog: &'a RecipeCatalog,
            client: &SourceClient,
            visiting: &mut HashSet<String>,
            order: &mut Vec<&'a CatalogRecipe>,
        ) -> Result<()> {
            let Some(rid) = RecipeRef::id(RecipeId::base(r)) else {
                return Err(Error::with_code(codes::NOT_FOUND, format!("Not a recipe: {r}")));
            };
            if !visiting.insert(rid.clone()) {
                return Ok(());
            }
            let Some(found) = catalog.recipes.iter().find(|c| c.id == rid) else {
                return Err(Error::with_code(
                    codes::NOT_FOUND,
                    format!("{r} is not in {}", client.source.display_name()),
                ));
            };
            for req in found.requires.iter().flatten() {
                collect(req, catalog, client, visiting, order)?;
            }
            order.push(found);
            Ok(())
        }
        let mut order = Vec::new();
        collect(id, catalog, client, &mut HashSet::new(), &mut order)?;
        let mut manifest = self.installed();
        let mut written = Vec::new();
        let here: HashSet<String> = self.available(&Settings::load(&self.paths)).into_iter().map(|a| a.id).collect();
        for r in order {
            let hash = r.content_hash();
            let full = RecipeId::make_with_hash(&r.id, &hash, OmaflakeId::LENGTH);
            if here.contains(&full) || manifest.contains_key(&full) {
                continue;
            }
            let Some(target) = self.folder(&full) else { continue };
            self.download(r, client, &target)?;
            if Self::content_hash(&target).ok().as_deref() != Some(hash.as_str()) {
                let _ = Disposal::delete(&[&target]);
                return Err(vm_err!("{} does not match the catalog – nothing was installed", r.id));
            }
            let requires = r.requires.as_ref().map(|list| list.iter().filter_map(|x| RecipeRef::id(x)).collect());
            manifest.insert(
                full.clone(),
                Installed {
                    source: client.source.id(),
                    version: r.version.clone(),
                    requires,
                    installed: Self::install_time(&manifest),
                },
            );
            self.save_manifest(&manifest)?;
            written.push(full);
        }
        if !written.is_empty() {
            self.prune_versions(); // an older version of this line goes, unless kept (plan B111)
        }
        Ok(written)
    }

    /// One recipe into `target` via a `.download-<uuid>` work folder; the catalog entry is checked first (path, sizes,
    /// recipe.toml and default.nix present, file paths inside the folder)
    pub(crate) fn download(&self, r: &CatalogRecipe, client: &SourceClient, target: &Path) -> Result<()> {
        // The catalog is untrusted: its path must be the recipe's own folder, file paths stay inside it
        let well_formed = match RecipeRef::parts(&r.id) {
            Some((author, kind, name)) => {
                author == r.author
                    && kind == r.kind
                    && name == r.name
                    && r.path == format!("recipes/{author}/{}/{name}", kind.as_str())
            }
            None => false,
        };
        if !well_formed {
            return Err(vm_err!("The catalog's entry for {} is not well formed", r.id));
        }
        if r.files.iter().map(|f| f.size).sum::<u64>() > Self::MAX_RECIPE {
            return Err(vm_err!("{} is larger than 5 MiB", r.id));
        }
        if !r.files.iter().any(|f| f.path == "recipe.toml") || !r.files.iter().any(|f| f.path == "default.nix") {
            return Err(vm_err!("{} has no recipe.toml or default.nix", r.id));
        }
        let recipes = self.paths.recipes();
        fs::create_dir_all(&recipes).with_path(&recipes)?;
        let work = recipes.join(format!(".download-{}", uuid()));
        let result = (|| -> Result<()> {
            for f in &r.files {
                let parts: Vec<&str> = f.path.split('/').collect();
                if f.path.starts_with('/') || parts.iter().any(|p| p.is_empty() || *p == "." || *p == "..") {
                    return Err(vm_err!("{}: a file path leaves the recipe's folder ({})", r.id, f.path));
                }
                if f.size > Self::MAX_FILE {
                    return Err(vm_err!("{}/{} is larger than 1 MiB", r.id, f.path));
                }
                let data = client.fetch(&format!("{}/{}", r.path, f.path))?;
                if data.len() as u64 != f.size || sha256_hex(&data) != f.sha256.to_lowercase() {
                    return Err(vm_err!(
                        "{}/{} does not match the catalog (size or SHA-256) – nothing was installed",
                        r.id,
                        f.path
                    ));
                }
                let dest = work.join(&f.path);
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent).with_path(parent)?;
                }
                fs::write(&dest, &data).with_path(&dest)?;
            }
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).with_path(parent)?;
            }
            Disposal::delete(&[target])?;
            fs::rename(&work, target).with_path(target)?;
            Ok(())
        })();
        let _ = Disposal::delete(&[&work]);
        result
    }

    /// When a recipe version is added: now, but always after every version already here – the newest version of a
    /// line is the last that came, and two imports within one millisecond must not tie (seen on a fast Linux host)
    fn install_time(manifest: &BTreeMap<String, Installed>) -> DateTime<Utc> {
        // Whole milliseconds: `.installed.json` keeps no more, so a later time within the same millisecond would tie
        let now = DateTime::from_timestamp_millis(Utc::now().timestamp_millis()).unwrap_or_else(Utc::now);
        match manifest.values().map(|i| i.installed).max() {
            Some(last) if last >= now => last + chrono::Duration::milliseconds(1),
            _ => now,
        }
    }

    // MARK: Versions (plan B111)

    /// Every line with its versions, newest first (line key → versions)
    pub fn versions(&self, settings: &Settings) -> HashMap<String, Vec<Version>> {
        let installed = self.installed();
        let mut images: HashMap<String, Vec<String>> = HashMap::new();
        for image in ImageStore::new(self.paths.clone()).list() {
            images.entry(image.recipe().to_string()).or_default().push(image.name.clone());
        }
        for names in images.values_mut() {
            names.sort();
        }
        let live_folders: Vec<String> = settings
            .recipe_sources()
            .into_iter()
            .filter(|s| s.kind == RecipeSourceKind::Folder)
            .map(|s| s.location)
            .collect();
        let mut out: HashMap<String, Vec<Version>> = HashMap::new();
        for entry in self.available(settings) {
            // available() lists a local folder's recipe before a kept copy of the same content
            let path = entry.folder.to_string_lossy();
            let live = live_folders.iter().any(|f| path.starts_with(&format!("{f}/")));
            let v = Version {
                added: installed.get(&entry.id).map(|i| i.installed),
                is_newest: false,
                is_live: live,
                images: images.get(&entry.id).cloned().unwrap_or_default(),
                id: entry.id,
                source: entry.source,
            };
            out.entry(v.line()).or_default().push(v);
        }
        for list in out.values_mut() {
            // Live first, then the last that came first (None = the folder's recipe now, newest)
            list.sort_by_key(|v| (Reverse(v.is_live), Reverse(v.added.map_or((1, None), |d| (0, Some(d))))));
            for (i, v) in list.iter_mut().enumerate() {
                v.is_newest = i == 0;
            }
        }
        out
    }

    /// Every version's line and place in it (0 = newest) – what a name without words resolves to
    /// (`RecipeId::resolve_newest`, plan B111)
    pub fn places(&self, settings: &Settings) -> HashMap<String, crate::models::LinePlace> {
        let mut out: HashMap<String, crate::models::LinePlace> = HashMap::new();
        for (line, list) in self.versions(settings) {
            for (rank, v) in list.into_iter().enumerate() {
                // the same content in a folder and as a kept copy: its best place
                let place = out.entry(v.id).or_insert(crate::models::LinePlace { line: line.clone(), rank });
                place.rank = place.rank.min(rank);
            }
        }
        out
    }

    /// Without `keepRecipeVersions`: the versions of each line that are not its newest and no image was built from are
    /// removed (downloaded, imported and kept copies – a local folder's recipe stays). Returns the removed IDs.
    pub fn prune_versions(&self) -> Vec<String> {
        let settings = Settings::load(&self.paths);
        if settings.keep_recipe_versions.unwrap_or(false) {
            return Vec::new();
        }
        let mut removed = Vec::new();
        let versions = self.versions(&settings);
        let mut lines: Vec<&String> = versions.keys().collect();
        lines.sort();
        for line in lines {
            for v in &versions[line] {
                if !v.is_newest && !v.is_live && v.images.is_empty() && self.remove(&v.id).is_ok() {
                    removed.push(v.id.clone());
                }
            }
        }
        removed
    }

    /// Keeps the version an image is built from (plan B111): a recipe from a local folder – and those it requires – is
    /// copied to `recipes/<id>/`, so it stays when the folder changes. Downloaded and imported ones are kept already.
    pub fn keep(&self, id: &str, settings: &Settings) {
        let available = self.available(settings);
        let before = self.installed();
        let mut manifest = before.clone();
        let Some(entry) = available.iter().find(|a| a.id == id) else { return };
        self.keep_copy(entry, &available, &mut manifest);
        if manifest != before {
            let _ = self.save_manifest(&manifest);
        }
    }

    /// `keep`'s copy of one recipe and, recursively, those it requires
    fn keep_copy(
        &self,
        entry: &AvailableRecipe,
        available: &[AvailableRecipe],
        manifest: &mut BTreeMap<String, Installed>,
    ) {
        if manifest.contains_key(&entry.id) {
            return;
        }
        let Some(target) = self.folder(&entry.id) else { return };
        if target.exists() {
            return;
        }
        let work = self.paths.recipes().join(format!(".keep-{}", uuid()));
        let copied = (|| -> Result<()> {
            for rel in Self::recipe_files(&entry.folder)? {
                let file = work.join(&rel);
                if let Some(parent) = file.parent() {
                    fs::create_dir_all(parent).with_path(parent)?;
                }
                fs::copy(entry.folder.join(&rel), &file).with_path(&file)?;
            }
            let recipes = self.paths.recipes();
            fs::create_dir_all(&recipes).with_path(&recipes)?;
            fs::rename(&work, &target).with_path(&target)?;
            Ok(())
        })();
        let _ = Disposal::delete(&[&work]);
        if copied.is_err() {
            return;
        }
        manifest.insert(
            entry.id.clone(),
            Installed {
                source: entry.source.clone(),
                version: RecipeId::words(&entry.id).unwrap_or("").to_string(),
                requires: Some(Self::requires_of(&target)),
                installed: Self::install_time(manifest),
            },
        );
        for req in Self::requires_of(&entry.folder) {
            if let Some(r) = self.requirement(&req, &entry.source, available) {
                self.keep_copy(r, available, manifest);
            }
        }
    }

    /// Removes every version of a line that can go (not a local folder's recipe, no image built from it, not required
    /// by another recipe); returns the removed ones and why the others stay
    pub fn remove_line(&self, line: &str) -> (Vec<String>, Vec<String>) {
        let mut removed = Vec::new();
        let mut kept = Vec::new();
        let versions = self.versions(&Settings::load(&self.paths));
        for v in versions.get(line).into_iter().flatten() {
            match self.remove(&v.id) {
                Ok(()) => removed.push(v.id.clone()),
                Err(e) => kept.push(format!("{}: {e}", OmaflakeId::recipe(&v.id, 2))),
            }
        }
        (removed, kept)
    }

    /// Removes one version of a recipe (downloaded, imported or kept). Refused while an image is built from it (it is
    /// kept for that image), for a local folder's recipe as it is now, and while another recipe requires it and no other
    /// version of it is left; images built from it stay.
    pub fn remove(&self, id: &str) -> Result<()> {
        let mut manifest = self.installed();
        let Some(source) = manifest.get(id).map(|i| i.source.clone()) else {
            return Err(vm_err!("{} lives in a local folder – change or remove it there", OmaflakeId::recipe(id, 2)));
        };
        let used = ImageStore::new(self.paths.clone()).list().iter().any(|i| i.recipe() == id);
        if used {
            return Err(Error::with_code(
                codes::IN_USE,
                format!("Images are built from {} – it is kept while they are there", OmaflakeId::recipe(id, 2)),
            ));
        }
        // Required by another recipe of the same source, and no other recipe of that base left for it
        let base = RecipeId::base(id);
        let others_left =
            self.available(&Settings::load(&self.paths)).iter().any(|a| a.id != id && RecipeId::base(&a.id) == base);
        let mut users: Vec<&String> = if others_left {
            Vec::new()
        } else {
            manifest
                .iter()
                .filter(|(k, v)| {
                    k.as_str() != id
                        && v.source == source
                        && v.requires.as_ref().is_some_and(|r| r.iter().any(|x| x == base))
                })
                .map(|(k, _)| k)
                .collect()
        };
        users.sort();
        if !users.is_empty() {
            let names: Vec<String> = users.iter().map(|u| RecipeId::qualified(u)).collect();
            return Err(Error::with_code(
                codes::IN_USE,
                format!("{} is required by {} – remove those first", RecipeId::qualified(id), names.join(", ")),
            ));
        }
        if let Some(folder) = self.folder(id) {
            Disposal::delete(&[folder])?;
        }
        manifest.remove(id);
        self.save_manifest(&manifest)
    }

    // MARK: Packages (plan B108)

    /// The regular files of a recipe folder (relative paths, sorted), within the download limits; symbolic links,
    /// Finder files, a top-level manifest.json and Nix `result` links are left out
    pub fn recipe_files(folder: &Path) -> Result<Vec<String>> {
        fn walk(folder: &Path, rel: &str, files: &mut Vec<String>, total: &mut u64) -> Result<()> {
            let dir = if rel.is_empty() { folder.to_path_buf() } else { folder.join(rel) };
            let Ok(entries) = fs::read_dir(&dir) else { return Ok(()) };
            let mut names: Vec<String> =
                entries.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
            names.sort();
            for name in names {
                let rel = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
                let path = folder.join(&rel);
                let meta = fs::symlink_metadata(&path).with_path(&path)?;
                if meta.is_dir() {
                    // Swift's enumerator goes into every folder (a `result` folder's files are left out by their path)
                    walk(folder, &rel, files, total)?;
                    continue;
                }
                if name == ".DS_Store" || (name == "manifest.json" && !rel.contains('/')) || rel.starts_with("result") {
                    continue;
                }
                if !meta.is_file() {
                    continue;
                }
                if meta.len() > RecipeLibrary::MAX_FILE {
                    return Err(vm_err!("{rel} is larger than 1 MiB"));
                }
                *total += meta.len();
                files.push(rel);
            }
            Ok(())
        }
        let mut files = Vec::new();
        let mut total = 0;
        walk(folder, "", &mut files, &mut total)?;
        if total > Self::MAX_RECIPE {
            let name = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            return Err(vm_err!("{name} is larger than 5 MiB"));
        }
        files.sort();
        Ok(files)
    }

    /// What a recipe is, as one SHA-256 over its files (sorted relative paths and each file's SHA-256; the same files
    /// `recipe_files` takes) – hex; its words are part of the recipe's ID (`RecipeId`, plan B110)
    pub fn content_hash(folder: &Path) -> Result<String> {
        let mut hasher = Sha256::new();
        for rel in Self::recipe_files(folder)? {
            let path = folder.join(&rel);
            let data = fs::read(&path).with_path(&path)?;
            hasher.update(rel.as_bytes());
            hasher.update([0u8]);
            hasher.update(Sha256::digest(&data));
        }
        Ok(hex::encode(hasher.finalize()))
    }

    /// A recipe version this omaflake builds
    pub fn readable(version: usize) -> bool {
        (Self::RECIPE_MINIMUM..=Self::RECIPE_VERSION).contains(&version)
    }

    /// The version a recipe.toml's `"$schema"` names (0 without one or with another schema, written before B114)
    pub fn version_of(folder: &Path) -> usize {
        let Ok(toml) = fs::read_to_string(folder.join("recipe.toml")) else { return 0 };
        for line in toml.split('\n').filter(|l| !l.is_empty()) {
            let Some((key, value)) = line.split_once('=') else { continue };
            if key.trim_matches(is_blank) != "\"$schema\"" {
                continue;
            }
            let Some(value) = value.trim_matches(is_blank).split('"').find(|s| !s.is_empty()) else { continue };
            match FileFormat::parse(value) {
                Some((t, version)) if t == "recipe" => return version,
                _ => continue,
            }
        }
        0
    }

    /// Available recipes of a version this omaflake does not build (too old or too new): left out of what the image
    /// flake sees – one of them would stop every build – and named with the reason
    pub fn unreadable(&self, settings: &Settings) -> Vec<UnreadableFile> {
        self.available(settings)
            .into_iter()
            .filter_map(|e| {
                FileFormat::check(
                    Self::version_of(&e.folder),
                    Self::RECIPE_MINIMUM,
                    Self::RECIPE_VERSION,
                    "recipe.toml",
                )
                .err()
                .map(|err| UnreadableFile::new(e.id, err.message()))
            })
            .collect()
    }

    /// `requires = ["author/kind/name", …]` of a recipe.toml → recipe IDs
    pub fn requires_of(folder: &Path) -> Vec<String> {
        let Ok(toml) = fs::read_to_string(folder.join("recipe.toml")) else { return Vec::new() };
        let Some(line) = toml.split('\n').find(|l| l.trim_matches(is_blank).starts_with("requires")) else {
            return Vec::new();
        };
        let (Some(open), Some(close)) = (line.find('['), line.rfind(']')) else { return Vec::new() };
        if open >= close {
            return Vec::new();
        }
        line[open + 1..close]
            .split(',')
            .filter(|s| !s.is_empty())
            .filter_map(|s| RecipeRef::id(s.trim_matches(|c| c == ' ' || c == '"' || c == '\'')))
            .collect()
    }

    /// Recipes and the ones they require → `<folder>/<base>.omaflake-recipe` (`recipes/<author>/<kind>/<name>/` +
    /// manifest.json) – what people share
    pub fn export_recipe(&self, id: &str, folder: &Path, settings: &Settings) -> Result<PathBuf> {
        let available = self.available(settings);
        let Some(top) = available.iter().find(|a| a.id == id) else {
            return Err(Error::with_code(
                codes::NOT_FOUND,
                format!("{} is not available here", RecipeId::qualified(id)),
            ));
        };
        // The recipe and what it requires, as its build would see them (same source first)
        fn collect(
            lib: &RecipeLibrary,
            entry: &AvailableRecipe,
            available: &[AvailableRecipe],
            order: &mut Vec<(String, PathBuf)>,
        ) -> Result<()> {
            if order.iter().any(|(i, _)| RecipeId::base(i) == RecipeId::base(&entry.id)) {
                return Ok(());
            }
            order.push((entry.id.clone(), entry.folder.clone()));
            for req in RecipeLibrary::requires_of(&entry.folder) {
                let Some(r) = lib.requirement(&req, &entry.source, available) else {
                    return Err(Error::with_code(
                        codes::NOT_FOUND,
                        format!("{} is not available here", RecipeId::qualified(&req)),
                    ));
                };
                collect(lib, r, available, order)?;
            }
            Ok(())
        }
        let mut order = Vec::new();
        collect(self, top, &available, &mut order)?;
        let base = RecipeId::base(id);
        let dest = folder.join(format!("{base}.{}", Package::RECIPE));
        if dest.exists() {
            return Err(Error::with_code(codes::EXISTS, format!("{} exists already", dest.display())));
        }
        let partial = folder.join(format!(".{base}.{}.partial", Package::RECIPE));
        Disposal::delete(&[&partial])?;
        let result = (|| -> Result<()> {
            for (rid, source) in &order {
                let Some((author, kind, name)) = RecipeRef::parts(RecipeId::base(rid)) else { continue };
                let target = partial.join(format!("recipes/{author}/{}/{name}", kind.as_str()));
                for rel in Self::recipe_files(source)? {
                    let file = target.join(&rel);
                    if let Some(parent) = file.parent() {
                        fs::create_dir_all(parent).with_path(parent)?;
                    }
                    fs::copy(source.join(&rel), &file).with_path(&file)?;
                }
            }
            fs::create_dir_all(&partial).with_path(&partial)?;
            Manifest::new(PackageKind::Recipe, base, None, Some(id.to_string()), None, None).write(&partial)?;
            fs::rename(&partial, &dest).with_path(&dest)?;
            Ok(())
        })();
        if let Err(e) = result {
            let _ = Disposal::delete(&[&partial]);
            return Err(e);
        }
        Ok(dest)
    }

    /// The recipes in a package, a checkout, a folder of recipe folders or one recipe folder (whose place says who it
    /// is: …/<author>/<kind>/<name>); an error when there are none. (base ID, folder)
    pub fn recipes_in(&self, url: &Path) -> Result<Vec<(String, PathBuf)>> {
        let mut found = Self::recipe_folders(url);
        if found.is_empty() && url.join("recipe.toml").exists() {
            // Swift `standardizedFileURL.pathComponents.suffix(3)` ("/" is a component of its own)
            let components: Vec<String> =
                standardize(url).components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
            if components.len() >= 3 {
                let last = components[components.len() - 3..].join("/");
                if let Some(id) = RecipeRef::id(&last) {
                    found = vec![(id, url.to_path_buf())];
                }
            }
        }
        if found.is_empty() {
            return Err(Error::with_code(
                codes::NOT_FOUND,
                format!("No recipes in {} (expected <author>/<kind>/<name>/recipe.toml)", url.display()),
            ));
        }
        Ok(found)
    }

    /// Recipes from a package, a checkout or a folder of recipe folders (`<author>/<kind>/<name>/recipe.toml`), or
    /// one recipe folder → the downloaded recipes (source "import"). Same limits as a download; a recipe it requires
    /// must come along or be available already. A recipe whose content is here already is skipped; other content is a
    /// recipe of its own next to the ones there are (plan B110) – nothing is replaced. Returns the IDs written.
    pub fn import_recipes(&self, from: &Path) -> Result<Vec<String>> {
        let found = self.recipes_in(from)?;
        let available = self.available(&Settings::load(&self.paths));
        let mut known: HashSet<String> = available.iter().map(|a| RecipeId::base(&a.id).to_string()).collect();
        known.extend(found.iter().map(|(id, _)| id.clone()));
        for (base, folder) in &found {
            if !folder.join("default.nix").exists() {
                return Err(vm_err!("{} has no default.nix", RecipeId::qualified(base)));
            }
            if let Some(missing) = Self::requires_of(folder).into_iter().find(|r| !known.contains(r)) {
                return Err(Error::with_code(
                    codes::NOT_FOUND,
                    format!(
                        "{} requires {}, which is not here – nothing was imported",
                        RecipeId::qualified(base),
                        RecipeId::qualified(&missing)
                    ),
                ));
            }
        }
        let here: HashSet<String> = available.into_iter().map(|a| a.id).collect();
        let mut manifest = self.installed();
        let mut written = Vec::new();
        let recipes = self.paths.recipes();
        fs::create_dir_all(&recipes).with_path(&recipes)?;
        for (base, folder) in &found {
            let id = RecipeId::make_with_hash(base, &Self::content_hash(folder)?, OmaflakeId::LENGTH);
            if here.contains(&id) || manifest.contains_key(&id) {
                continue;
            }
            let Some(target) = self.folder(&id) else { continue };
            let work = recipes.join(format!(".import-{}", uuid()));
            let result = (|| -> Result<()> {
                for rel in Self::recipe_files(folder)? {
                    let source = folder.join(&rel);
                    let data = fs::read(&source).with_path(&source)?;
                    let file = work.join(&rel);
                    if let Some(parent) = file.parent() {
                        fs::create_dir_all(parent).with_path(parent)?;
                    }
                    fs::write(&file, &data).with_path(&file)?;
                }
                fs::rename(&work, &target).with_path(&target)?;
                Ok(())
            })();
            let _ = Disposal::delete(&[&work]);
            result?;
            manifest.insert(
                id.clone(),
                Installed {
                    source: Self::IMPORTED_SOURCE.into(),
                    version: RecipeId::words(&id).unwrap_or("").to_string(),
                    requires: Some(Self::requires_of(&target)),
                    installed: Self::install_time(&manifest),
                },
            );
            self.save_manifest(&manifest)?;
            written.push(id);
        }
        if !written.is_empty() {
            self.prune_versions();
        }
        Ok(written)
    }

    // MARK: Staging

    /// Puts every available recipe into one folder for the image flake's input `omaflake-recipes` (plan B110):
    /// `<cache>/recipes/<hash of the content>/<id>/<author>/<kind>/<name>/`, each recipe in a tree of its own with the
    /// recipes it requires next to it (same source first) – recipes import those by relative paths (`../minimal`), so
    /// each sees its own. `recipes.json` tells the flake which folder is which recipe (ID → path). The same content
    /// gives the same folder, so a change in a local folder is picked up by the next Nix call and nothing is copied
    /// twice. Symbolic links and Finder files are left out. Keeps the 8 most recently used staged folders.
    pub fn stage(&self, settings: &Settings) -> Result<PathBuf> {
        // Only recipes of a version this omaflake builds (`unreadable` names the others)
        let available: Vec<AvailableRecipe> =
            self.available(settings).into_iter().filter(|a| Self::readable(Self::version_of(&a.folder))).collect();
        let mut files: Vec<(String, PathBuf)> = Vec::new();
        let mut mapping: BTreeMap<String, String> = BTreeMap::new();
        fn place(
            lib: &RecipeLibrary,
            entry: &AvailableRecipe,
            tree: &str,
            available: &[AvailableRecipe],
            placed: &mut HashSet<String>,
            files: &mut Vec<(String, PathBuf)>,
        ) {
            let base = RecipeId::base(&entry.id);
            if !placed.insert(base.to_string()) {
                return;
            }
            let Some((author, kind, name)) = RecipeRef::parts(base) else { return };
            let prefix = format!("{tree}/{author}/{}/{name}", kind.as_str());
            for rel in RecipeLibrary::recipe_files(&entry.folder).unwrap_or_default() {
                files.push((format!("{prefix}/{rel}"), entry.folder.join(&rel)));
            }
            for req in RecipeLibrary::requires_of(&entry.folder) {
                if let Some(r) = lib.requirement(&req, &entry.source, available) {
                    place(lib, r, tree, available, placed, files);
                }
            }
        }
        for entry in &available {
            let Some((author, kind, name)) = RecipeRef::parts(RecipeId::base(&entry.id)) else { continue };
            let mut placed = HashSet::new();
            place(self, entry, &entry.id, &available, &mut placed, &mut files);
            mapping.insert(entry.id.clone(), format!("{}/{author}/{}/{name}", entry.id, kind.as_str()));
        }
        // Swift's JSONEncoder [.prettyPrinted, .sortedKeys] (slashes escaped) – the same bytes, the same hash
        let mapping_data = crate::json::to_pretty(&mapping, true)?;
        files.sort_by(|a, b| a.0.cmp(&b.0));
        let mut hasher = Sha256::new();
        let mut contents: Vec<(String, Vec<u8>)> = vec![("recipes.json".into(), mapping_data.clone())];
        hasher.update(&mapping_data);
        for (rel, source) in files {
            let data = fs::read(&source).with_path(&source)?;
            hasher.update(rel.as_bytes());
            hasher.update([0u8]);
            hasher.update(Sha256::digest(&data));
            contents.push((rel, data));
        }
        let hash = hex::encode(&hasher.finalize()[..10]);
        let root = self.paths.cache().join("recipes");
        let target = root.join(&hash);
        if target.exists() {
            touch(&target); // recently used
            return Ok(target);
        }
        let work = root.join(format!(".{hash}-{}", uuid()));
        fs::create_dir_all(&work).with_path(&work)?;
        for (rel, data) in &contents {
            let dest = work.join(rel);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent).with_path(parent)?;
            }
            fs::write(&dest, data).with_path(&dest)?;
        }
        if let Err(e) = fs::rename(&work, &target) {
            let _ = Disposal::delete(&[&work]); // another process staged the same content meanwhile
            if !target.exists() {
                return Err(Error::from(e).context(format!("{}: ", target.display())));
            }
        }
        self.prune(&root, Self::STAGED_KEPT);
        Ok(target)
    }

    /// Keeps the `keep` most recently used staged folders
    pub(crate) fn prune(&self, root: &Path, keep: usize) {
        let Ok(entries) = fs::read_dir(root) else { return };
        let mut dirs: Vec<(std::time::SystemTime, PathBuf)> = entries
            .flatten()
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
            .map(|e| {
                let modified = e.path().metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
                (modified, e.path())
            })
            .collect();
        dirs.sort_by_key(|d| Reverse(d.0));
        let old: Vec<PathBuf> = dirs.into_iter().skip(keep).map(|d| d.1).collect();
        let _ = Disposal::delete(&old);
    }
}

/// Sets a folder's modification date to now (Swift `setAttributes([.modificationDate: Date()])`)
fn touch(path: &Path) {
    if let Ok(f) = fs::File::open(path) {
        let _ = f.set_modified(std::time::SystemTime::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_store::ImageInfo;
    use crate::paths::tests::temp_paths;
    use serde_json::json;

    fn put(root: &Path, rel: &str, text: &str) {
        let u = root.join(rel);
        fs::create_dir_all(u.parent().unwrap()).unwrap();
        fs::write(u, text).unwrap();
    }

    /// A catalog entry for `id` whose files are written under `src` (Swift test `entry`)
    fn entry(src: &Path, id: &str, requires: &[&str], files: &[(&str, &str)]) -> Value {
        let (author, kind, name) = RecipeRef::parts(id).unwrap();
        let path = format!("recipes/{author}/{}/{name}", kind.as_str());
        let mut sorted = files.to_vec();
        sorted.sort();
        let list: Vec<Value> = sorted
            .iter()
            .map(|(f, text)| {
                put(src, &format!("{path}/{f}"), text);
                json!({"path": f, "size": text.len(), "sha256": sha256_hex(text.as_bytes())})
            })
            .collect();
        json!({"id": id, "author": author, "kind": kind.as_str(), "name": name, "path": path, "description": "d",
               "systems": ["aarch64-linux"], "requires": requires, "version": format!("v1-{name}"), "files": list})
    }

    fn catalog(recipes: &[Value]) -> RecipeCatalog {
        let json = json!({"$schema": "https://omaflake.dev/schema/catalog/v2.json", "source": {"name": "test"},
                          "recipes": recipes});
        RecipeCatalog::decode(&serde_json::to_vec(&json).unwrap()).unwrap()
    }

    const V1: &str = "\"$schema\" = \"https://omaflake.dev/schema/recipe/v1.json\"\n";

    fn two_recipes(src: &Path) -> Vec<Value> {
        vec![
            entry(
                src,
                "omaflake.nixos.minimal",
                &[],
                &[("recipe.toml", &format!("{V1}cpus = 1")), ("default.nix", "{ }")],
            ),
            entry(
                src,
                "someone.nixos.rust",
                &["omaflake/nixos/minimal"],
                &[
                    ("recipe.toml", &format!("{V1}cpus = 2\nrequires = [\"omaflake/nixos/minimal\"]\n")),
                    ("default.nix", "{ imports = [ ../../../omaflake/nixos/minimal ]; }"),
                ],
            ),
        ]
    }

    /// Settings that keep every version: `install`/`import_recipes` then skip `prune_versions` (and with it
    /// `ImageStore::list`, F1) – for the tests that run before the merge
    fn keeping(p: &Paths) -> Settings {
        let s = Settings { sources: Some(vec![]), keep_recipe_versions: Some(true), ..Default::default() };
        s.save(p).unwrap();
        s
    }

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
    fn parses_what_users_type() {
        let a = RecipeSource::parse("https://github.com/omaflake/recipes", false).unwrap();
        assert!(a.kind == RecipeSourceKind::Github && a.location == "omaflake/recipes" && a.branch.is_none());
        assert!(a.is_official() && a.is_private.is_none());
        let b = RecipeSource::parse("github.com/Some-One/my.recipes/tree/dev", true).unwrap();
        assert!(b.kind == RecipeSourceKind::Github && b.location == "Some-One/my.recipes");
        assert!(b.branch.as_deref() == Some("dev") && b.is_private_repository() && !b.is_official());
        assert_eq!(RecipeSource::parse("someone/recipes", false).unwrap().location, "someone/recipes");
        assert_eq!(RecipeSource::parse("https://www.github.com/a/b.git/", false).unwrap().location, "a/b");
        let c = RecipeSource::parse("https://example.org/omaflake/", false).unwrap();
        assert!(c.kind == RecipeSourceKind::Url && c.location == "https://example.org/omaflake/catalog.json");
        // as Foundation's URL appends a path component (checked with Swift)
        let u = |s: &str| RecipeSource::parse(s, false).unwrap().location;
        assert_eq!(u("https://example.org"), "https://example.org/catalog.json");
        assert_eq!(u("https://example.org/a?b=1"), "https://example.org/a/catalog.json?b=1");
        assert_eq!(u("HTTPS://example.org/x"), "HTTPS://example.org/x/catalog.json");
        assert_eq!(u("https://example.org/catalog.json"), "https://example.org/catalog.json");
        assert!(RecipeSource::parse("http://example.org/catalog.json", false).is_err());
        assert!(RecipeSource::parse("https://", false).is_err());
        assert!(RecipeSource::parse("  ", false).is_err());
        let (_t, p) = temp_paths();
        let dir = p.root.to_string_lossy().into_owned();
        assert_eq!(RecipeSource::parse(&dir, false).unwrap().kind, RecipeSourceKind::Folder);
        assert_eq!(RecipeSource::parse(&format!("file://{dir}"), false).unwrap().location, dir);
        let missing = RecipeSource::parse(&format!("{dir}/missing"), false).unwrap_err();
        assert_eq!(missing.code(), Some(codes::NOT_FOUND));
        // forks are not official, whatever their recipes' author says
        assert!(!RecipeSource::parse("someone/recipes", false).unwrap().is_official());
        assert_eq!(Settings::default().recipe_sources(), vec![RecipeSource::official()]);
    }

    /// Display names and file addresses as Swift's `URL` gives them
    #[test]
    fn names_and_addresses() {
        let url = |l: &str| RecipeSource::new(RecipeSourceKind::Url, l, None, None, None);
        assert_eq!(url("https://example.org/omaflake/catalog.json").display_name(), "example.org/omaflake");
        assert_eq!(url("https://example.org/catalog.json").display_name(), "example.org/");
        assert_eq!(url("https://Example.ORG:8443/x/catalog.json").display_name(), "Example.ORG/x");
        let gh = RecipeSource::new(RecipeSourceKind::Github, "a/b", Some("dev".into()), None, None);
        assert_eq!(gh.display_name(), "a/b (dev)");
        let client = |s: RecipeSource| SourceClient::new(s, Some("t".into()));
        assert_eq!(
            client(url("https://x.org/a/catalog.json?t=1")).url("recipes/a b/x.nix").unwrap(),
            "https://x.org/a/recipes/a%20b/x.nix?t=1"
        );
        assert_eq!(
            client(url("https://x.org/catalog.json")).url("catalog.json").unwrap(),
            "https://x.org/catalog.json"
        );
        assert_eq!(
            client(RecipeSource::official()).url("recipes/omaflake/nixos/minimal/recipe.toml").unwrap(),
            "https://raw.githubusercontent.com/omaflake/recipes/main/recipes/omaflake/nixos/minimal/recipe.toml"
        );
        let private = RecipeSource::new(RecipeSourceKind::Github, "a/b", None, Some(true), None);
        assert_eq!(
            client(private).url("catalog.json").unwrap(),
            "https://api.github.com/repos/a/b/contents/catalog.json?ref=main"
        );
    }

    #[test]
    fn references() {
        assert_eq!(RecipeRef::id("omaflake/nixos/minimal").as_deref(), Some("omaflake.nixos.minimal"));
        assert_eq!(RecipeRef::id("omaflake.macos.lab").as_deref(), Some("omaflake.macos.lab"));
        for bad in ["omaflake/linux/x", "../nixos/x", "a/nixos/B", "a/nixos", "-a/nixos/x", "Omaflake/nixos/minimal"] {
            assert!(RecipeRef::id(bad).is_none(), "{bad}");
        }
    }

    /// The content hash of a folder – the same as Swift's `RecipeLibrary.contentHash` (and tools/catalog.py's file
    /// list) gives: symbolic links, Finder files, a top-level manifest.json and `result` are left out
    #[test]
    fn content_hash_of_a_folder() {
        let (_t, p) = temp_paths();
        let f = p.root.join("r");
        put(&f, "recipe.toml", &format!("{V1}cpus = 1"));
        put(&f, "default.nix", "{ }");
        put(&f, "sub/x.nix", "x");
        put(&f, ".DS_Store", "finder");
        put(&f, "manifest.json", "{}");
        put(&f, "result/out", "nix");
        std::os::unix::fs::symlink(f.join("default.nix"), f.join("link.nix")).unwrap();
        assert_eq!(RecipeLibrary::recipe_files(&f).unwrap(), vec!["default.nix", "recipe.toml", "sub/x.nix"]);
        // computed with Swift's algorithm (sorted paths, path + NUL + SHA-256 of the file)
        let expected = "560177c420ae6e45fda27de95933c73f56b56ed8edde219857ec0239a8cb6fff";
        assert_eq!(RecipeLibrary::content_hash(&f).unwrap(), expected);
        // the catalog's file list gives the same
        let files =
            [("recipe.toml", format!("{V1}cpus = 1")), ("default.nix", "{ }".into()), ("sub/x.nix", "x".into())];
        let r = CatalogRecipe {
            id: "a.nixos.b".into(),
            author: "a".into(),
            kind: GuestKind::Nixos,
            name: "b".into(),
            path: "recipes/a/nixos/b".into(),
            description: "d".into(),
            descriptions: None,
            tags: None,
            license: None,
            lab: None,
            systems: vec![],
            requires: None,
            homepage: None,
            version: "v".into(),
            files: files
                .iter()
                .map(|(path, text)| CatalogFile {
                    path: path.to_string(),
                    size: text.len() as u64,
                    sha256: sha256_hex(text.as_bytes()).to_uppercase(),
                })
                .collect(),
        };
        assert_eq!(r.content_hash(), expected);
        put(&f, "big.bin", &"x".repeat(RecipeLibrary::MAX_FILE as usize + 1));
        assert_eq!(RecipeLibrary::recipe_files(&f).unwrap_err().message(), "big.bin is larger than 1 MiB");
    }

    #[test]
    fn hex_bytes_as_swift_reads_them() {
        assert_eq!(CatalogRecipe::bytes("00ff10"), vec![0, 255, 16]);
        assert_eq!(CatalogRecipe::bytes("0zff1"), vec![255]); // "0z" skipped, a single last digit ignored
        assert_eq!(CatalogRecipe::bytes("+f-0"), vec![15, 0]);
    }

    #[test]
    fn requires_and_descriptions() {
        let (_t, p) = temp_paths();
        put(&p.root, "recipe.toml", "cpus = 1\n  requires = [ \"omaflake/nixos/minimal\", 'a.nixos.b', \"bad\" ]\n");
        assert_eq!(RecipeLibrary::requires_of(&p.root), vec!["omaflake.nixos.minimal", "a.nixos.b"]);
        let mut c = catalog(&[json!({"id": "a.nixos.b", "author": "a", "kind": "nixos", "name": "b",
            "path": "recipes/a/nixos/b", "description": "en", "descriptions": {"de": "de"}, "systems": ["aarch64-linux"],
            "version": "v", "files": []})]);
        let r = c.recipes.remove(0);
        assert_eq!(r.description_in(Some("de")), "de");
        assert_eq!(r.description_in(Some("fr")), "en");
        assert_eq!(r.description_in(None), "en");
        assert_eq!(r.runs_here(), Platform::host_system(GuestKind::Nixos) == Some("aarch64-linux"));
    }

    /// A local catalog folder as source: installing checks every file, rejects paths outside the recipe, brings
    /// what a recipe requires; staging puts local folders first and gives the same folder for the same content.
    /// (Swift `installStageRemove` without the removals and with kept versions – see the ignored test below.)
    #[test]
    fn install_and_stage() {
        let (_t, paths) = temp_paths();
        keeping(&paths);
        let src = paths.root.join("source");
        let mut recipes = two_recipes(&src);
        let source = RecipeSource::new(RecipeSourceKind::Folder, &src.to_string_lossy(), None, None, None);
        let lib = RecipeLibrary::new(paths.clone());
        let client = SourceClient::new(source, None);
        let installed = lib.install("someone/nixos/rust", &catalog(&recipes), &client).unwrap();
        let bases: Vec<&str> = installed.iter().map(|i| RecipeId::base(i)).collect();
        assert_eq!(bases, ["omaflake.nixos.minimal", "someone.nixos.rust"]);
        let (minimal, rust) = (installed[0].clone(), installed[1].clone());
        // the ID's words are the catalog's content hash, the same as the downloaded folder's
        let hash = catalog(&recipes).recipes[1].content_hash();
        assert_eq!(rust, RecipeId::make_with_hash("someone.nixos.rust", &hash, OmaflakeId::LENGTH));
        assert!(paths.recipes().join(&rust).join("default.nix").exists());
        assert_eq!(lib.installed()[&rust].requires.as_deref(), Some(&["omaflake.nixos.minimal".to_string()][..]));
        assert!(lib.install("someone/nixos/rust", &catalog(&recipes), &client).unwrap().is_empty()); // same content
        assert_eq!(
            lib.install("someone/nixos/nothing", &catalog(&recipes), &client).unwrap_err().code(),
            Some(codes::NOT_FOUND)
        );

        // a new version whose served file does not match the catalog: nothing is written
        put(&src, "recipes/someone/nixos/rust/default.nix", "{ evil = true; }");
        recipes[1]["version"] = json!("v2");
        recipes[1]["files"][0]["sha256"] = json!(sha256_hex(b"{ v2 }")); // default.nix (sorted first)
        recipes[1]["files"][0]["size"] = json!(6);
        let e = lib.install("someone/nixos/rust", &catalog(&recipes), &client).unwrap_err();
        assert!(e.message().contains("does not match the catalog"), "{e}");
        let kept = fs::read_to_string(paths.recipes().join(&rust).join("default.nix")).unwrap();
        assert!(kept.contains("imports"));
        // a path leaving the recipe's folder is refused
        // (an extra file: with default.nix renamed the entry would already fail for lacking it)
        let escape = json!({"path": "../../../../escape", "size": 1, "sha256": sha256_hex(b"x")});
        recipes[1]["files"].as_array_mut().unwrap().insert(0, escape);
        let e = lib.install("someone/nixos/rust", &catalog(&recipes), &client).unwrap_err();
        assert!(e.message().contains("leaves the recipe's folder"), "{e}");
        // no work folders are left behind
        let left: Vec<String> = fs::read_dir(paths.recipes())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(".download-"))
            .collect();
        assert!(left.is_empty(), "{left:?}");

        // staging: the downloaded recipes, a local folder's first; the same content, the same folder
        let mut settings = Settings { sources: Some(vec![]), ..Default::default() };
        let staged = lib.stage(&settings).unwrap();
        // rust's tree has the minimal it requires next to it
        assert!(staged.join(&rust).join("omaflake/nixos/minimal/recipe.toml").exists());
        assert!(staged.join(&minimal).join("omaflake/nixos/minimal/recipe.toml").exists());
        let mapping: BTreeMap<String, String> =
            serde_json::from_slice(&fs::read(staged.join("recipes.json")).unwrap()).unwrap();
        assert_eq!(mapping[&rust], format!("{rust}/someone/nixos/rust"));
        assert_eq!(lib.stage(&settings).unwrap(), staged);
        let local = paths.root.join("local-recipes/omaflake/nixos/minimal");
        put(&local, "recipe.toml", &format!("{V1}cpus = 7"));
        put(&local, "default.nix", "{ }");
        let local_source = RecipeSource::new(
            RecipeSourceKind::Folder,
            &paths.root.join("local-recipes").to_string_lossy(),
            None,
            None,
            None,
        );
        settings.sources = Some(vec![local_source.clone()]);
        let with_local = lib.stage(&settings).unwrap();
        assert_ne!(with_local, staged);
        // another minimal (other content, other source) next to the downloaded one; rust still gets its own source's
        let available = lib.available(&settings);
        let local_minimal = available.iter().find(|a| a.source == local_source.id()).unwrap().id.clone();
        assert!(RecipeId::base(&local_minimal) == "omaflake.nixos.minimal" && local_minimal != minimal);
        assert_eq!(available.len(), 3);
        let read = |p: PathBuf| fs::read_to_string(p).unwrap();
        assert_eq!(
            read(with_local.join(&local_minimal).join("omaflake/nixos/minimal/recipe.toml")),
            format!("{V1}cpus = 7")
        );
        assert_eq!(read(with_local.join(&rust).join("omaflake/nixos/minimal/recipe.toml")), format!("{V1}cpus = 1"));
        let d = lib.distinctions(&settings);
        assert!(d.contains_key(&local_minimal) && !d.contains_key(&rust));
        assert_eq!(d[&minimal], client.source.id()); // a source no longer in the settings: its ID
        assert_eq!(
            lib.source_name(RecipeLibrary::IMPORTED_SOURCE, &settings),
            abbreviate_tilde(&paths.recipes().to_string_lossy())
        );
        // a recipe of a newer version is left out of what the flake sees, and named
        put(&local, "recipe.toml", "\"$schema\" = \"https://omaflake.dev/schema/recipe/v9.json\"\n");
        let unreadable = lib.unreadable(&settings);
        assert_eq!(unreadable.len(), 1);
        assert_eq!(unreadable[0].reason, "recipe.toml has version 9 – this omaflake reads up to 1; update omaflake");
        let staged_again = lib.stage(&settings).unwrap();
        assert_eq!(staged_again, staged);
    }

    /// The full Swift `installStageRemove` (removals need `ImageStore::list`, F1) – run after the merge
    #[test]
    fn install_stage_remove() {
        let (_t, paths) = temp_paths();
        let src = paths.root.join("source");
        let recipes = two_recipes(&src);
        let source = RecipeSource::new(RecipeSourceKind::Folder, &src.to_string_lossy(), None, None, None);
        let lib = RecipeLibrary::new(paths.clone());
        let client = SourceClient::new(source, None);
        let installed = lib.install("someone/nixos/rust", &catalog(&recipes), &client).unwrap();
        let (minimal, rust) = (installed[0].clone(), installed[1].clone());
        assert!(lib.remove(&minimal).is_err()); // still required
        let settings = Settings { sources: Some(vec![]), ..Default::default() };
        lib.stage(&settings).unwrap();
        lib.remove(&rust).unwrap();
        lib.remove(&minimal).unwrap();
        assert!(lib.installed().is_empty());
    }

    /// Export (with what it requires) and import into another data folder
    #[test]
    fn export_and_import() {
        let (_t, paths) = temp_paths();
        keeping(&paths);
        let src = paths.root.join("source");
        let recipes = two_recipes(&src);
        let source = RecipeSource::new(RecipeSourceKind::Folder, &src.to_string_lossy(), None, None, None);
        let lib = RecipeLibrary::new(paths.clone());
        let ids = lib.install("someone/nixos/rust", &catalog(&recipes), &SourceClient::new(source, None)).unwrap();
        let out = paths.root.join("out");
        fs::create_dir_all(&out).unwrap();
        let settings = Settings::load(&paths);
        let package = lib.export_recipe(&ids[1], &out, &settings).unwrap();
        assert_eq!(package, out.join("someone.nixos.rust.omaflake-recipe"));
        assert!(package.join("recipes/omaflake/nixos/minimal/default.nix").exists());
        assert_eq!(Manifest::read(&package).unwrap().recipe.as_deref(), Some(ids[1].as_str()));
        assert_eq!(lib.export_recipe(&ids[1], &out, &settings).unwrap_err().code(), Some(codes::EXISTS));
        assert_eq!(lib.export_recipe("a.nixos.b@x", &out, &settings).unwrap_err().code(), Some(codes::NOT_FOUND));

        let (_t2, other) = temp_paths();
        keeping(&other);
        let lib2 = RecipeLibrary::new(other.clone());
        let mut imported = lib2.import_recipes(&package).unwrap();
        imported.sort();
        let mut expected = ids.clone();
        expected.sort();
        assert_eq!(imported, expected); // the same content, the same IDs
        assert!(lib2.import_recipes(&package).unwrap().is_empty());
        assert_eq!(lib2.installed()[&ids[0]].source, RecipeLibrary::IMPORTED_SOURCE);
        // one recipe folder: its place says who it is; what it requires must be here
        let (_t3, third) = temp_paths();
        keeping(&third);
        let one = package.join("recipes/someone/nixos/rust");
        assert_eq!(RecipeLibrary::new(third.clone()).recipes_in(&one).unwrap()[0].0, "someone.nixos.rust");
        let e = RecipeLibrary::new(third).import_recipes(&one).unwrap_err();
        assert!(e.message().contains("requires omaflake/nixos/minimal, which is not here"), "{e}");
        assert_eq!(lib2.recipes_in(&other.root).unwrap_err().code(), Some(codes::NOT_FOUND));
    }

    /// A local folder source is verified by its recipes; a remote one needs a token when private
    #[test]
    fn verify_and_fetch() {
        let (_t, paths) = temp_paths();
        let src = paths.root.join("source");
        let client = |l: &Path| {
            SourceClient::new(RecipeSource::new(RecipeSourceKind::Folder, &l.to_string_lossy(), None, None, None), None)
        };
        assert_eq!(client(&src).verify().unwrap_err().code(), Some(codes::NOT_FOUND));
        two_recipes(&src);
        assert_eq!(client(&src).verify().unwrap(), "2 recipes");
        assert_eq!(client(&src).fetch("catalog.json").unwrap_err().code(), Some(codes::NOT_FOUND));
        let private = RecipeSource::new(RecipeSourceKind::Github, "a/b", None, Some(true), None);
        let no_token = SourceClient { source: private, token: None };
        assert_eq!(no_token.fetch("catalog.json").unwrap_err().code(), Some(codes::AUTH));
        assert_eq!(no_token.verify().unwrap_err().code(), Some(codes::AUTH));
    }

    /// The public omaflake/recipes catalog: read over the network, and – with a ../recipes checkout next to this one
    /// – each recipe's catalog hash equals its folder's
    #[test]
    #[ignore = "network"]
    fn official_catalog() {
        let client = SourceClient::new(RecipeSource::official(), None);
        let c = client.catalog().unwrap();
        assert!(!c.recipes.is_empty());
        assert!(client.verify().unwrap().starts_with(&format!("{}: ", c.source.name)));
        let checkout = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../recipes");
        let local = fs::read(checkout.join("catalog.json")).ok().map(|d| RecipeCatalog::decode(&d).unwrap());
        for r in local.map(|l| l.recipes).unwrap_or_default() {
            assert_eq!(RecipeLibrary::content_hash(&checkout.join(&r.path)).unwrap(), r.content_hash(), "{}", r.id);
        }
    }

    // MARK: Versions (Swift RecipeVersionTests – need F1's ImageStore)

    fn recipe(root: &Path, text: &str) {
        let d = root.join("me/nixos/desk");
        put(&d, "recipe.toml", &format!("{V1}description = \"x\"\n"));
        put(&d, "default.nix", text);
    }

    fn image(p: &Paths, recipe: &str, source: &str, store: &str) -> PathBuf {
        let img = p.image_dir(&format!("me.nixos.desk@{}", OmaflakeId::words_of(store, OmaflakeId::LENGTH)));
        fs::create_dir_all(&img).unwrap();
        let info = ImageInfo::new(
            recipe,
            Some(source.into()),
            GuestKind::Nixos,
            "aarch64-linux",
            "",
            "r",
            Some(store.into()),
            crate::json::now(),
        );
        ImageStore::write_info(&info, &img).unwrap();
        fs::write(img.join("disk.qcow2"), b"").unwrap();
        img
    }

    /// Imports make versions of one line; without keeping only the newest stays – and one an image is built from
    #[test]
    fn versions_are_kept_while_used_or_when_asked() {
        let (_t, p) = temp_paths();
        let lib = RecipeLibrary::new(p.clone());
        let src = p.root.join("src");
        let mut s = Settings { sources: Some(vec![]), ..Default::default() };
        s.save(&p).unwrap();
        recipe(&src, "{ v = 1; }");
        let v1 = lib.import_recipes(&src).unwrap()[0].clone();
        image(&p, &v1, "import", "/nix/store/a");
        recipe(&src, "{ v = 2; }");
        let v2 = lib.import_recipes(&src).unwrap()[0].clone();
        recipe(&src, "{ v = 3; }");
        let v3 = lib.import_recipes(&src).unwrap()[0].clone();
        // v2 went (not newest, no image), v1 stays (its image), v3 is the newest
        let line = RecipeId::line_key(&v1, "import");
        let ids = |lib: &RecipeLibrary| -> Vec<String> {
            lib.versions(&Settings::load(&p))[&line].iter().map(|v| v.id.clone()).collect()
        };
        assert_eq!(ids(&lib), vec![v3.clone(), v1.clone()]);
        assert!(lib.versions(&Settings::load(&p))[&line][0].is_newest);
        assert!(!p.recipes().join(&v2).exists());
        assert!(lib.remove(&v1).is_err()); // kept for its image
                                           // with keeping, an older version stays
        s.keep_recipe_versions = Some(true);
        s.save(&p).unwrap();
        recipe(&src, "{ v = 4; }");
        let v4 = lib.import_recipes(&src).unwrap()[0].clone();
        assert_eq!(ids(&lib), vec![v4.clone(), v3.clone(), v1.clone()]);
        // all versions: what can go goes, the one with an image stays
        let (removed, kept) = lib.remove_line(&line);
        assert_eq!(removed.iter().cloned().collect::<HashSet<_>>(), HashSet::from([v4, v3]));
        assert_eq!(kept.len(), 1);
        assert_eq!(ids(&lib), vec![v1]);
    }

    /// A build keeps the version of a local folder's recipe it used; the folder changes, the version stays
    #[test]
    fn build_keeps_the_version_of_a_local_folder() {
        let (_t, p) = temp_paths();
        let lib = RecipeLibrary::new(p.clone());
        let folder = p.root.join("mine");
        recipe(&folder, "{ v = 1; }");
        let source = RecipeSource::new(RecipeSourceKind::Folder, &folder.to_string_lossy(), None, None, None);
        let s = Settings { sources: Some(vec![source.clone()]), ..Default::default() };
        s.save(&p).unwrap();
        let v1 = lib.available(&s)[0].id.clone();
        assert!(lib.versions(&s).values().next().unwrap()[0].is_live);
        lib.keep(&v1, &s);
        assert!(p.recipes().join(&v1).join("default.nix").exists());
        let img = image(&p, &v1, &source.id(), "/nix/store/b");
        recipe(&folder, "{ v = 2; }");
        let list = lib.versions(&s).values().next().unwrap().clone();
        assert!(
            list.len() == 2 && list[0].is_live && list[0].is_newest && list[1].id == v1 && list[1].images.len() == 1
        );
        assert!(lib.prune_versions().is_empty()); // used by its image
        Disposal::delete(&[img]).unwrap();
        assert_eq!(lib.prune_versions(), vec![v1]); // no image any more, not the newest
        assert!(lib.remove(&list[0].id).is_err()); // the folder's own recipe is changed there
    }

    /// `keep` copies a local folder's recipe and what it requires (without `ImageStore`)
    #[test]
    fn keep_copies_a_local_folder_recipe() {
        let (_t, p) = temp_paths();
        let lib = RecipeLibrary::new(p.clone());
        let folder = p.root.join("mine");
        let src = folder.clone();
        two_recipes(&src);
        let source = RecipeSource::new(RecipeSourceKind::Folder, &folder.to_string_lossy(), None, None, None);
        let s = Settings { sources: Some(vec![source.clone()]), ..Default::default() };
        let available = lib.available(&s);
        let rust = available.iter().find(|a| RecipeId::base(&a.id) == "someone.nixos.rust").unwrap().id.clone();
        lib.keep(&rust, &s);
        let installed = lib.installed();
        assert_eq!(installed.len(), 2);
        assert!(installed.values().all(|i| i.source == source.id()));
        assert_eq!(installed[&rust].version, RecipeId::words(&rust).unwrap());
        assert!(p.recipes().join(&rust).join("recipe.toml").exists());
        // the same content from the folder and the kept copy is one recipe
        assert_eq!(lib.available(&s).len(), 2);
    }

    // MARK: File formats (Swift FileFormatTests, the parts of this package)

    /// docs/schema/<type>/v<n>.json of this checkout
    fn schema(url: &str) -> Value {
        let (t, v) = FileFormat::parse(url).unwrap();
        let file = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../docs/schema/{t}/v{v}.json"));
        serde_json::from_slice(&fs::read(&file).unwrap_or_else(|_| panic!("{}", file.display()))).unwrap()
    }

    /// The keys of `data` are the schema's, and its required ones are there (a shallow check of what omaflake writes)
    fn matches(data: &[u8]) {
        let object: Map<String, Value> = serde_json::from_slice(data).unwrap();
        let s = schema(object["$schema"].as_str().unwrap());
        let properties: HashSet<&String> = s["properties"].as_object().map(|p| p.keys().collect()).unwrap_or_default();
        let extra: Vec<&String> = object.keys().filter(|k| !properties.contains(k)).collect();
        assert!(extra.is_empty(), "keys not in the schema: {extra:?}");
        let missing: Vec<&str> = s["required"]
            .as_array()
            .map(|r| r.iter().filter_map(Value::as_str).filter(|k| !object.contains_key(*k)).collect())
            .unwrap_or_default();
        assert!(missing.is_empty(), "required keys missing: {missing:?}");
    }

    /// Every version up to the current one is in docs/schema/ – a published version is never removed (the catalog's
    /// schemas start at v2: v1 was `"schema": 1`, before B114)
    #[test]
    fn every_version_has_its_schema() {
        let formats = [
            ImageInfo::FILE_FORMAT,
            crate::models::InstanceConfig::FILE_FORMAT,
            Settings::FILE_FORMAT,
            RecipeLibrary::FILE_FORMAT,
            crate::mac_vm::MacVmConfig::FILE_FORMAT,
            Manifest::FILE_FORMAT,
            crate::source_tokens::TokenFile::FILE_FORMAT,
        ];
        let mut urls: Vec<String> =
            formats.iter().flat_map(|f| (1..=f.current()).map(|v| FileFormat::schema_for(f.type_, v))).collect();
        urls.extend((2..=RecipeCatalog::VERSION).map(|v| FileFormat::schema_for("catalog", v)));
        urls.extend((1..=RecipeLibrary::RECIPE_VERSION).map(|v| FileFormat::schema_for("recipe", v)));
        for url in urls {
            assert_eq!(schema(&url)["$id"].as_str(), Some(url.as_str()));
        }
    }

    #[test]
    fn what_omaflake_writes_matches_its_schema() {
        use crate::guest_settings::GuestOverrides;
        let info = ImageInfo::new(
            "omaflake.nixos.minimal@a",
            Some("s".into()),
            GuestKind::Nixos,
            "aarch64-linux",
            "/f",
            "r",
            Some("/nix/store/x".into()),
            crate::json::now(),
        );
        matches(&ImageInfo::FILE_FORMAT.encode(&info).unwrap());
        matches(&crate::source_tokens::sample_file());
        let mut c = crate::models::tests::config("t1");
        c.identity = Some("a-b-c".into());
        c.share_path = Some("/s".into());
        matches(&crate::models::InstanceConfig::FILE_FORMAT.encode(&c).unwrap());
        let s = Settings {
            flake: Some("/f".into()),
            sources: Some(vec![RecipeSource::official()]),
            guest: Some(GuestOverrides { user: Some("u".into()), ..Default::default() }),
            ..Default::default()
        };
        matches(&Settings::FILE_FORMAT.encode(&s).unwrap());
        let mac = crate::mac_vm::MacVmConfig::new(4, 8192, 80, "02:00:00:00:00:01", Some("26A434".into()));
        matches(&crate::mac_vm::MacVmConfig::FILE_FORMAT.encode(&mac).unwrap());
        let m = Manifest::new(PackageKind::Vm, "a", Some(GuestKind::Nixos), None, None, None);
        matches(&Manifest::FILE_FORMAT.encode(&m).unwrap());
        let installed = InstalledFile {
            recipes: BTreeMap::from([(
                "x".to_string(),
                Installed { source: "s".into(), version: "v".into(), requires: None, installed: Utc::now() },
            )]),
        };
        matches(&RecipeLibrary::FILE_FORMAT.encode(&installed).unwrap());
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
        // through the library: read, written back at the current version, read again
        let (_t, p) = temp_paths();
        put(&p.recipes(), ".installed.json", std::str::from_utf8(old).unwrap());
        let lib = RecipeLibrary::new(p.clone());
        assert_eq!(lib.installed()["omaflake.nixos.minimal@abcd1234"].version, "v");
        matches(&fs::read(p.recipes().join(".installed.json")).unwrap());
        assert_eq!(lib.installed().len(), 1);
    }

    #[test]
    fn recipe_version_is_read_from_the_toml() {
        let (_t, p) = temp_paths();
        let folder = p.root.join("omaflake/nixos/minimal");
        put(&folder, "recipe.toml", "format = 1\ncpus = 1\n");
        assert_eq!(RecipeLibrary::version_of(&folder), 0);
        put(
            &folder,
            "recipe.toml",
            "# \"$schema\" = \"https://omaflake.dev/schema/recipe/v7.json\"\n\"$schema\" = \"https://omaflake.dev/schema/recipe/v1.json\"   # the version\ncpus = 1\n",
        );
        assert_eq!(RecipeLibrary::version_of(&folder), 1);
        // Recipes: version 0 (no $schema) is still built, a newer one is not
        assert!(RecipeLibrary::readable(0) && RecipeLibrary::readable(1) && !RecipeLibrary::readable(2));
    }

    #[test]
    fn catalog_version_one_is_read() {
        let v1 = br#"{"schema":1,"source":{"name":"old"},"recipes":[{"id":"a.macos.m","author":"a","kind":"macos","name":"m","path":"recipes/a/macos/m","description":"d","version":"v","files":[]}]}"#;
        let catalog = RecipeCatalog::decode(v1).unwrap();
        assert_eq!(catalog.recipes[0].systems, vec!["aarch64-darwin"]);
        let v3 = br#"{"$schema":"https://omaflake.dev/schema/catalog/v3.json","source":{"name":"x"},"recipes":[]}"#;
        assert_eq!(RecipeCatalog::decode(v3).unwrap_err().code(), Some(codes::NEWER_FILE));
        let foreign = br#"{"$schema":"https://omaflake.dev/schema/image/v1.json","source":{"name":"x"},"recipes":[]}"#;
        assert!(RecipeCatalog::decode(foreign)
            .unwrap_err()
            .message()
            .starts_with("Not a recipe catalog (catalog.json): "));
        assert_eq!(RecipeCatalog::decode(b"[").unwrap_err().message(), "Not a recipe catalog (catalog.json): not JSON");
        // version 0 (no schema at all) is too old
        assert_eq!(
            RecipeCatalog::decode(br#"{"source":{"name":"x"},"recipes":[]}"#).unwrap_err().code(),
            Some(codes::UNREADABLE_FILE)
        );
    }
}
