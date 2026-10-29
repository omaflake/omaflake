// SPDX-License-Identifier: AGPL-3.0-or-later
//! Profiles (plan B141, B142): how big a VM is – CPU cores and RAM –, created and named like locales and kept in
//! config.json (`profiles`, `defaultProfile`), never in a catalog. A new VM is created with one (the default unless
//! another is chosen; "custom" sets the values by hand). A recipe says what a VM from it needs at least
//! (`[requirements]`); a smaller profile gets a warning, the VM can still be created. The disk belongs to the VM:
//! the recipe's size, changed when it is created.

use serde::{Deserialize, Serialize};

use crate::error::{Result, codes};
use crate::paths::{Paths, Settings};
use crate::vm_err;

/// One profile in config.json
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    /// Stable ID ("p-…")
    pub id: String,
    /// The user's own name for it ("Small", "Big")
    pub name: String,
    pub cpus: u32,
    #[serde(rename = "memoryGB")]
    pub memory_gb: u32,
}

/// What a profile's fields are changed to: None leaves a field as it is
#[derive(Debug, Clone, Default)]
pub struct ProfileChange {
    pub name: Option<String>,
    pub cpus: Option<u32>,
    pub memory_gb: Option<u32>,
}

pub struct Profiles {
    paths: Paths,
}

impl Profiles {
    /// The most a profile gives (the New VM page's ranges)
    pub const MAX_CPUS: u32 = 64;
    pub const MAX_MEMORY_GB: u32 = 512;

    pub fn new(paths: Paths) -> Profiles {
        Profiles { paths }
    }

    pub fn list(&self) -> Vec<Profile> {
        Settings::load(&self.paths).profiles.unwrap_or_default()
    }

    pub fn get(&self, id: &str) -> Option<Profile> {
        self.list().into_iter().find(|p| p.id == id)
    }

    /// By ID, else by name (any case)
    pub fn find(&self, id_or_name: &str) -> Option<Profile> {
        let all = self.list();
        all.iter()
            .find(|p| p.id == id_or_name)
            .or_else(|| all.iter().find(|p| p.name.eq_ignore_ascii_case(id_or_name)))
            .cloned()
    }

    /// The default: the settings' `defaultProfile`, else the first – None only without any
    pub fn default_profile(&self) -> Option<Profile> {
        let s = Settings::load(&self.paths);
        let all = s.profiles.unwrap_or_default();
        s.default_profile.and_then(|id| all.iter().find(|p| p.id == id).cloned()).or_else(|| all.into_iter().next())
    }

    fn check(p: &Profile) -> Result<()> {
        if p.name.trim().is_empty() {
            return Err(vm_err!("A profile needs a name"));
        }
        if p.cpus == 0 || p.cpus > Self::MAX_CPUS || p.memory_gb == 0 || p.memory_gb > Self::MAX_MEMORY_GB {
            return Err(vm_err!("A profile has 1 to 64 CPU cores and 1 to 512 GB of RAM"));
        }
        Ok(())
    }

    fn apply(p: &mut Profile, c: &ProfileChange) {
        if let Some(v) = &c.name {
            p.name = v.trim().to_string();
        }
        if let Some(v) = c.cpus {
            p.cpus = v;
        }
        if let Some(v) = c.memory_gb {
            p.memory_gb = v;
        }
    }

    /// A new profile (the first one becomes the default)
    pub fn add(&self, change: ProfileChange) -> Result<Profile> {
        let mut s = Settings::load(&self.paths);
        let mut all = s.profiles.clone().unwrap_or_default();
        let mut p = Profile { id: random_id(), ..Default::default() };
        Self::apply(&mut p, &change);
        Self::check(&p)?;
        if all.iter().any(|x| x.name.eq_ignore_ascii_case(&p.name)) {
            return Err(vm_err!(codes::EXISTS, "There is a profile \"{}\" already", p.name));
        }
        all.push(p.clone());
        if s.default_profile.as_ref().is_none_or(|id| !all.iter().any(|x| &x.id == id)) {
            s.default_profile = Some(p.id.clone());
        }
        s.profiles = Some(all);
        s.save(&self.paths)?;
        Ok(p)
    }

    /// A profile's values change; VMs created with it keep theirs
    pub fn update(&self, id: &str, change: ProfileChange) -> Result<Profile> {
        let mut s = Settings::load(&self.paths);
        let mut all = s.profiles.clone().unwrap_or_default();
        let Some(i) = all.iter().position(|p| p.id == id) else {
            return Err(vm_err!(codes::NOT_FOUND, "No profile \"{}\"", id));
        };
        let mut p = all[i].clone();
        Self::apply(&mut p, &change);
        Self::check(&p)?;
        if all.iter().any(|x| x.id != id && x.name.eq_ignore_ascii_case(&p.name)) {
            return Err(vm_err!(codes::EXISTS, "There is a profile \"{}\" already", p.name));
        }
        all[i] = p.clone();
        s.profiles = Some(all);
        s.save(&self.paths)?;
        Ok(p)
    }

    /// Removes a profile; when it was the default, another one becomes it
    pub fn remove(&self, id: &str) -> Result<Profile> {
        let mut s = Settings::load(&self.paths);
        let mut all = s.profiles.clone().unwrap_or_default();
        let Some(i) = all.iter().position(|p| p.id == id) else {
            return Err(vm_err!(codes::NOT_FOUND, "No profile \"{}\"", id));
        };
        let p = all.remove(i);
        if s.default_profile.as_deref() == Some(id) {
            s.default_profile = all.first().map(|x| x.id.clone());
        }
        s.profiles = (!all.is_empty()).then_some(all);
        s.save(&self.paths)?;
        Ok(p)
    }

    pub fn set_default(&self, id: &str) -> Result<()> {
        let mut s = Settings::load(&self.paths);
        if !s.profiles.as_ref().is_some_and(|all| all.iter().any(|p| p.id == id)) {
            return Err(vm_err!(codes::NOT_FOUND, "No profile \"{}\"", id));
        }
        s.default_profile = Some(id.to_string());
        s.save(&self.paths)
    }
}

/// A random profile ID: "p-" and 12 hex digits
pub fn random_id() -> String {
    format!("p-{}", &crate::credentials::random_id()[2..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::tests::temp_paths;

    fn change(name: &str, cpus: u32, memory_gb: u32) -> ProfileChange {
        ProfileChange { name: Some(name.into()), cpus: Some(cpus), memory_gb: Some(memory_gb) }
    }

    #[test]
    fn add_update_remove_and_the_default() {
        let (_t, p) = temp_paths();
        let profiles = Profiles::new(p);
        assert!(profiles.default_profile().is_none());
        let small = profiles.add(change("Small", 2, 4)).unwrap();
        assert!(small.id.starts_with("p-") && small.id.len() == 14);
        assert_eq!(profiles.default_profile().map(|x| x.id), Some(small.id.clone()));
        assert!(profiles.add(change("small", 4, 8)).is_err(), "names are unique, any case");
        assert!(profiles.add(change("Zero", 0, 8)).is_err());
        assert!(profiles.add(change("Huge", 4, 1024)).is_err());
        let big = profiles.add(change("Big", 8, 16)).unwrap();
        let bigger = profiles.update(&big.id, ProfileChange { memory_gb: Some(32), ..Default::default() }).unwrap();
        assert_eq!((bigger.cpus, bigger.memory_gb), (8, 32));
        assert_eq!(profiles.find("BIG").map(|x| x.id), Some(big.id.clone()));
        profiles.set_default(&big.id).unwrap();
        profiles.remove(&big.id).unwrap();
        assert_eq!(profiles.default_profile().map(|x| x.id), Some(small.id), "another becomes the default");
    }
}
