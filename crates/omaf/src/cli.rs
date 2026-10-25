// SPDX-License-Identifier: AGPL-3.0-or-later
//! The command line of `omaf` (port of the swift-argument-parser declarations in Sources/omaf/*.swift): the same
//! commands, options, flags, defaults and help texts. Names follow swift-argument-parser's (camelCase properties →
//! kebab-case options, value names as the property names); `--x/--no-x` inversions are a pair of flags of which the
//! last one wins.

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "omaf", about = "Clean NixOS and macOS VMs from Nix recipes.", version = omaflake_core::VERSION)]
#[command(subcommand_required = true, arg_required_else_help = true)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    // "flavors": the name before the rename (plan B92)
    #[command(
        about = "Recipes from your sources: list them, install and remove them (spec §11.3).",
        alias = "flavors",
        args_conflicts_with_subcommands = true
    )]
    Recipes(Recipes),
    #[command(
        about = "Where recipes come from: GitHub repositories (also private), HTTPS folders and local folders.",
        args_conflicts_with_subcommands = true
    )]
    Sources(Sources),
    #[command(
        about = "Build a recipe's image (NixOS: nix build <flake>#images.<recipe>; macOS: from a base, spec §6.5)."
    )]
    Build(Build),
    #[command(name = "macos", about = "macOS bases: install from Apple's IPSW.")]
    #[command(subcommand_required = true, arg_required_else_help = true)]
    MacOs {
        #[command(subcommand)]
        command: MacOsCommand,
    },
    #[command(about = "Built images (Vorlagen): list them, remove one.", args_conflicts_with_subcommands = true)]
    Images(Images),
    #[command(about = "Create an instance (NixOS: overlay on the current image; macOS: APFS clone of it).")]
    New(New),
    #[command(about = "Change an instance's sound, microphone, clipboard and camera (from its next start).")]
    Set(Set),
    #[command(about = "Start an instance (without a window by default).")]
    Start(Start),
    #[command(about = "Shut down cleanly; force off after the timeout.")]
    Stop(Stop),
    #[command(about = "Reset: delete the instance's state and start over from its image.")]
    Reset(NameArg),
    #[command(about = "Upgrade to the recipe's current image (the old state is deleted).")]
    Upgrade(NameArg),
    #[command(name = "rm", about = "Delete an instance (its share folder stays).")]
    Remove(NameArg),
    #[command(about = "Instances with status.")]
    List(List),
    #[command(
        about = "Copy a VM, an image or a recipe as a package into a folder (an external disk, a NAS, a local folder).",
        long_about = "Copy a VM, an image or a recipe as a package into a folder (an external disk, a NAS, a local \
                      folder).\n\nA VM goes back to the same person (it carries their user, keys and data); recipes \
                      are what people share.\nPackages are folders the Finder shows as one file – copying one by hand \
                      is just as good."
    )]
    #[command(subcommand_required = true, arg_required_else_help = true)]
    Export {
        #[command(subcommand)]
        command: ExportCommand,
    },
    #[command(
        about = "Import a package (.omaflake-vm, .omaflake-image, .omaflake-recipe) or a folder of recipes – always as \
                 a copy into the data folder.",
        long_about = "Import a package (.omaflake-vm, .omaflake-image, .omaflake-recipe) or a folder of recipes – \
                      always as a copy into the data folder.\n\nA VM comes back as the same VM; when its name is \
                      taken, import it under another --name (a second VM with its\nown SSH port and macOS machine \
                      identity). The same image or recipes that are here already are replaced only\nwith --replace. \
                      The source is never changed. To run a VM from where it is instead: omaf external add."
    )]
    Import(Import),
    #[command(
        about = "VMs that run from where their package is (an external disk, a stick, a NAS, a folder).",
        args_conflicts_with_subcommands = true
    )]
    External(External),
    #[command(name = "ssh", about = "Connect via SSH as the guest user (requires ssh-agent or a key).")]
    Ssh(Ssh),
    #[command(about = "Wait until the instance's SSH server responds.")]
    Wait(Wait),
    #[command(about = "Show what the VM's omaflake agent reports (NixOS guests): hello and status.")]
    Agent(Agent),
    #[command(about = "Show the instance's serial console.")]
    Log(Log),
    #[command(about = "Show/set settings in ~/.omaflake/config.json.")]
    Config(Config),
    #[command(about = "Check what this computer brings and whether NixOS and macOS images can be built (preflight).")]
    Doctor(Doctor),
    #[command(about = "Linux builder: apply omaflake's builder host, clean up its disk (runs sudo).")]
    #[command(subcommand_required = true, arg_required_else_help = true)]
    Builder {
        #[command(subcommand)]
        command: BuilderCommand,
    },
    /// New in the Rust CLI (docs/api.md §1)
    #[command(
        about = "Serve the core API (JSON-RPC 2.0, one object per line) on stdin/stdout, or with --socket as the \
                       core service every interface shares (docs/api.md)."
    )]
    Serve(Serve),
    /// Plan B120
    #[command(about = "The core service (omaf serve --socket): install it as a user agent, start, stop, status.")]
    #[command(subcommand_required = true, arg_required_else_help = true)]
    Service {
        #[command(subcommand)]
        command: ServiceCommand,
    },
    /// New in the Rust CLI (docs/api.md §1)
    #[command(about = "Run a single core API request and print its result as JSON (docs/api.md).")]
    Call(Call),
    /// Plan B121
    #[command(
        about = "The terminal interface: VMs, images, recipes, builds, the Doctor and settings, as in the app.",
        long_about = "The terminal interface: VMs, images, recipes, builds, the Doctor and settings, as in the app.\n\n\
                      A client of the core service (started on demand; a server in this process when it cannot be \
                      reached). Keys: Tab pages, j/k move, Enter the row's default action, 1-9 its actions, a all \
                      of them, Space details, / filter, Esc back, ? help, q quit. Colours: truecolor with \
                      COLORTERM=truecolor, else 256; none with NO_COLOR. Nerd Font symbols with --nerd-font or \
                      OMAF_NERD_FONT=1."
    )]
    Ui(Ui),
    /// Plan B119/B121
    #[command(about = "Running VMs and builds in one line; --waybar: JSON for a waybar custom module.")]
    Status(Status),
    /// Plan B126: what `omaf-runner bridge` is on a Mac, for a Linux host – started by the core next to QEMU
    #[command(
        about = "Agent, clipboard and camera bridge between this Linux host and a VM (started by omaf).",
        hide = true
    )]
    Bridge(Bridge),
}

#[derive(Debug, Args)]
pub struct Bridge {
    #[arg(long, value_name = "agent", help = "Unix socket of the VM's agent port")]
    pub agent: std::path::PathBuf,
    #[arg(long, value_name = "agent-state", help = "File for what the agent reports")]
    pub agent_state: std::path::PathBuf,
    #[arg(long, help = "Carry the clipboard (the VM's switch)")]
    pub clipboard: bool,
    #[arg(long, help = "Carry the camera (the VM's switch)")]
    pub camera: bool,
    #[arg(long, help = "Send a test pattern instead of the camera")]
    pub camera_test_pattern: bool,
}

#[derive(Debug, Args)]
pub struct Ui {
    #[arg(long, help = "Nerd Font symbols (the terminal's font has them; also OMAF_NERD_FONT=1)")]
    pub nerd_font: bool,
    #[arg(long, help = "Mouse: click selects, the wheel moves (the terminal's text selection needs Shift then)")]
    pub mouse: bool,
    #[arg(long, help = "A server in this process instead of the core service (its jobs end with the TUI)")]
    pub in_process: bool,
}

#[derive(Debug, Args)]
pub struct Status {
    #[arg(long, help = "As waybar custom-module JSON: text, tooltip, class, alt, percentage")]
    pub waybar: bool,
    #[arg(long, help = "Keep printing a line whenever it changes (waybar: exec without interval)")]
    pub follow: bool,
}

// MARK: - Recipes & images

#[derive(Debug, Args)]
pub struct Recipes {
    #[command(subcommand)]
    pub command: Option<RecipesCommand>,
    /// `omaf recipes [--all] …` = `omaf recipes list …` (Swift `defaultSubcommand`)
    #[command(flatten)]
    pub list: RecipesList,
}

#[derive(Debug, Subcommand)]
pub enum RecipesCommand {
    #[command(
        about = "The recipes omaflake can build (installed and in local folders) and the state of their images."
    )]
    List(RecipesList),
    #[command(
        about = "Each recipe's versions per line (source and author/kind/name), newest first, and the images built \
                 from them."
    )]
    Versions(RecipesVersions),
    #[command(
        about = "What the remote sources offer (fetches their catalogs), and what is installed or has an update."
    )]
    Available(RecipesAvailable),
    #[command(about = "Download a recipe (and those it requires) from the first source that has it; also updates it.")]
    Install(RecipesInstall),
    #[command(
        about = "Remove a version of a downloaded, imported or kept recipe, or with --all every version of its line.",
        long_about = "Remove a version of a downloaded, imported or kept recipe, or with --all every version of its \
                      line.\n\nA version images are built from stays while they are there; a local folder's recipe is \
                      changed in its folder."
    )]
    Remove(RecipesRemove),
}

#[derive(Debug, Args, Default)]
pub struct RecipesList {
    #[arg(long, help = "Also lab recipes (development; shown by default in developer mode)")]
    pub all: bool,
    #[arg(long, help = "Also recipes for other architectures (they cannot be built here)")]
    pub all_systems: bool,
    #[arg(long, help = "With each recipe's description")]
    pub long: bool,
    #[arg(long, help = "Evaluate each recipe and mark images it has outgrown (a few seconds per recipe)")]
    pub check: bool,
}

#[derive(Debug, Args)]
pub struct RecipesVersions {
    #[arg(value_name = "recipe", help = "Only this recipe (<name>, <kind>/<name>, <author>/<kind>/<name>)")]
    pub recipe: Option<String>,
}

#[derive(Debug, Args)]
pub struct RecipesAvailable {
    #[arg(long, help = "Also lab recipes")]
    pub all: bool,
    #[arg(long, help = "Also recipes for other architectures")]
    pub all_systems: bool,
}

#[derive(Debug, Args)]
pub struct RecipesInstall {
    #[arg(value_name = "recipe", help = "<author>/<kind>/<name>, e.g. omaflake/nixos/neovim-minimal")]
    pub recipe: String,
}

#[derive(Debug, Args)]
pub struct RecipesRemove {
    #[arg(
        value_name = "recipe",
        help = "<author>/<kind>/<name>, or its omaflake ID (omaf recipes versions) when there are several"
    )]
    pub recipe: String,
    #[arg(long, help = "Every version of the recipe's line (source and author/kind/name)")]
    pub all: bool,
}

#[derive(Debug, Args)]
pub struct Sources {
    #[command(subcommand)]
    pub command: Option<SourcesCommand>,
}

#[derive(Debug, Subcommand)]
pub enum SourcesCommand {
    #[command(about = "The sources in their order.")]
    List,
    #[command(
        about = "Add a source: a GitHub repository (https://github.com/owner/repo, owner/repo), the HTTPS URL of a \
                 catalog.json, or a folder."
    )]
    Add(SourcesAdd),
    #[command(about = "Remove a source (its token goes from the Keychain or keyring; installed recipes stay).")]
    Remove(SourceRef),
    #[command(about = "Switch a source on.")]
    Enable(SourceRef),
    #[command(about = "Switch a source off (a local folder's recipes are no longer offered; installed ones stay).")]
    Disable(SourceRef),
    #[command(about = "Check that every source can be read.")]
    Check,
}

#[derive(Debug, Args)]
pub struct SourcesAdd {
    #[arg(value_name = "location", help = "GitHub repository, HTTPS URL or folder")]
    pub location: String,
    #[arg(long, help = "A private GitHub repository: read a fine-grained token (Contents: Read-only) from stdin")]
    pub private: bool,
    #[arg(long, value_name = "branch", help = "GitHub: the branch (default main)")]
    pub branch: Option<String>,
    #[arg(long, help = "Put it first (earlier sources win when two have the same recipe)")]
    pub first: bool,
}

#[derive(Debug, Args)]
pub struct SourceRef {
    #[arg(value_name = "source", help = "Number in omaf sources, or the source's ID")]
    pub source: String,
}

#[derive(Debug, Args)]
pub struct Build {
    #[arg(
        value_name = "recipe",
        help = "Recipe: its name (e.g. dev), or <kind>/<name> when a NixOS and a macOS recipe share it (e.g. \
                macos/minimal)"
    )]
    pub recipe: String,
    #[arg(long, value_name = "base", help = "macOS: base folder (default: newest ~/.omaflake/bases/mac-base-*)")]
    pub base: Option<String>,
    #[arg(long, help = "macOS: read the guest user's password (for sudo, once) from stdin instead of prompting")]
    pub password_stdin: bool,
    #[arg(long, help = "macOS: keep a failed build folder for inspection instead of deleting it")]
    pub keep_failed: bool,
    #[arg(long, help = "Show every line of the build log, also the disk image tool's routine ones")]
    pub verbose: bool,
}

#[derive(Debug, Args)]
pub struct Images {
    #[command(subcommand)]
    pub command: Option<ImagesCommand>,
}

#[derive(Debug, Subcommand)]
pub enum ImagesCommand {
    #[command(about = "Built images and the instances using them.")]
    List,
    #[command(name = "rm", about = "Delete an image (refused while instances use it).")]
    Remove {
        #[arg(
            value_name = "name",
            help = "Image name as listed by omaf images, its omaflake ID, or the start of either"
        )]
        name: String,
    },
}

// MARK: - Instances

/// `--x` / `--no-x` (swift-argument-parser `inversion: .prefixedNo`): None when neither is given, else the last one
/// (the two flags override each other)
pub fn inverted(yes: bool, no: bool) -> Option<bool> {
    if yes {
        Some(true)
    } else if no {
        Some(false)
    } else {
        None
    }
}

#[derive(Debug, Args)]
pub struct New {
    #[arg(value_name = "name", help = "Instance name (a–z, 0–9, -)")]
    pub name: String,
    #[arg(short, long, value_name = "recipe", help = "Recipe: its name, or <kind>/<name> (e.g. macos/minimal)")]
    pub recipe: Option<String>,
    /// --flavor / -f: the name before the rename (plan B92), not shown in the help
    #[arg(short = 'f', long = "flavor", value_name = "flavor", hide = true)]
    pub flavor: Option<String>,
    #[arg(long, overrides_with = "no_ephemeral", help = "Ephemeral instance (changes are discarded on power-off)")]
    pub ephemeral: bool,
    #[arg(long, overrides_with = "ephemeral", help = "Not ephemeral, whatever the recipe says")]
    pub no_ephemeral: bool,
    #[arg(
        long,
        overrides_with = "no_share",
        help = "Share the folder ~/.omaflake/share/<name> on this computer with the guest"
    )]
    pub share: bool,
    #[arg(long, overrides_with = "share", help = "No shared folder, whatever the recipe says")]
    pub no_share: bool,
    #[arg(
        long,
        value_name = "share-dir",
        help = "Share this existing folder on this computer instead (implies --share)"
    )]
    pub share_dir: Option<String>,
    #[arg(long, overrides_with = "no_audio", help = "Sound from the VM to this computer (default on)")]
    pub audio: bool,
    #[arg(long, overrides_with = "audio", help = "No sound from the VM")]
    pub no_audio: bool,
    #[arg(
        long,
        overrides_with = "no_microphone",
        help = "This computer's microphone in the VM (default off; macOS asks for permission)"
    )]
    pub microphone: bool,
    #[arg(long, overrides_with = "microphone", help = "Without this computer's microphone")]
    pub no_microphone: bool,
    #[arg(long, overrides_with = "no_clipboard", help = "Clipboard shared with this computer (default on)")]
    pub clipboard: bool,
    #[arg(long, overrides_with = "clipboard", help = "Clipboard not shared")]
    pub no_clipboard: bool,
    #[arg(
        long,
        overrides_with = "no_camera",
        help = "This computer's camera in the VM, NixOS (default off; on only while an app uses it)"
    )]
    pub camera: bool,
    #[arg(long, overrides_with = "camera", help = "Without this computer's camera")]
    pub no_camera: bool,
    #[arg(long, value_name = "cpus", help = "CPU cores")]
    pub cpus: Option<u32>,
    #[arg(long, value_name = "memory", help = "RAM in GiB")]
    pub memory: Option<u32>,
    #[arg(long, value_name = "disk", help = "Disk size in GiB")]
    pub disk: Option<u32>,
    #[arg(long, value_name = "network", help = "Network: shared | isolated | none")]
    pub network: Option<String>,
    #[arg(
        long,
        value_name = "user",
        help = "Guest user for SSH (default: user from the recipe, otherwise your user here)"
    )]
    pub user: Option<String>,
}

#[derive(Debug, Args)]
pub struct Set {
    #[arg(value_name = "name")]
    pub name: String,
    #[arg(long, overrides_with = "no_audio", help = "Sound from the VM to this computer")]
    pub audio: bool,
    #[arg(long, overrides_with = "audio", help = "No sound from the VM")]
    pub no_audio: bool,
    #[arg(
        long,
        overrides_with = "no_microphone",
        help = "This computer's microphone in the VM (macOS asks for permission the first time)"
    )]
    pub microphone: bool,
    #[arg(long, overrides_with = "microphone", help = "Without this computer's microphone")]
    pub no_microphone: bool,
    #[arg(long, overrides_with = "no_clipboard", help = "Clipboard shared with this computer")]
    pub clipboard: bool,
    #[arg(long, overrides_with = "clipboard", help = "Clipboard not shared")]
    pub no_clipboard: bool,
    #[arg(
        long,
        overrides_with = "no_camera",
        help = "This computer's camera in the VM (NixOS; macOS asks for permission the first time)"
    )]
    pub camera: bool,
    #[arg(long, overrides_with = "camera", help = "Without this computer's camera")]
    pub no_camera: bool,
}

#[derive(Debug, Args)]
pub struct Start {
    #[arg(value_name = "name")]
    pub name: String,
    #[arg(long, help = "Open a window (NixOS: with 3D if the QEMU binary supports virtio-gpu-gl)")]
    pub window: bool,
    #[arg(long, help = "Without a window, whatever the recipe's display says")]
    pub headless: bool,
    #[arg(long, help = "Window without 3D (virtio-gpu-pci)")]
    pub no_gl: bool,
    #[arg(
        long,
        value_name = "gl",
        default_value = "on",
        help = "OpenGL path: on (native OpenGL, default) | es (ANGLE, only GL 3.0 in the guest)"
    )]
    pub gl: String,
    #[arg(long, help = "Wait until SSH is ready")]
    pub wait: bool,
}

#[derive(Debug, Args)]
pub struct Stop {
    #[arg(value_name = "name")]
    pub name: String,
    #[arg(long, value_name = "timeout", default_value_t = 60.0, help = "Seconds until forced power-off")]
    pub timeout: f64,
}

#[derive(Debug, Args)]
pub struct NameArg {
    #[arg(value_name = "name")]
    pub name: String,
}

#[derive(Debug, Args)]
pub struct List {
    #[arg(long, help = "With each VM's omaflake ID and the recipe it comes from")]
    pub long: bool,
}

#[derive(Debug, Args)]
pub struct Ssh {
    #[arg(value_name = "name")]
    pub name: String,
    #[arg(
        value_name = "command",
        trailing_var_arg = true,
        allow_hyphen_values = true,
        help = "Command to run in the guest (optional)"
    )]
    pub command: Vec<String>,
}

/// Plan B122
#[derive(Debug, Args)]
pub struct Doctor {
    #[arg(
        long,
        conflicts_with = "uninstall_cli",
        help = "Install the omaf command: a link to this omaf in /usr/local/bin (macOS, runs sudo) or ~/.local/bin (Linux)"
    )]
    pub install_cli: bool,
    #[arg(long, help = "Remove omaflake's link of the omaf command again")]
    pub uninstall_cli: bool,
}

#[derive(Debug, Args)]
pub struct Wait {
    #[arg(value_name = "name")]
    pub name: String,
    #[arg(long, value_name = "timeout", default_value_t = 300.0, help = "Seconds")]
    pub timeout: f64,
}

#[derive(Debug, Args)]
pub struct Agent {
    #[arg(value_name = "name")]
    pub name: String,
    #[arg(
        long,
        value_name = "wait",
        default_value_t = 0.0,
        help = "Wait up to this many seconds for the agent's hello"
    )]
    pub wait: f64,
    #[arg(long, help = "Print the raw state (JSON)")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct Log {
    #[arg(value_name = "name")]
    pub name: String,
    #[arg(short = 'n', value_name = "n", default_value_t = 40, help = "Number of lines")]
    pub lines: u32,
    #[arg(short = 'f', help = "Keep following (tail -f)")]
    pub follow: bool,
    #[arg(long, help = "QEMU log instead of the serial console")]
    pub qemu: bool,
}

#[derive(Debug, Args)]
pub struct Config {
    #[arg(
        long,
        value_name = "home",
        help = "Move the data folder (images, instances, …) there – \"default\" = ~/.omaflake; VMs must be stopped"
    )]
    pub home: Option<String>,
    #[arg(long, value_name = "flake", help = "The image flake: a folder (e.g. <checkout>/nix) or a flake URL")]
    pub flake: Option<String>,
    #[arg(
        long,
        value_name = "qemu",
        help = "Path to qemu-system-aarch64 on a Mac (e.g. a custom build from qemu/build.sh), qemu-system-x86_64 or -aarch64 on Linux"
    )]
    pub qemu: Option<String>,
    #[arg(
        long,
        value_name = "runner",
        help = "Path to omaf-runner signed by scripts/sign-runner.sh (default: next to omaf)"
    )]
    pub runner: Option<String>,
    #[arg(long, value_name = "nix", help = "Path to the nix binary (default: PATH and the usual Nix locations)")]
    pub nix: Option<String>,
    #[arg(
        long,
        value_name = "guest-user",
        help = "Guest user of NixOS VMs (default: your user here; macOS VMs use the base's user)"
    )]
    pub guest_user: Option<String>,
    #[arg(long, value_name = "guest-full-name", help = "Full name of the guest user")]
    pub guest_full_name: Option<String>,
    #[arg(
        long,
        value_name = "guest-locale",
        help = "Locale in the guests, e.g. de_DE.UTF-8 (default: this computer's)"
    )]
    pub guest_locale: Option<String>,
    #[arg(
        long,
        value_name = "guest-keyboard",
        help = "Keyboard layout in the guests, e.g. de or us (default: this computer's)"
    )]
    pub guest_keyboard: Option<String>,
    #[arg(
        long,
        value_name = "guest-time-zone",
        help = "Time zone in the guests, e.g. Europe/Berlin (default: this computer's)"
    )]
    pub guest_time_zone: Option<String>,
    #[arg(
        long,
        value_name = "guest-look",
        help = "Look of the guests: omaflake, catppuccin, rose-pine-dawn or vanilla; empty: each recipe's own (default)"
    )]
    pub guest_look: Option<String>,
    #[arg(
        long,
        value_name = "keep-recipe-versions",
        help = "Keep older versions of a recipe when a newer one comes: on or off (default off; versions images use \
                stay)"
    )]
    pub keep_recipe_versions: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum BuilderCommand {
    #[command(about = "Set up nix-darwin with the Linux builder on a Mac without it (two stages, sudo).")]
    Setup,
    #[command(about = "darwin-rebuild switch with the image flake's #linux-builder host.")]
    Apply {
        #[arg(long, help = "Also when this Mac's nix-darwin system is not omaflake's (it gets replaced)")]
        force: bool,
    },
    #[command(name = "clean-up", about = "Remove unused store paths in the builder (free space before and after).")]
    CleanUp,
}

// MARK: - Packages

#[derive(Debug, Subcommand)]
pub enum ExportCommand {
    #[command(name = "vm", about = "A stopped VM → <folder>/<name>.omaflake-vm (NixOS: with its image).")]
    Vm {
        #[arg(value_name = "name")]
        name: String,
        #[arg(value_name = "folder", help = "Target folder")]
        folder: String,
        #[arg(long, help = "macOS: also carry the image (needed to reset the VM where no image is)")]
        with_image: bool,
    },
    #[command(about = "An image → <folder>/<name>.omaflake-image.")]
    Image {
        #[arg(value_name = "name", help = "Image name as listed by omaf images")]
        name: String,
        #[arg(value_name = "folder", help = "Target folder")]
        folder: String,
    },
    #[command(about = "A recipe and those it requires → <folder>/<id>.omaflake-recipe.")]
    Recipe {
        #[arg(value_name = "recipe", help = "Recipe: its name, <kind>/<name> or <author>/<kind>/<name>")]
        recipe: String,
        #[arg(value_name = "folder", help = "Target folder")]
        folder: String,
    },
}

#[derive(Debug, Args)]
pub struct Import {
    #[arg(value_name = "path", help = "Package or recipe folder")]
    pub path: String,
    #[arg(long, value_name = "name", help = "VM: import it under this name (a second VM with its own identity)")]
    pub name: Option<String>,
    #[arg(long, help = "Image or recipes: replace what is here already")]
    pub replace: bool,
}

#[derive(Debug, Args)]
pub struct External {
    #[command(subcommand)]
    pub command: Option<ExternalCommand>,
}

#[derive(Debug, Subcommand)]
pub enum ExternalCommand {
    #[command(about = "The external VMs and where they are.")]
    List,
    #[command(about = "Run a VM package from where it is – nothing is copied (export it with omaf export vm first).")]
    Add {
        #[arg(value_name = "path", help = "The .omaflake-vm package")]
        path: String,
    },
    #[command(about = "Stop listing an external VM (also while its place is not there); its package stays.")]
    Remove {
        #[arg(value_name = "name")]
        name: String,
    },
}

// MARK: - macOS

#[derive(Debug, Subcommand)]
pub enum MacOsCommand {
    #[command(
        about = "Download macOS from Apple and build a ready base ~/.omaflake/bases/mac-base-<build> – no manual steps.",
        long_about = "Download macOS from Apple and build a ready base ~/.omaflake/bases/mac-base-<build> – no manual \
                      steps.\n\nSteps: omaf-runner install (download of the latest supported IPSW unless --ipsw) → \
                      mac-fresh-<build> (never started, read-only) → APFS clone mac-base-<build> with a new identity → \
                      first boot (headless) with provisioning: user, automatic login and SSH, without the Setup \
                      Assistant → your SSH public keys (~/.ssh/*.pub) into the guest → shut down → read-only. Existing \
                      folders are reused, never overwritten."
    )]
    Install(MacInstall),
}

#[derive(Debug, Args)]
pub struct MacInstall {
    #[arg(long, value_name = "ipsw", help = "Local IPSW file (default: the latest supported by this Mac, from Apple)")]
    pub ipsw: Option<String>,
    #[arg(long, value_name = "cpus", default_value_t = 4, help = "CPU cores")]
    pub cpus: u32,
    #[arg(long, value_name = "memory", default_value_t = 8, help = "RAM in GiB")]
    pub memory: u32,
    #[arg(long, value_name = "disk", default_value_t = 80, help = "Disk size in GiB (thin-provisioned)")]
    pub disk: u32,
    #[arg(long, value_name = "user", help = "Guest user to create (default: your Mac user name)")]
    pub user: Option<String>,
    #[arg(long, value_name = "full-name", help = "Full name of the guest user (default: yours)")]
    pub full_name: Option<String>,
    #[arg(long, help = "Read the guest user's password from stdin instead of prompting")]
    pub password_stdin: bool,
    #[arg(long, help = "Stop after the clone, without the first boot")]
    pub no_first_boot: bool,
    #[arg(long, help = "Show the first boot in a window")]
    pub window: bool,
}

// MARK: - API

#[derive(Debug, Args)]
pub struct Call {
    #[arg(value_name = "method", help = "The method, e.g. vm.list (docs/api.md)")]
    pub method: String,
    #[arg(value_name = "params", help = "Its params as one JSON value, e.g. '{\"name\":\"dev\"}' (default: none)")]
    pub params: Option<String>,
    #[arg(
        long,
        value_name = "path",
        num_args = 0..=1,
        require_equals = true,
        help = "Through the core service (started when it does not run), on its socket or this one \
                (--socket=<path>); without: in this process, without a session"
    )]
    pub socket: Option<Option<std::path::PathBuf>>,
}

#[derive(Debug, Args)]
pub struct Serve {
    #[arg(
        long,
        value_name = "path",
        num_args = 0..=1,
        require_equals = true,
        help = "Serve as the core service on a Unix socket (default: <data folder>/run/omaf.sock) instead of \
                stdin/stdout; a path is given as --socket=<path>"
    )]
    pub socket: Option<Option<std::path::PathBuf>>,
    #[arg(
        long,
        value_name = "secs",
        requires = "socket",
        help = "With --socket: end when no client has been connected and no job has run for this long"
    )]
    pub idle_exit: Option<u64>,
}

#[derive(Debug, Subcommand)]
pub enum ServiceCommand {
    #[command(about = "Install the service as a user agent (macOS: launchd; Linux: systemd --user) and start it.")]
    Install,
    #[command(about = "Stop the user agent and remove it.")]
    Uninstall,
    #[command(about = "Is the service installed and running; its socket, version and jobs.")]
    Status {
        #[arg(long, help = "As JSON")]
        json: bool,
    },
    #[command(about = "Start the service unless it runs (the user agent if installed, else on its own until idle).")]
    Start {
        #[arg(long, help = "Print {socket, version, started} as JSON")]
        json: bool,
    },
    #[command(about = "Stop the service once its jobs are done (service.restart).")]
    Stop,
}
