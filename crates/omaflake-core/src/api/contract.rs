// SPDX-License-Identifier: AGPL-3.0-or-later
//! Contract tests of the API's JSON (docs/api.md): a representative value of every result, event and error the server
//! sends is serialized and compared with the golden files in `crates/omaflake-core/testdata/api/` – the same files
//! the Swift app's tests decode with its DTOs (Tests/OmaflakeKitTests/APIContractTests.swift). A change of a shape
//! breaks one side's test instead of a client.
//!
//! The params files (`*.params.json`) go the other way: what a client sends, parsed here with the server's types.
//!
//! Regenerate after a deliberate change: `OMAF_UPDATE_GOLDEN=1 cargo test -p omaflake-core api::contract`, then run the
//! Swift tests and update docs/api.md.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Serialize;
use serde_json::{json, Value};

use super::protocol::{self, RpcError};
use super::types::*;
use crate::agent_protocol::AgentState;
use crate::error::codes;
use crate::json::parse_date;
use crate::models::{DisplayMode, GuestKind, NetworkMode, StopResult, VmStatus};
use crate::prerequisites::{PrerequisiteItem, PrerequisiteStatus};
use crate::recipe_sources::RecipeSourceKind;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/api")
}

pub(crate) fn date(s: &str) -> chrono::DateTime<chrono::Utc> {
    parse_date(s).unwrap()
}

/// Every golden file: (file name without `.json`, its JSON)
fn goldens() -> Vec<(&'static str, Value)> {
    fn v<T: Serialize>(t: T) -> Value {
        serde_json::to_value(t).unwrap()
    }
    let vm = vm_full();
    let vm_list = VmList {
        vms: vec![vm.clone(), vm_mac()],
        unreadable: vec![Unreadable { name: "old".into(), reason: "written by a newer omaflake (version 9)".into() }],
    };
    let image_list = ImageList {
        images: vec![image(false), image_old()],
        unreadable: vec![Unreadable { name: "broken".into(), reason: "image.json is damaged".into() }],
    };
    let recipes = recipe_list();
    let doctor = doctor();
    let jobs = jobs();
    vec![
        ("initialize.result", v(initialize_result())),
        ("settings.get", v(settings_answer())),
        ("settings.set", v(settings_answer().settings)),
        (
            "updates.check",
            v(UpdateAnswer {
                state: UpdateState::Available,
                release: Some(ReleaseDto {
                    version: "0.2.0".into(),
                    url: "https://example.org/omaflake-0.2.0.zip".into(),
                    sha256: Some("ab12".into()),
                    notes: Some("Faster builds".into()),
                    page: Some("https://example.org/releases/0.2.0".into()),
                }),
            }),
        ),
        ("updates.check.upToDate", v(UpdateAnswer { state: UpdateState::UpToDate, release: None })),
        ("vms.list", v(&vm_list)),
        ("vms.get", v(&vm)),
        ("vms.stop", v(StopAnswer { result: StopResult::Powerdown })),
        ("vms.set", v(VmSetAnswer { vm: vm_mac(), takes_effect_at_next_start: true })),
        (
            "vms.ssh",
            v(SshAnswer {
                argv: vec!["ssh".into(), "-p".into(), "2201".into(), "alex@127.0.0.1".into()],
                shell_line: "ssh -p 2201 alex@127.0.0.1".into(),
            }),
        ),
        ("vms.agent", agent()),
        ("vms.log", v(LogLines { lines: vec!["<<< NixOS Stage 1 >>>".into(), "login:".into()] })),
        ("vms.wait", v(WaitAnswer { via: Via::Agent })),
        ("vms.share", v(PathAnswer { path: "/data/share/research".into() })),
        ("vms.nextName", v(NameAnswer { name: "dev-2".into() })),
        ("vms.limits", v(Limits { mac_max: 2, mac_running: vec!["work".into()] })),
        ("images.list", v(&image_list)),
        ("images.get", v(image(true))),
        ("recipes.list", v(&recipes)),
        ("recipes.available", v(available())),
        ("recipes.removeLine", v(LineRemoval { removed: vec!["omaflake.nixos.dev@old".into()], kept: vec![] })),
        ("recipes.pruneVersions", v(Pruned { removed: vec!["omaflake.nixos.dev@old".into()] })),
        ("sources.list", v(vec![source_official(), source_private()])),
        ("sources.add", v(SourceAdded { source: source_private(), note: "acme/recipes: 3 recipes".into() })),
        (
            "sources.check",
            v(SourceChecks {
                results: BTreeMap::from([
                    ("github:omaflake/recipes@main".into(), SourceCheck { ok: true, note: "12 recipes".into() }),
                    ("url:https://example.org/catalog.json".into(), SourceCheck { ok: false, note: "HTTP 404".into() }),
                ]),
            }),
        ),
        (
            "packages.inspect",
            v(Inspection {
                kind: PackageKindDto::Vm,
                valid: true,
                reason: None,
                conflict: Some(ConflictDto {
                    vm_name: Some("research".into()),
                    suggested_name: Some("research-2".into()),
                    image: None,
                }),
                network_volume: false,
            }),
        ),
        (
            "packages.inspect.invalid",
            v(Inspection {
                kind: PackageKindDto::Image,
                valid: false,
                reason: Some(super::views::Text::verbatim("Not an image package (.omaflake-image)")),
                conflict: None,
                network_volume: true,
            }),
        ),
        ("doctor.check", v(&doctor)),
        ("jobs.list", v(&jobs)),
        ("jobs.log", v(JobLog { lines: vec!["building '/nix/store/x.drv'".into()], next: 1 })),
        // Events (params)
        ("event.vms.changed", v(&vm_list)),
        ("event.vm.ended", v(VmEnded { name: "research".into(), requested: false })),
        ("event.vm.ready", v(VmReady { name: "research".into(), via: Via::Ssh })),
        ("event.images.changed", v(&image_list)),
        ("event.recipes.changed", v(&recipes)),
        ("event.jobs.changed", v(JobChanged { job: jobs[0].clone() })),
        ("event.job.log", v(JobLines { id: "j1".into(), lines: vec!["[2/5 built]".into(), "copying".into()] })),
        ("event.doctor.changed", v(&doctor)),
        ("event.message", v(Message { text: "Older recipe versions removed: 2".into(), level: Level::Info })),
        // Errors: the whole response
        (
            "error.vm-running",
            protocol::response_err(json!(7), &RpcError::server(Some(codes::VM_RUNNING), "running – stop it first")),
        ),
        (
            "error.network-volume",
            protocol::response_err(
                json!(8),
                &RpcError::server(None, "x.omaflake-vm is on a network volume.")
                    .with_data("networkVolume", json!(true)),
            ),
        ),
    ]
}

fn initialize_result() -> InitializeResult {
    InitializeResult {
        api_version: API_VERSION,
        omaf_version: "0.1.0".into(),
        host: Host { system: "aarch64-darwin".into(), os: "macOS 27.0.1".into() },
        data_folder: "/data".into(),
        config_file: "/data/config.json".into(),
    }
}

pub(crate) fn settings_answer() -> SettingsAnswer {
    SettingsAnswer {
        settings: SettingsDto {
            developer_mode: true,
            check_for_updates: true,
            notifications: false,
            keep_recipe_versions: false,
            guest: GuestValues {
                user: Some("alex".into()),
                locale: Some("de_CH.UTF-8".into()),
                keyboard: Some("ch".into()),
                ..GuestValues::default()
            },
            flake: Some("/src/omaflake/nix".into()),
            qemu: None,
            nix: None,
            runner: None,
            flake_in_use: Some("/src/omaflake/nix".into()),
            data_folder: "/data".into(),
        },
        guest_defaults: GuestValues {
            user: Some("mac".into()),
            full_name: Some("Mac User".into()),
            email: None,
            time_zone: Some("Europe/Zurich".into()),
            locale: Some("de_CH.UTF-8".into()),
            keyboard: Some("ch".into()),
        },
        choices: Choices {
            time_zones: vec!["Europe/Berlin".into(), "Europe/Zurich".into()],
            locales: vec!["de_CH.UTF-8".into(), "en_US.UTF-8".into()],
            keyboards: vec![
                Keyboard { code: "ch".into(), locale: "de_CH".into() },
                Keyboard { code: "us".into(), locale: "en_US".into() },
            ],
        },
        is_default_data_folder: false,
    }
}

pub(crate) fn agent() -> Value {
    let state: AgentState = serde_json::from_value(json!({
        "hello": {"agent": "omaflake-agent 0.1.0", "protocol": {"min": 1, "max": 1}, "osName": "NixOS"},
        "helloAt": "2026-10-20T08:00:18Z",
        "protocolVersion": 1,
        "status": {"load": [0.5, 0.4, 0.3], "memory": {"total": 8000000000_u64, "available": 6000000000_u64}},
        "statusAt": "2026-10-20T08:01:00Z"
    }))
    .unwrap();
    serde_json::to_value(state).unwrap()
}

pub(crate) fn vm_full() -> Vm {
    Vm {
        name: "research".into(),
        recipe: "omaflake.nixos.dev@velvet-tiger-orbit".into(),
        image: "omaflake.nixos.dev@cedar-mango-lift".into(),
        kind: GuestKind::Nixos,
        system: "aarch64-linux".into(),
        status: VmStatus::Running,
        busy: false,
        cpus: 4,
        memory_gb: 8,
        disk_gb: 64,
        network: NetworkMode::Shared,
        share: true,
        share_path: Some("/data/share/research".into()),
        ephemeral: false,
        ssh_port: Some(2201),
        user: "alex".into(),
        audio: true,
        microphone: false,
        clipboard: true,
        camera: false,
        display: DisplayMode::Gpu,
        created: date("2026-10-20T08:00:00Z"),
        identity: Some("amber".into()),
        omaflake_id: "vm:research:amber-lake-quiet-otter-lamp-river-stone-cloud-maple-ocean-tiger-velvet".into(),
        label: "research".into(),
        address: None,
        agent: Some(agent()),
        accelerator: None,
        location: Some("/Volumes/Disk/research.omaflake-vm".into()),
        latest_image: Some("omaflake.nixos.dev@mint-river-cloud".into()),
        upgradable: true,
    }
}

/// A VM with every optional field left out
pub(crate) fn vm_mac() -> Vm {
    Vm {
        name: "work".into(),
        recipe: "omaflake.macos.dev@ocean-maple".into(),
        image: "omaflake.macos.dev@stone-lamp".into(),
        kind: GuestKind::Macos,
        system: "aarch64-darwin".into(),
        status: VmStatus::Stopped,
        busy: true,
        cpus: 6,
        memory_gb: 16,
        disk_gb: 100,
        network: NetworkMode::Isolated,
        share: false,
        share_path: None,
        ephemeral: true,
        ssh_port: None,
        user: "alex".into(),
        audio: false,
        microphone: true,
        clipboard: false,
        camera: true,
        display: DisplayMode::Window,
        created: date("2026-10-21T12:30:00Z"),
        identity: None,
        omaflake_id: "vm:work:?".into(),
        label: "work".into(),
        address: None,
        agent: None,
        accelerator: None,
        location: None,
        latest_image: None,
        upgradable: false,
    }
}

pub(crate) fn image(size: bool) -> ImageDto {
    ImageDto {
        name: "omaflake.nixos.dev@cedar-mango-lift".into(),
        recipe: "omaflake.nixos.dev@velvet-tiger-orbit".into(),
        source: Some("github:omaflake/recipes@main".into()),
        kind: GuestKind::Nixos,
        system: "aarch64-linux".into(),
        revision: "abc123-dirty".into(),
        store_path: Some("/nix/store/abc-nixos-disk-image".into()),
        created: date("2026-10-21T10:00:00Z"),
        line: "omaflake.nixos.dev@github:omaflake/recipes@main".into(),
        path: "/data/images/omaflake.nixos.dev@cedar-mango-lift".into(),
        size_bytes: size.then_some(12_345_678_901),
        label: "image:omaflake/nixos/dev:cedar".into(),
        omaflake_id: "image:omaflake/nixos/dev:cedar-mango-lift-amber-lake-quiet-otter-lamp-river-stone-cloud-maple"
            .into(),
        instances: vec!["research".into()],
        current: true,
        outdated: Some(false),
    }
}

/// An image with every optional field left out
pub(crate) fn image_old() -> ImageDto {
    ImageDto {
        name: "omaflake.macos.dev@stone-lamp".into(),
        recipe: "omaflake.macos.dev@ocean-maple".into(),
        source: None,
        kind: GuestKind::Macos,
        system: "aarch64-darwin".into(),
        revision: "nogit".into(),
        store_path: None,
        created: date("2026-10-19T07:00:00Z"),
        line: "omaflake.macos.dev@".into(),
        path: "/data/images/omaflake.macos.dev@stone-lamp".into(),
        size_bytes: None,
        label: "image:omaflake/macos/dev:stone".into(),
        omaflake_id: "image:omaflake/macos/dev:stone-lamp".into(),
        instances: vec![],
        current: false,
        outdated: None,
    }
}

pub(crate) fn version(id: &str, newest: bool, live: bool, images: &[&str]) -> VersionDto {
    VersionDto {
        id: id.into(),
        source: "github:omaflake/recipes@main".into(),
        added: (!live).then(|| date("2026-10-20T08:00:00Z")),
        newest,
        live,
        images: images.iter().map(|s| s.to_string()).collect(),
        label: format!("recipe:omaflake/nixos/dev:{}", id.split('@').nth(1).unwrap_or("?").split('-').next().unwrap()),
    }
}

pub(crate) fn recipe_list() -> RecipeList {
    let versions = vec![
        version("omaflake.nixos.dev@velvet-tiger-orbit", true, false, &["omaflake.nixos.dev@cedar-mango-lift"]),
        version("omaflake.nixos.dev@amber-oak-lamp", false, false, &[]),
    ];
    let full = RecipeDto {
        id: "omaflake.nixos.dev@velvet-tiger-orbit".into(),
        base: "omaflake.nixos.dev".into(),
        qualified: "omaflake/nixos/dev".into(),
        display_name: "dev".into(),
        label: "recipe:omaflake/nixos/dev:velvet".into(),
        kind: GuestKind::Nixos,
        systems: vec!["aarch64-linux".into()],
        runs_here: true,
        cpus: 4,
        memory_gb: 8,
        disk_gb: 64,
        network: NetworkMode::Shared,
        share: true,
        ephemeral: false,
        user: Some("alex".into()),
        name: Some("dev".into()),
        author: Some("omaflake".into()),
        audio: Some(true),
        microphone: Some(false),
        clipboard: Some(true),
        camera: Some(false),
        description: Some("Entwicklungsumgebung".into()),
        lab: false,
        display: Some(DisplayMode::Gpu),
        tags: vec!["dev".into(), "gui".into()],
        license: Some("MIT".into()),
        homepage: Some("https://omaflake.dev".into()),
        source: Some("github:omaflake/recipes@main".into()),
        line: "omaflake.nixos.dev@github:omaflake/recipes@main".into(),
        distinction: Some("velvet".into()),
        folder: Some("/data/recipes/omaflake.nixos.dev@velvet-tiger-orbit".into()),
        installed: Some(InstalledDto {
            source: "github:omaflake/recipes@main".into(),
            version: "3".into(),
            installed: date("2026-10-20T08:00:00Z"),
        }),
        image: Some("omaflake.nixos.dev@cedar-mango-lift".into()),
        outdated: true,
        versions: versions.clone(),
    };
    // Only what is always there
    let lab = RecipeDto {
        id: "lab.macos.base@ocean-maple".into(),
        base: "lab.macos.base".into(),
        qualified: "lab/macos/base".into(),
        display_name: "base".into(),
        label: "recipe:lab/macos/base:ocean".into(),
        kind: GuestKind::Macos,
        systems: vec!["aarch64-darwin".into()],
        runs_here: false,
        cpus: 4,
        memory_gb: 8,
        disk_gb: 80,
        network: NetworkMode::None,
        share: false,
        ephemeral: true,
        user: None,
        name: None,
        author: None,
        audio: None,
        microphone: None,
        clipboard: None,
        camera: None,
        description: None,
        lab: true,
        display: None,
        tags: vec![],
        license: None,
        homepage: None,
        source: None,
        line: "lab.macos.base@".into(),
        distinction: None,
        folder: None,
        installed: None,
        image: None,
        outdated: false,
        versions: vec![],
    };
    RecipeList {
        recipes: vec![lab, full],
        lines: BTreeMap::from([("omaflake.nixos.dev@github:omaflake/recipes@main".to_string(), versions)]),
        id_lengths: IdLengths { recipe: 1, image: 1, vm: 1 },
        qemu_gl: true,
        unreadable: vec![Unreadable { name: "acme.nixos.x".into(), reason: "recipe.toml: unknown key".into() }],
        lab_hidden: false,
    }
}

pub(crate) fn catalog(id: &str, lab: bool) -> CatalogRecipeDto {
    let mut parts = id.split('.');
    CatalogRecipeDto {
        id: id.into(),
        author: parts.next().unwrap().into(),
        kind: if parts.next() == Some("macos") { GuestKind::Macos } else { GuestKind::Nixos },
        name: parts.next().unwrap().into(),
        description: "A small NixOS".into(),
        tags: vec!["minimal".into()],
        license: (!lab).then(|| "MIT".into()),
        lab,
        systems: vec!["aarch64-linux".into()],
        requires: if lab { vec![] } else { vec!["omaflake.nixos.base".into()] },
        homepage: None,
        version: "2".into(),
        files: vec![
            CatalogFileDto { path: "recipe.toml".into(), size: 412, sha256: "9f86d081884c7d65".into() },
            CatalogFileDto { path: "configuration.nix".into(), size: 2048, sha256: "60303ae22b998861".into() },
        ],
        source: "github:omaflake/recipes@main".into(),
        runs_here: true,
    }
}

pub(crate) fn available() -> Available {
    Available {
        offered: vec![catalog("omaflake.nixos.minimal", false), catalog("lab.nixos.try", true)],
        updates: vec![RecipeUpdate {
            id: "omaflake.nixos.dev@velvet-tiger-orbit".into(),
            recipe: catalog("omaflake.nixos.dev", false),
        }],
        suggested: vec![catalog("omaflake.nixos.minimal", false)],
        errors: BTreeMap::from([("url:https://example.org/catalog.json".into(), "HTTP 404".into())]),
    }
}

pub(crate) fn source_official() -> SourceDto {
    SourceDto {
        id: "github:omaflake/recipes@main".into(),
        kind: RecipeSourceKind::Github,
        location: "omaflake/recipes".into(),
        branch: Some("main".into()),
        display_name: "omaflake/recipes".into(),
        enabled: true,
        private: false,
        official: true,
        has_token: false,
    }
}

pub(crate) fn source_private() -> SourceDto {
    SourceDto {
        id: "url:https://example.org/catalog.json".into(),
        kind: RecipeSourceKind::Url,
        location: "https://example.org/catalog.json".into(),
        branch: None,
        display_name: "example.org".into(),
        enabled: false,
        private: true,
        official: false,
        has_token: true,
    }
}

pub(crate) fn prerequisite(item: PrerequisiteItem, status: PrerequisiteStatus) -> PrerequisiteDto {
    PrerequisiteDto {
        item,
        status,
        title: super::model::requirement_title_on(item, super::views::HostOs::Macos).into(),
        required: !matches!(item, PrerequisiteItem::Git | PrerequisiteItem::Cli),
        detail: "/opt/homebrew/bin/qemu-system-aarch64".into(),
        hint: "Install QEMU".into(),
        install: None,
        settable_path: matches!(item, PrerequisiteItem::Nix | PrerequisiteItem::Qemu | PrerequisiteItem::Flake),
        free_bytes: None,
        needed_bytes: None,
        base_user: None,
        expected_user: None,
    }
}

pub(crate) fn doctor() -> DoctorReport {
    use PrerequisiteItem as I;
    use PrerequisiteStatus as S;
    let qemu = prerequisite(I::Qemu, S::Missing);
    let nix = PrerequisiteDto {
        install: Some("curl -sSfL https://install.determinate.systems/nix | sh -s -- install".into()),
        ..prerequisite(I::Nix, S::Ok)
    };
    let builder = prerequisite(I::BuilderRunning, S::Missing);
    let space = PrerequisiteDto {
        free_bytes: Some(20_000_000_000),
        needed_bytes: Some(40_000_000_000),
        ..prerequisite(I::MacSpace, S::Limited)
    };
    let base = PrerequisiteDto {
        base_user: Some("alex".into()),
        expected_user: Some("sam".into()),
        ..prerequisite(I::MacBase, S::Limited)
    };
    let cli = PrerequisiteDto {
        detail: "not on your PATH".into(),
        hint: "Optional: install the omaf command to use omaflake in a terminal".into(),
        install: Some("/Applications/omaflake.app/Contents/MacOS/omaf doctor --install-cli".into()),
        ..prerequisite(I::Cli, S::Missing)
    };
    DoctorReport {
        prerequisites: vec![nix, qemu.clone(), prerequisite(I::Git, S::Missing), cli],
        nixos_preflight: vec![builder.clone()],
        macos_preflight: vec![prerequisite(I::Ipsw, S::Ok), base.clone(), space],
        latest_release: Some(MacOsReleaseDto {
            version: "27.0.1".into(),
            build: "27A100".into(),
            url: "https://updates.example.org/x.ipsw".into(),
            bytes: Some(18_000_000_000),
        }),
        build_blockers: PerKind { nixos: vec![builder], macos: vec![base] },
        start_blockers: PerKind { nixos: vec![qemu], macos: vec![] },
        issues: 3,
        requirements_missing: true,
        has_mac_base: true,
        mac_base_user: Some("alex".into()),
        builder_system_is_omaflakes: false,
        cli: Some(CliLinkDto {
            link: "/usr/local/bin/omaf".into(),
            target: Some("/Applications/omaflake.app/Contents/MacOS/omaf".into()),
            installed: false,
            blocked: false,
            on_path: true,
            admin: true,
        }),
        checked: date("2026-10-23T09:41:00Z"),
    }
}

pub(crate) fn job(id: &str, kind: JobKind, state: JobState, result: Option<Value>) -> JobDto {
    let ended = !state.is_active();
    JobDto {
        id: id.into(),
        kind,
        title: format!("{kind:?}"),
        recipe: None,
        guest_kind: None,
        state,
        progress: None,
        started: (state != JobState::Queued).then(|| date("2026-10-23T09:00:00Z")),
        finished: ended.then(|| date("2026-10-23T09:05:00Z")),
        last_output: (state != JobState::Queued).then(|| date("2026-10-23T09:04:59Z")),
        log_lines: 3,
        hidden_lines: 0,
        log_file: Some(format!("/data/logs/{id}.log")),
        cancellable: !ended,
        error: (state == JobState::Failed).then(|| "it went wrong".into()),
        result,
    }
}

pub(crate) fn jobs() -> Vec<JobDto> {
    fn r<T: Serialize>(t: T) -> Option<Value> {
        Some(serde_json::to_value(t).unwrap())
    }
    vec![
        JobDto {
            recipe: Some("omaflake.nixos.dev@velvet-tiger-orbit".into()),
            guest_kind: Some(GuestKind::Nixos),
            progress: Some(JobProgress {
                step: 3,
                total: 5,
                title: "Copying the image".into(),
                done: 120,
                of: 400,
                fraction: 0.62,
            }),
            hidden_lines: 37,
            ..job("j1", JobKind::Build, JobState::Running, None)
        },
        JobDto {
            recipe: Some("omaflake.macos.dev@ocean-maple".into()),
            guest_kind: Some(GuestKind::Macos),
            log_lines: 0,
            ..job("j2", JobKind::Build, JobState::Queued, None)
        },
        job(
            "j3",
            JobKind::Build,
            JobState::Finished,
            r(BuildResult {
                image: "omaflake.nixos.dev@cedar-mango-lift".into(),
                removed: vec!["omaflake.nixos.dev@old-image".into()],
            }),
        ),
        JobDto {
            guest_kind: Some(GuestKind::Macos),
            progress: Some(JobProgress {
                step: 0,
                total: 0,
                title: "Downloading macOS 27.0.1".into(),
                done: 1_000_000_000,
                of: 18_000_000_000,
                fraction: 0.055,
            }),
            ..job("j4", JobKind::IpswDownload, JobState::Running, None)
        },
        job(
            "j5",
            JobKind::IpswDownload,
            JobState::Finished,
            r(IpswResult { file: "/data/ipsw/27A100.ipsw".into(), version: "27.0.1".into() }),
        ),
        job(
            "j6",
            JobKind::MacosInstall,
            JobState::Finished,
            r(BaseResult { base: "/data/bases/mac-base-27A100".into(), existing: false, ready: true }),
        ),
        job(
            "j7",
            JobKind::Import,
            JobState::Finished,
            r(ImportResult {
                kind: PackageKindDto::Recipe,
                names: vec!["acme.nixos.x@amber".into()],
                notes: vec!["also installed acme.nixos.base".into()],
            }),
        ),
        job(
            "j8",
            JobKind::Export,
            JobState::Finished,
            r(ExportResult { path: "/Volumes/Disk/research.omaflake-vm".into() }),
        ),
        job(
            "j9",
            JobKind::DataMove,
            JobState::Finished,
            r(DataMoveResult { data_folder: "/Volumes/Disk/omaflake".into() }),
        ),
        JobDto {
            recipe: Some("omaflake.nixos.minimal".into()),
            ..job(
                "j10",
                JobKind::RecipeInstall,
                JobState::Finished,
                r(InstallResult { installed: vec!["omaflake.nixos.minimal@lamp-river".into()] }),
            )
        },
        job("j11", JobKind::BuilderInstallNix, JobState::Failed, None),
        job("j12", JobKind::BuilderSetup, JobState::Cancelled, None),
        job("j13", JobKind::BuilderApply, JobState::Checking, None),
        job(
            "j14",
            JobKind::BuilderCleanUp,
            JobState::Finished,
            r(MessageResult { message: "Linux builder: 18G → 28G free".into() }),
        ),
    ]
}

/// The page models (docs/api.md §10), in `testdata/api/views/`: every page from the views' test context
/// (`views::tests::ctx`, built from the fixtures above), the `view.changed` event and what a client sends
fn view_goldens() -> Vec<(&'static str, Value)> {
    use super::views::{build, import, tests::ctx, PageRef};
    let c = ctx();
    let page = |p: &str, params: Value| build(&c, &PageRef { page: p.into(), params }).unwrap();
    let mut failed = ctx();
    failed.jobs[0].state = JobState::Failed;
    failed.jobs[0].error = Some("nix build failed".into());
    let mut mac = ctx();
    mac.recipes.recipes[0].runs_here = true;
    let mut linux = ctx();
    linux.host = super::views::HostOs::Linux;
    let mut no_download = ctx();
    no_download.jobs.retain(|j| j.id != "j4");
    let mut loading = ctx();
    loading.fetching = true;
    loading.recipes.lab_hidden = true;
    let inspection = Inspection {
        kind: PackageKindDto::Vm,
        valid: true,
        reason: None,
        conflict: Some(ConflictDto {
            vm_name: Some("research".into()),
            suggested_name: Some("research-2".into()),
            image: None,
        }),
        network_volume: false,
    };
    let import_params =
        import::ImportParams { path: "/Volumes/Disk/research.omaflake-vm".into(), expect: None, external: false };
    vec![
        ("home", page("home", json!({}))),
        ("vms", page("vms", json!({}))),
        ("vms.filtered", page("vms", json!({"filter": {"status": false, "kind": "nixos"}}))),
        ("vm", page("vm", json!({"name": "research"}))),
        ("images", page("images", json!({}))),
        ("image", page("image", json!({"name": "omaflake.macos.dev@stone-lamp"}))),
        ("recipes", page("recipes", json!({"filter": {"otherSystems": true}}))),
        ("newVm", page("newVm", json!({"image": "omaflake.nixos.dev@cedar-mango-lift"}))),
        ("newVm.macos", page("newVm", json!({"image": "omaflake.macos.dev@stone-lamp", "values": {"name": "Work"}}))),
        ("build", page("build", json!({"recipe": "omaflake.nixos.dev@velvet-tiger-orbit"}))),
        (
            "build.failed",
            build(
                &failed,
                &PageRef { page: "build".into(), params: json!({"recipe": "omaflake.nixos.dev@velvet-tiger-orbit"}) },
            )
            .unwrap(),
        ),
        (
            "build.macos",
            build(&mac, &PageRef { page: "build".into(), params: json!({"recipe": "lab.macos.base@ocean-maple"}) })
                .unwrap(),
        ),
        ("doctor", page("doctor", json!({}))),
        ("macBase", page("macBase", json!({}))),
        (
            "macBase.ipsw",
            build(
                &no_download,
                &PageRef {
                    page: "macBase".into(),
                    params: json!({"values": {"ipsw": "/Volumes/Disk/UniversalMac.ipsw"}}),
                },
            )
            .unwrap(),
        ),
        ("settings", page("settings", json!({"values": {"input": "acme/recipes", "private": true}}))),
        ("activity", page("activity", json!({}))),
        ("home.linux", build(&linux, &PageRef { page: "home".into(), params: json!({}) }).unwrap()),
        ("settings.linux", build(&linux, &PageRef { page: "settings".into(), params: json!({}) }).unwrap()),
        ("recipes.loading", build(&loading, &PageRef { page: "recipes".into(), params: json!({}) }).unwrap()),
        ("import", serde_json::to_value(import::page(&import_params, inspection)).unwrap()),
        (
            "event.view.changed",
            json!({"page": "vms", "params": {"filter": {"status": true}}, "model": page("vms", json!({"filter": {"status": true}}))}),
        ),
    ]
}

/// What a client sends: (file name, the JSON) – parsed here with the server's types
fn params_files() -> Vec<String> {
    vec!["initialize.params".into(), "vms.create.params".into()]
}

fn view_params_files() -> Vec<String> {
    vec!["views.get.params".into(), "views.subscribe.params".into()]
}

/// Compares (or with OMAF_UPDATE_GOLDEN writes) the golden files of `dir`; returns those that differ
fn compare(dir: &std::path::Path, goldens: Vec<(&'static str, Value)>) -> Vec<&'static str> {
    let update = std::env::var_os("OMAF_UPDATE_GOLDEN").is_some();
    std::fs::create_dir_all(dir).unwrap();
    let mut stale = Vec::new();
    for (name, value) in goldens {
        let file = dir.join(format!("{name}.json"));
        let text = serde_json::to_string_pretty(&value).unwrap() + "\n";
        if update {
            std::fs::write(&file, &text).unwrap();
            continue;
        }
        match std::fs::read_to_string(&file) {
            Ok(old) if serde_json::from_str::<Value>(&old).ok() == Some(value) => {}
            _ => stale.push(name),
        }
    }
    stale
}

#[test]
fn view_golden_files_match_the_types() {
    let stale = compare(&dir().join("views"), view_goldens());
    assert!(
        stale.is_empty(),
        "view golden files differ: {stale:?} – OMAF_UPDATE_GOLDEN=1 cargo test -p omaflake-core api::contract"
    );
}

#[test]
fn view_params_parse() {
    let read = |name: &str| -> Value {
        serde_json::from_str(&std::fs::read_to_string(dir().join("views").join(format!("{name}.json"))).unwrap())
            .unwrap()
    };
    let get: super::views::PageRef = serde_json::from_value(read("views.get.params")).unwrap();
    assert_eq!(get.page, "newVm");
    #[derive(serde::Deserialize)]
    struct Sub {
        pages: Vec<super::views::PageRef>,
    }
    let sub: Sub = serde_json::from_value(read("views.subscribe.params")).unwrap();
    assert_eq!(sub.pages.len(), 3);
    assert_eq!(sub.pages[0].params, json!({}), "params may be left out");
}

#[test]
fn golden_files_match_the_types() {
    let update = std::env::var_os("OMAF_UPDATE_GOLDEN").is_some();
    let dir = dir();
    std::fs::create_dir_all(&dir).unwrap();
    let mut stale = Vec::new();
    for (name, value) in goldens() {
        let file = dir.join(format!("{name}.json"));
        let text = serde_json::to_string_pretty(&value).unwrap() + "\n";
        if update {
            std::fs::write(&file, &text).unwrap();
            continue;
        }
        match std::fs::read_to_string(&file) {
            Ok(old) if serde_json::from_str::<Value>(&old).ok() == Some(value) => {}
            _ => stale.push(name),
        }
    }
    assert!(
        stale.is_empty(),
        "golden files differ from the API types: {stale:?} – if that is intended, run OMAF_UPDATE_GOLDEN=1 cargo test \
         -p omaflake-core api::contract, then the Swift tests, and update docs/api.md"
    );
}

#[test]
fn every_golden_file_is_known() {
    // A file nobody writes any more would keep the Swift side testing a shape the server no longer sends
    let known: Vec<String> =
        goldens().into_iter().map(|(n, _)| n.to_string()).chain(params_files()).map(|n| format!("{n}.json")).collect();
    for entry in std::fs::read_dir(dir()).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.path().is_dir() && name == "views" {
            continue;
        }
        assert!(known.contains(&name), "testdata/api/{name} is not written by api::contract");
    }
    let known: Vec<String> = view_goldens()
        .into_iter()
        .map(|(n, _)| n.to_string())
        .chain(view_params_files())
        .map(|n| format!("{n}.json"))
        .collect();
    for entry in std::fs::read_dir(dir().join("views")).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        assert!(known.contains(&name), "testdata/api/views/{name} is not written by api::contract");
    }
}

#[test]
fn golden_values_read_back() {
    // What the server sends, it can also read (a Rust UI linking the API uses the same types)
    let check = |name: &str, value: Value| -> Result<(), String> {
        macro_rules! back {
            ($t:ty) => {{
                let parsed: $t = serde_json::from_value(value.clone()).map_err(|e| format!("{name}: {e}"))?;
                if serde_json::to_value(parsed).unwrap() != value {
                    return Err(format!("{name}: does not round-trip"));
                }
            }};
        }
        match name {
            "initialize.result" => back!(InitializeResult),
            "settings.get" => back!(SettingsAnswer),
            "vms.list" | "event.vms.changed" => back!(VmList),
            "vms.get" => back!(Vm),
            "images.list" | "event.images.changed" => back!(ImageList),
            "recipes.list" | "event.recipes.changed" => back!(RecipeList),
            "doctor.check" | "event.doctor.changed" => back!(DoctorReport),
            "jobs.list" => back!(Vec<JobDto>),
            "recipes.available" => back!(Available),
            _ => {}
        }
        Ok(())
    };
    for (name, value) in goldens() {
        check(name, value).unwrap();
    }
}

#[test]
fn client_params_parse() {
    let read = |name: &str| -> Value {
        serde_json::from_str(&std::fs::read_to_string(dir().join(format!("{name}.json"))).unwrap()).unwrap()
    };
    let init: InitializeParams = serde_json::from_value(read("initialize.params")).unwrap();
    assert_eq!(init.api_version, Some(API_VERSION));
    assert_eq!(init.language.as_deref(), Some("de"));
    assert_eq!(init.update_feed.as_deref(), Some("https://example.org/feed.json"));
    let create = read("vms.create.params");
    let settings: RecipeSettings = serde_json::from_value(create["settings"].clone()).unwrap();
    assert_eq!((settings.kind, settings.memory_gb, settings.disk_gb), (GuestKind::Nixos, 8, 64));
    assert_eq!((settings.audio, settings.display), (Some(true), None));
    assert_eq!(create["sharePath"], json!("/Volumes/Projects/research"));
    assert_eq!(create["start"], json!(true));
}
