// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of OmaflakeID.swift.
//!
//! omaflake IDs (plan B110): one readable form for VMs, images and recipes – `<kind>:<identifier>:<words>`:
//!   recipe:omaflake/nixos/minimal:velvet-tiger-orbit   identifier author/kind/name, words from the content hash
//!   image:omaflake/nixos/minimal:cedar-mango-lift      identifier the recipe's author/kind/name, words from the store path
//!   vm:research:quiet-otter-lamp                       identifier the VM's name, words from its identity (random, new
//!                                                      for a copy under another name)
//! The words are the first 132 bits of a hash as twelve words of BIP-39's English list (plan B111) – unique worldwide,
//! like a UUID. They are shown only as long as needed (`shortest`): one word, two as soon as one word is not enough
//! for two objects of the same kind and identifier, and so on, the same length everywhere for a kind. Files and Nix
//! attributes use the file form `<identifier with dots>@<words>` (`RecipeId`, image package names); a VM's package
//! keeps its name.

use std::collections::{HashMap, HashSet};
use std::io::Read;

use sha2::{Digest, Sha256};

use crate::image_store::{Image, ImageStore};
use crate::instance_store::InstanceStore;
use crate::models::{InstanceConfig, RecipeId};
use crate::paths::{Paths, Settings};
use crate::recipe_sources::RecipeLibrary;
use crate::word_list::WORDS;

/// Swift `OmaflakeID.Kind`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OmaflakeIdKind {
    Vm,
    Image,
    Recipe,
}

impl OmaflakeIdKind {
    pub const ALL: [OmaflakeIdKind; 3] = [OmaflakeIdKind::Vm, OmaflakeIdKind::Image, OmaflakeIdKind::Recipe];

    pub fn as_str(self) -> &'static str {
        match self {
            OmaflakeIdKind::Vm => "vm",
            OmaflakeIdKind::Image => "image",
            OmaflakeIdKind::Recipe => "recipe",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

/// An omaflake ID taken apart (Swift's `(kind, identifier, words)` tuple of `parse`)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedId {
    pub kind: OmaflakeIdKind,
    pub identifier: String,
    pub words: String,
}

/// Swift `enum OmaflakeID` (namespace)
pub enum OmaflakeId {}

impl OmaflakeId {
    /// Words in an ID: 12 × 11 = 132 bits
    pub const LENGTH: usize = 12;

    /// The word list (Swift `OmaflakeID.list`)
    pub fn list() -> &'static [&'static str; 2048] {
        &WORDS
    }

    fn known(word: &str) -> bool {
        WORDS.binary_search(&word).is_ok()
    }

    /// `count` words (11 bits each) for the first bits of `digest`; "" when the digest is too short
    pub fn words(digest: &[u8], count: usize) -> String {
        if digest.len() * 8 < count * 11 {
            return String::new();
        }
        let bit = |i: usize| ((digest[i / 8] >> (7 - (i % 8))) & 1) as usize;
        (0..count)
            .map(|w| WORDS[(0..11).fold(0usize, |acc, b| (acc << 1) | bit(w * 11 + b))])
            .collect::<Vec<_>>()
            .join("-")
    }

    /// Words of a hex digest ("3fa2c1d0…"); reading stops at the first pair that is not hex
    pub fn words_hex(hex: &str, count: usize) -> String {
        let mut bytes = Vec::new();
        let raw = hex.as_bytes();
        let mut i = 0;
        while i + 2 <= raw.len() {
            let Ok(pair) = std::str::from_utf8(&raw[i..i + 2]) else { break };
            let Ok(b) = u8::from_str_radix(pair, 16) else { break };
            bytes.push(b);
            i += 2;
        }
        Self::words(&bytes, count)
    }

    /// Words of SHA-256 of a string (e.g. a store path)
    pub fn words_of(string: &str, count: usize) -> String {
        Self::words(&Sha256::digest(string.as_bytes()), count)
    }

    /// New random words – a VM's identity
    pub fn random_words() -> String {
        let mut bytes = [0u8; 17];
        let filled = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes));
        if filled.is_err() {
            // No /dev/urandom: the clock and the process are not random, but still give distinct identities
            let seed = format!("{:?}{}", std::time::SystemTime::now(), std::process::id());
            bytes.copy_from_slice(&Sha256::digest(seed.as_bytes())[..17]);
        }
        Self::words(&bytes, Self::LENGTH)
    }

    /// Words of the list joined by "-": all of an ID's (`full`) or the start of them
    pub fn is_words(s: &str, full: bool) -> bool {
        let p: Vec<&str> = s.split('-').collect();
        let count_ok = if full { p.len() == Self::LENGTH } else { (1..=Self::LENGTH).contains(&p.len()) };
        count_ok && p.iter().all(|w| Self::known(w))
    }

    // MARK: Shown as short as possible

    /// The first `count` words (at least one)
    pub fn short(words: &str, count: usize) -> String {
        words.split('-').filter(|w| !w.is_empty()).take(count.max(1)).collect::<Vec<_>>().join("-")
    }

    /// How many words tell apart the objects of each group (one group: the same kind and identifier) – at least one
    pub fn shortest<I: IntoIterator<Item = Vec<String>>>(groups: I) -> usize {
        let groups: Vec<Vec<String>> = groups.into_iter().collect();
        for n in 1..Self::LENGTH {
            let ok = groups.iter().all(|g| {
                let shortened: HashSet<String> = g.iter().map(|w| Self::short(w, n)).collect();
                let distinct: HashSet<&String> = g.iter().collect();
                shortened.len() == distinct.len()
            });
            if ok {
                return n;
            }
        }
        Self::LENGTH
    }

    pub fn make(kind: OmaflakeIdKind, identifier: &str, words: &str) -> String {
        format!("{}:{identifier}:{words}", kind.as_str())
    }

    /// (kind, identifier, words) of an ID in its readable form; None otherwise
    pub fn parse(id: &str) -> Option<ParsedId> {
        let p: Vec<&str> = id.split(':').collect();
        if p.len() != 3 || p[1].is_empty() || !Self::is_words(p[2], false) {
            return None;
        }
        Some(ParsedId { kind: OmaflakeIdKind::parse(p[0])?, identifier: p[1].into(), words: p[2].into() })
    }

    /// "omaflake.nixos.minimal@velvet-tiger-orbit-…" → "recipe:omaflake/nixos/minimal:velvet" (as many words as `n`;
    /// Swift's default is `LENGTH`)
    pub fn recipe(recipe_id: &str, n: usize) -> String {
        let words = RecipeId::words(recipe_id).map(|w| Self::short(w, n)).unwrap_or_else(|| "?".into());
        Self::make(OmaflakeIdKind::Recipe, &RecipeId::qualified(recipe_id), &words)
    }

    pub fn image(image: &Image, n: usize) -> String {
        let words = RecipeId::words(&image.name).map(|w| Self::short(w, n)).unwrap_or_else(|| "?".into());
        Self::make(OmaflakeIdKind::Image, &RecipeId::qualified(image.recipe()), &words)
    }

    pub fn vm(c: &InstanceConfig, n: usize) -> String {
        let words = c.identity.as_deref().map(|w| Self::short(w, n)).unwrap_or_else(|| "?".into());
        Self::make(OmaflakeIdKind::Vm, &c.name, &words)
    }
}

/// How many words each kind shows (plan B111): the same for every object of a kind (Swift `OmaflakeID.Lengths`)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lengths {
    pub recipe: usize,
    pub image: usize,
    pub vm: usize,
}

impl Default for Lengths {
    fn default() -> Self {
        Lengths { recipe: 1, image: 1, vm: 1 }
    }
}

impl Lengths {
    pub const FULL: Lengths = Lengths { recipe: OmaflakeId::LENGTH, image: OmaflakeId::LENGTH, vm: OmaflakeId::LENGTH };

    /// From the recipe IDs, image names and VMs there are
    pub fn new(recipes: &[String], images: &[String], vms: &[InstanceConfig]) -> Self {
        fn groups(ids: &[String]) -> Vec<Vec<String>> {
            let mut g: HashMap<&str, Vec<String>> = HashMap::new();
            for id in ids {
                let entry = g.entry(RecipeId::base(id)).or_default();
                if let Some(w) = RecipeId::words(id) {
                    entry.push(w.to_string());
                }
            }
            g.into_values().collect()
        }
        let mut by_name: HashMap<&str, Vec<String>> = HashMap::new();
        for c in vms {
            let entry = by_name.entry(c.name.as_str()).or_default();
            if let Some(i) = &c.identity {
                entry.push(i.clone());
            }
        }
        Lengths {
            recipe: OmaflakeId::shortest(groups(recipes)),
            image: OmaflakeId::shortest(groups(images)),
            vm: OmaflakeId::shortest(by_name.into_values()),
        }
    }

    /// The lengths for what this data folder has: every recipe version, image and VM
    pub fn current(paths: &Paths) -> Self {
        let settings = Settings::load(paths);
        let recipes: Vec<String> =
            RecipeLibrary::new(paths.clone()).versions(&settings).into_values().flatten().map(|v| v.id).collect();
        let images: Vec<String> = ImageStore::new(paths.clone()).list().into_iter().map(|i| i.name).collect();
        let vms = InstanceStore::new(paths.clone(), "qemu-img").list();
        Self::new(&recipes, &images, &vms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::tests::config;

    #[test]
    fn word_list_is_bip39() {
        assert_eq!(WORDS.len(), 2048);
        assert_eq!(WORDS[0], "abandon");
        assert_eq!(WORDS[2047], "zoo");
        let file = WORDS.join("\n") + "\n";
        assert_eq!(
            hex::encode(Sha256::digest(file.as_bytes())),
            "2f5eed53a4727b4bf8880d8f3f199efc90e58503646d9ff8eff3a2ed3b24dbda"
        );
        assert!(WORDS.windows(2).all(|w| w[0] < w[1]), "sorted, for binary search");
    }

    #[test]
    fn words_spell_hashes() {
        let zero = [0u8; 17];
        let ones = [0xffu8; 17];
        assert_eq!(OmaflakeId::words(&zero, 12), ["abandon"; 12].join("-"));
        assert!(OmaflakeId::words(&ones, 12).starts_with("zoo-zoo-zoo"));
        // 11 bits each: 00000000 001… → word 1 ("ability")
        let mut b = vec![0x00, 0x20];
        b.extend_from_slice(&zero);
        assert!(OmaflakeId::words(&b, 12).starts_with("ability-"));
        assert_eq!(OmaflakeId::words_of("x", 12).split('-').count(), 12);
        assert_eq!(OmaflakeId::words_of("x", 12), OmaflakeId::words_of("x", 12));
        assert_ne!(OmaflakeId::words_of("x", 12), OmaflakeId::words_of("y", 12));
        assert!(OmaflakeId::is_words(&OmaflakeId::random_words(), true));
        assert!(OmaflakeId::is_words("abandon-ability", false) && !OmaflakeId::is_words("abandon-ability", true));
        assert!(!OmaflakeId::is_words("abandon-notaword", false));
        assert_eq!(OmaflakeId::words(&[0u8; 2], 12), "");
        assert_eq!(OmaflakeId::words_hex(&"00".repeat(17), 1), "abandon");
    }

    /// As short as possible, the same length for every object of a kind (plan B111)
    #[test]
    fn shown_as_short_as_needed() {
        let (a, b, c) = ("able-acid-also-x", "able-acid-zoo-x", "zoo-acid-also-x");
        let g = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(OmaflakeId::shortest(vec![g(&[a, c])]), 1);
        assert_eq!(OmaflakeId::shortest(vec![g(&[a, c]), g(&[b])]), 1);
        assert_eq!(OmaflakeId::shortest(vec![g(&[a, b])]), 3); // two of one identifier share two words
        assert_eq!(OmaflakeId::shortest(vec![g(&[a]), g(&[b])]), 1); // different identifiers need no telling apart
        assert_eq!(OmaflakeId::short(a, 2), "able-acid");
        let l = Lengths::new(
            &[format!("o.nixos.m@{a}"), format!("o.nixos.m@{b}"), format!("o.nixos.n@{a}")],
            &[format!("o.nixos.m@{a}")],
            &[],
        );
        assert!(l.recipe == 3 && l.image == 1 && l.vm == 1);
    }

    #[test]
    fn ids_of_the_three_kinds() {
        let r = format!("omaflake.nixos.minimal@{}", OmaflakeId::words(&[0u8; 17], 12));
        assert_eq!(OmaflakeId::recipe(&r, 1), "recipe:omaflake/nixos/minimal:abandon");
        assert_eq!(OmaflakeId::recipe(&r, 12).split(':').nth(2).unwrap().split('-').count(), 12);
        assert_eq!(
            OmaflakeId::parse(&OmaflakeId::recipe(&r, 2)).unwrap(),
            ParsedId {
                kind: OmaflakeIdKind::Recipe,
                identifier: "omaflake/nixos/minimal".into(),
                words: "abandon-abandon".into()
            }
        );
        let mut c = config("research");
        c.identity = Some("quiet-otter-lamp".into());
        assert_eq!(OmaflakeId::vm(&c, 2), "vm:research:quiet-otter");
        assert_eq!(OmaflakeId::parse("vm:research:not-words"), None);
        // a recipe by its omaflake ID (or the start of its words), by base
        let other = format!("omaflake.nixos.minimal@{}", OmaflakeId::words(&[0xffu8; 17], 12));
        let ids = vec![r.clone(), other.clone()];
        assert_eq!(RecipeId::candidates("recipe:omaflake/nixos/minimal:zoo", &ids), vec![other.clone()]);
        assert_eq!(RecipeId::candidates("minimal", &ids).len(), 2);
        assert_eq!(RecipeId::candidates("omaflake.nixos.minimal@zoo", &ids), vec![other]);
    }
}
