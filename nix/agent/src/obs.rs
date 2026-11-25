// SPDX-License-Identifier: AGPL-3.0-or-later
//! OBS as the camera in macOS guests until omaflake's own camera extension (M12, plan B169). OBS is a stopgap:
//! nothing installs it – when the user has, the agent writes one scene collection, "omaflake Camera", whose media
//! source reads the agent's camera stream (camera_mac.rs), and makes it OBS's current collection. The user's own
//! collections are never touched, an existing "omaflake Camera" is left as it is, and nothing is written while OBS
//! runs (it writes its settings when it quits). The rest is the user's: approving OBS's camera extension and "Start
//! Virtual Camera".
//!
//! The file work is plain functions over a home folder, so it is tested on every platform; only `prepare` looks at
//! the running system.

#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use serde_json::json;
use std::path::{Path, PathBuf};

pub const COLLECTION: &str = "omaflake Camera";
/// OBS names a collection's file after it, spaces as underscores
pub const COLLECTION_FILE: &str = "omaflake_Camera.json";
const APP: &str = "/Applications/OBS.app";
/// The camera stream (camera_mac.rs) as OBS's media source reads it
const INPUT: &str = "tcp://127.0.0.1:47042";
const FFMPEG_OPTIONS: &str = "video_size=1280x720 pixel_format=nv12 framerate=30 use_wallclock_as_timestamps=1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// OBS is not installed
    NotInstalled,
    /// Installed, but never started: no settings yet to add to (OBS's first start sets them up)
    NeverStarted,
    /// OBS runs and the collection is not there yet: written once OBS has quit
    Running,
    /// The collection is there (`created`: written just now and made the current one)
    Ready {
        created: bool,
    },
    Failed(String),
}

impl State {
    /// For the log and the diagnosis
    pub fn describe(&self) -> String {
        match self {
            State::NotInstalled => "OBS is not installed".into(),
            State::NeverStarted => "OBS has not been started yet".into(),
            State::Running => format!("OBS runs; the scene collection \"{COLLECTION}\" follows once it has quit"),
            State::Ready { created: true } => format!("wrote OBS's scene collection \"{COLLECTION}\" and chose it"),
            State::Ready { created: false } => format!("OBS's scene collection \"{COLLECTION}\" is there"),
            State::Failed(e) => format!("OBS's scene collection: {e}"),
        }
    }
}

/// On the running guest: OBS in /Applications, OBS's process, the user's home
pub fn prepare() -> State {
    let installed = Path::new(APP).exists();
    let running = installed
        && std::process::Command::new("/usr/bin/pgrep")
            .args(["-x", "OBS"])
            .stdout(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
    match std::env::var_os("HOME") {
        Some(home) => prepare_in(Path::new(&home), installed, running),
        None => State::Failed("no HOME".into()),
    }
}

/// OBS's camera extension's state as `systemextensionsctl list` shows it ("activated enabled", "activated waiting
/// for user", …); None: not installed (OBS installs it the first time its virtual camera starts)
pub fn extension_state() -> Option<String> {
    let out = std::process::Command::new("/usr/bin/systemextensionsctl").arg("list").output().ok()?;
    extension_state_in(&String::from_utf8_lossy(&out.stdout))
}

const EXTENSION: &str = "com.obsproject.obs-studio.mac-camera-extension";

pub fn extension_state_in(list: &str) -> Option<String> {
    let line = list.lines().rfind(|l| l.contains(EXTENSION))?;
    let state = line.rsplit_once('[')?.1.trim_end().trim_end_matches(']');
    Some(state.to_string())
}

pub fn config_dir(home: &Path) -> PathBuf {
    home.join("Library/Application Support/obs-studio")
}

pub fn prepare_in(home: &Path, installed: bool, running: bool) -> State {
    if !installed {
        return State::NotInstalled;
    }
    let config = config_dir(home);
    let scene = config.join("basic/scenes").join(COLLECTION_FILE);
    if scene.exists() {
        return State::Ready { created: false };
    }
    // The current collection is in user.ini since OBS 31, in global.ini before (and there without ".json")
    let (ini, file_value) = if config.join("user.ini").exists() {
        (config.join("user.ini"), COLLECTION_FILE)
    } else if config.join("global.ini").exists() {
        (config.join("global.ini"), COLLECTION_FILE.trim_end_matches(".json"))
    } else {
        return State::NeverStarted;
    };
    if running {
        return State::Running;
    }
    let result = (|| -> std::io::Result<()> {
        let mut text = std::fs::read_to_string(&ini)?;
        let canvas = canvas(&config, &text);
        std::fs::create_dir_all(scene.parent().expect("has a parent"))?;
        let collection = serde_json::to_string_pretty(&scene_collection(canvas)).expect("serializable");
        std::fs::write(&scene, collection)?;
        text = ini_set(&text, "Basic", "SceneCollection", COLLECTION);
        text = ini_set(&text, "Basic", "SceneCollectionFile", file_value);
        std::fs::write(&ini, text)
    })();
    match result {
        Ok(()) => State::Ready { created: true },
        Err(e) => State::Failed(e.to_string()),
    }
}

/// The canvas of OBS's current profile (`[Video] BaseCX/BaseCY`), else OBS's default 1920×1080
fn canvas(config: &Path, ini: &str) -> (u32, u32) {
    let size = || {
        let dir = ini_get(ini, "Basic", "ProfileDir")?;
        let basic = std::fs::read_to_string(config.join("basic/profiles").join(dir).join("basic.ini")).ok()?;
        let cx = ini_get(&basic, "Video", "BaseCX")?.parse().ok()?;
        let cy = ini_get(&basic, "Video", "BaseCY")?.parse().ok()?;
        Some((cx, cy)).filter(|&(x, y): &(u32, u32)| x > 0 && y > 0)
    };
    size().unwrap_or((1920, 1080))
}

/// One scene with one media source reading the camera stream, fitted into the canvas
pub fn scene_collection((cx, cy): (u32, u32)) -> serde_json::Value {
    let source = "Mac camera (omaflake)";
    let source_uuid = "6f0e2b8a-0c1d-4e57-9a3b-0a1f6c2d7e41";
    json!({
        "name": COLLECTION,
        "current_scene": "Mac camera",
        "current_program_scene": "Mac camera",
        "scene_order": [{ "name": "Mac camera" }],
        "sources": [
            {
                "name": source,
                "uuid": source_uuid,
                "id": "ffmpeg_source",
                "versioned_id": "ffmpeg_source",
                "settings": {
                    "is_local_file": false,
                    "input": INPUT,
                    "input_format": "rawvideo",
                    "ffmpeg_options": FFMPEG_OPTIONS,
                    "buffering_mb": 1,
                    "reconnect_delay_sec": 2,
                    "hw_decode": false,
                    "close_when_inactive": false,
                    "restart_on_activate": false,
                    "clear_on_media_end": false,
                    "seekable": false
                }
            },
            {
                "name": "Mac camera",
                "uuid": "1c9d4e3f-5a6b-4c7d-8e9f-0a1b2c3d4e5f",
                "id": "scene",
                "versioned_id": "scene",
                "settings": {
                    "id_counter": 1,
                    "custom_size": false,
                    "items": [{
                        "name": source,
                        "source_uuid": source_uuid,
                        "visible": true,
                        "locked": false,
                        "id": 1,
                        "pos": { "x": 0.0, "y": 0.0 },
                        "scale": { "x": 1.0, "y": 1.0 },
                        "align": 5,
                        // OBS_BOUNDS_SCALE_INNER: the picture fills the canvas, keeping its shape
                        "bounds_type": 2,
                        "bounds_align": 0,
                        "bounds": { "x": cx as f64, "y": cy as f64 }
                    }]
                }
            }
        ]
    })
}

/// A key's value in an INI section (OBS writes `Key=Value`, no spaces)
pub fn ini_get(text: &str, section: &str, key: &str) -> Option<String> {
    let mut inside = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line == format!("[{section}]");
        } else if inside
            && let Some((k, v)) = line.split_once('=')
            && k.trim() == key
        {
            return Some(v.trim().to_string());
        }
    }
    None
}

/// Sets a key in an INI section: the line is replaced in place, a missing key goes to the section's end, a missing
/// section to the file's end; every other line stays as it was
pub fn ini_set(text: &str, section: &str, key: &str, value: &str) -> String {
    let header = format!("[{section}]");
    let entry = format!("{key}={value}");
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let Some(start) = lines.iter().position(|l| l.trim() == header) else {
        if lines.last().is_some_and(|l| !l.trim().is_empty()) {
            lines.push(String::new());
        }
        lines.push(header);
        lines.push(entry);
        return lines.join("\n") + "\n";
    };
    let end = lines[start + 1..].iter().position(|l| l.trim().starts_with('[')).map_or(lines.len(), |i| start + 1 + i);
    match lines[start + 1..end].iter().position(|l| l.split_once('=').is_some_and(|(k, _)| k.trim() == key)) {
        Some(i) => lines[start + 1 + i] = entry,
        None => {
            // after the section's last non-empty line
            let at = (start + 1..end).rev().find(|&i| !lines[i].trim().is_empty()).map_or(start + 1, |i| i + 1);
            lines.insert(at, entry);
        }
    }
    lines.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("omaflake-obs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(config_dir(&dir)).unwrap();
        dir
    }

    const USER_INI: &str = "[General]\nFirstRun=true\n\n[Basic]\nProfile=Untitled\nProfileDir=Untitled\n\
SceneCollection=Untitled\nSceneCollectionFile=Untitled.json\n\n[Video]\nRenderer=Metal\n";

    #[test]
    fn ini_set_replaces_adds_and_keeps_the_rest() {
        let t = ini_set(USER_INI, "Basic", "SceneCollection", COLLECTION);
        assert_eq!(ini_get(&t, "Basic", "SceneCollection").as_deref(), Some(COLLECTION));
        assert_eq!(ini_get(&t, "Basic", "Profile").as_deref(), Some("Untitled"));
        assert_eq!(ini_get(&t, "Video", "Renderer").as_deref(), Some("Metal"));
        let t = ini_set(&t, "Basic", "New", "1");
        assert_eq!(ini_get(&t, "Basic", "New").as_deref(), Some("1"));
        assert!(t.find("New=1").unwrap() < t.find("[Video]").unwrap(), "added inside its section");
        let t = ini_set("[General]\nA=1\n", "Basic", "SceneCollection", COLLECTION);
        assert_eq!(t, "[General]\nA=1\n\n[Basic]\nSceneCollection=omaflake Camera\n");
        assert_eq!(ini_get(&t, "General", "SceneCollection"), None, "keys are per section");
    }

    #[test]
    fn not_installed_never_started_and_running_write_nothing() {
        let h = home("states");
        assert_eq!(prepare_in(&h, false, false), State::NotInstalled);
        assert_eq!(prepare_in(&h, true, false), State::NeverStarted);
        std::fs::write(config_dir(&h).join("user.ini"), USER_INI).unwrap();
        assert_eq!(prepare_in(&h, true, true), State::Running);
        assert!(!config_dir(&h).join("basic/scenes").join(COLLECTION_FILE).exists());
        assert_eq!(std::fs::read_to_string(config_dir(&h).join("user.ini")).unwrap(), USER_INI);
    }

    #[test]
    fn writes_the_collection_once_and_makes_it_current() {
        let h = home("write");
        let config = config_dir(&h);
        std::fs::write(config.join("user.ini"), USER_INI).unwrap();
        std::fs::create_dir_all(config.join("basic/profiles/Untitled")).unwrap();
        std::fs::write(config.join("basic/profiles/Untitled/basic.ini"), "[Video]\nBaseCX=2560\nBaseCY=1440\n")
            .unwrap();
        std::fs::create_dir_all(config.join("basic/scenes")).unwrap();
        std::fs::write(config.join("basic/scenes/Untitled.json"), "{\"name\":\"Untitled\"}").unwrap();

        assert_eq!(prepare_in(&h, true, false), State::Ready { created: true });
        let ini = std::fs::read_to_string(config.join("user.ini")).unwrap();
        assert_eq!(ini_get(&ini, "Basic", "SceneCollection").as_deref(), Some(COLLECTION));
        assert_eq!(ini_get(&ini, "Basic", "SceneCollectionFile").as_deref(), Some(COLLECTION_FILE));
        assert_eq!(ini_get(&ini, "General", "FirstRun").as_deref(), Some("true"));
        let scene: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(config.join("basic/scenes").join(COLLECTION_FILE)).unwrap())
                .unwrap();
        assert_eq!(scene["sources"][0]["settings"]["input"], INPUT);
        assert_eq!(scene["sources"][1]["settings"]["items"][0]["bounds"]["x"], 2560.0);
        // The user's own collection is untouched
        assert_eq!(
            std::fs::read_to_string(config.join("basic/scenes/Untitled.json")).unwrap(),
            "{\"name\":\"Untitled\"}"
        );

        // The user chose another collection since: it stays theirs
        std::fs::write(config.join("user.ini"), USER_INI).unwrap();
        assert_eq!(prepare_in(&h, true, false), State::Ready { created: false });
        assert_eq!(std::fs::read_to_string(config.join("user.ini")).unwrap(), USER_INI);
    }

    #[test]
    fn extension_state_from_systemextensionsctl() {
        let list = "1 extension(s)\n--- com.apple.system_extension.cmio\nenabled\tactive\tteamID\tbundleID (version)\tname\t[state]\n\
                    *\t*\t2MMRE5MTB8\tcom.obsproject.obs-studio.mac-camera-extension (33.0.2/44751735020)\tOBS Virtual Camera\t[activated enabled]\n";
        assert_eq!(extension_state_in(list).as_deref(), Some("activated enabled"));
        let waiting = list.replace("activated enabled", "activated waiting for user");
        assert_eq!(extension_state_in(&waiting).as_deref(), Some("activated waiting for user"));
        assert_eq!(extension_state_in("0 extension(s)\n"), None);
    }

    #[test]
    fn older_obs_keeps_the_collection_in_global_ini_without_json() {
        let h = home("global");
        std::fs::write(config_dir(&h).join("global.ini"), "[Basic]\nSceneCollection=Untitled\n").unwrap();
        assert_eq!(prepare_in(&h, true, false), State::Ready { created: true });
        let ini = std::fs::read_to_string(config_dir(&h).join("global.ini")).unwrap();
        assert_eq!(ini_get(&ini, "Basic", "SceneCollectionFile").as_deref(), Some("omaflake_Camera"));
    }
}
