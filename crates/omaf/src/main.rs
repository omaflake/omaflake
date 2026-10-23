// SPDX-License-Identifier: AGPL-3.0-or-later
//! `omaf` – build, create, start and reset recipe VMs (spec §6.2); with `omaf serve` the JSON-RPC interface of the core
//! (docs/api.md). Port of Sources/omaf (swift-argument-parser) on clap: the same commands, options, output and exit
//! codes – 0 on success, 1 for an error ("Error: …" on stderr), 64 for a usage error, a command's own code where Swift
//! threw `ExitCode` (doctor, sources check, sudo).

mod cli;
mod commands;
mod tty;
mod util;

use clap::error::ErrorKind;
use clap::{CommandFactory, Parser};

use crate::cli::Cli;
use crate::util::{Failure, EX_USAGE};

fn main() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => match e.kind() {
            ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => e.exit(),
            // A group without its subcommand shows its help, like swift-argument-parser (exit 0)
            ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
                print!("{}", e.render());
                std::process::exit(0)
            }
            _ => {
                let _ = e.print();
                std::process::exit(EX_USAGE)
            }
        },
    };
    if !matches!(cli.command, cli::Command::Serve(_)) {
        // Output into a closed pipe (`omaf list | head`) ends the process quietly, as it did in Swift; the server
        // keeps Rust's default and sees the error instead
        // SAFETY: setting a signal's disposition to the default
        unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    }
    let code = match commands::run(cli.command) {
        Ok(()) => 0,
        Err(Failure::Error(e)) => {
            eprintln!("Error: {e}");
            1
        }
        Err(Failure::Usage { message, command }) => {
            eprintln!("Error: {message}");
            eprintln!("{}", usage_of(&command));
            eprintln!("  See 'omaf {} --help' for more information.", command.join(" "));
            EX_USAGE
        }
        Err(Failure::Exit(code)) => code,
    };
    std::process::exit(code);
}

/// "Usage: omaf <command…> …" of a subcommand
fn usage_of(path: &[&str]) -> String {
    fn find(cmd: &mut clap::Command, path: &[&str]) -> String {
        match path.split_first() {
            Some((name, rest)) if cmd.find_subcommand(name).is_some() => {
                find(cmd.find_subcommand_mut(name).expect("found above"), rest)
            }
            _ => cmd.render_usage().to_string(),
        }
    }
    let mut cmd = Cli::command();
    cmd.build();
    find(&mut cmd, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("omaf").chain(args.iter().copied()))
    }

    #[test]
    fn command_line_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn default_subcommands_take_their_flags() {
        let Cli { command: cli::Command::Recipes(r) } = parse(&["recipes", "--all", "--long"]).unwrap() else {
            panic!()
        };
        assert!(r.command.is_none() && r.list.all && r.list.long && !r.list.check);
        let Cli { command: cli::Command::Recipes(r) } = parse(&["flavors", "versions", "dev"]).unwrap() else {
            panic!()
        };
        assert!(matches!(r.command, Some(cli::RecipesCommand::Versions(v)) if v.recipe.as_deref() == Some("dev")));
        assert!(matches!(parse(&["sources"]).unwrap().command, cli::Command::Sources(cli::Sources { command: None })));
        assert!(matches!(parse(&["images", "rm", "x"]).unwrap().command, cli::Command::Images(_)));
        assert!(matches!(parse(&["external"]).unwrap().command, cli::Command::External(_)));
    }

    #[test]
    fn inversions_and_hidden_flavor() {
        let Cli { command: cli::Command::New(n) } =
            parse(&["new", "a", "-f", "dev", "--no-share", "--ephemeral", "--no-ephemeral", "--cpus", "2"]).unwrap()
        else {
            panic!()
        };
        assert_eq!(n.flavor.as_deref(), Some("dev"));
        assert_eq!(n.recipe, None);
        assert_eq!(cli::inverted(n.share, n.no_share), Some(false));
        assert_eq!(cli::inverted(n.ephemeral, n.no_ephemeral), Some(false));
        assert_eq!(cli::inverted(n.audio, n.no_audio), None);
        assert_eq!(n.cpus, Some(2));
        let Cli { command: cli::Command::Set(s) } = parse(&["set", "a", "--no-audio", "--audio"]).unwrap() else {
            panic!()
        };
        assert_eq!(cli::inverted(s.audio, s.no_audio), Some(true));
    }

    #[test]
    fn defaults_and_short_options() {
        let Cli { command: cli::Command::Stop(s) } = parse(&["stop", "a"]).unwrap() else { panic!() };
        assert_eq!(s.timeout, 60.0);
        let Cli { command: cli::Command::Start(s) } = parse(&["start", "a", "--window", "--gl", "es"]).unwrap() else {
            panic!()
        };
        assert!(s.window && s.gl == "es" && !s.wait);
        let Cli { command: cli::Command::Log(l) } = parse(&["log", "a", "-n", "5", "-f"]).unwrap() else { panic!() };
        assert!(l.lines == 5 && l.follow && !l.qemu);
        let Cli { command: cli::Command::Wait(w) } = parse(&["wait", "a"]).unwrap() else { panic!() };
        assert_eq!(w.timeout, 300.0);
        let Cli { command: cli::Command::MacOs { command: cli::MacOsCommand::Install(m) } } =
            parse(&["macos", "install", "--no-first-boot"]).unwrap()
        else {
            panic!()
        };
        assert!(m.cpus == 4 && m.memory == 8 && m.disk == 80 && m.no_first_boot);
    }

    #[test]
    fn ssh_passes_the_command_through() {
        let Cli { command: cli::Command::Ssh(s) } = parse(&["ssh", "a", "--", "ls", "-la"]).unwrap() else { panic!() };
        assert_eq!(s.command, ["ls", "-la"]);
        let Cli { command: cli::Command::Ssh(s) } = parse(&["ssh", "a", "uname", "-a"]).unwrap() else { panic!() };
        assert_eq!(s.command, ["uname", "-a"]);
    }

    #[test]
    fn call_and_serve() {
        let Cli { command: cli::Command::Call(c) } = parse(&["call", "vm.list", "{}"]).unwrap() else { panic!() };
        assert_eq!((c.method.as_str(), c.params.as_deref()), ("vm.list", Some("{}")));
        assert!(matches!(parse(&["serve"]).unwrap().command, cli::Command::Serve(cli::Serve { socket: None, .. })));
        let Cli { command: cli::Command::Serve(s) } = parse(&["serve", "--socket", "--idle-exit", "300"]).unwrap()
        else {
            panic!()
        };
        assert!(matches!(s.socket, Some(None)) && s.idle_exit == Some(300));
        let Cli { command: cli::Command::Serve(s) } = parse(&["serve", "--socket=/tmp/x.sock"]).unwrap() else {
            panic!()
        };
        assert_eq!(s.socket, Some(Some("/tmp/x.sock".into())));
        assert!(parse(&["serve", "--idle-exit", "3"]).is_err());
        let Cli { command: cli::Command::Call(c) } = parse(&["call", "--socket", "jobs.list"]).unwrap() else {
            panic!()
        };
        assert_eq!((c.socket, c.method.as_str()), (Some(None), "jobs.list"));
        assert!(matches!(
            parse(&["service", "status", "--json"]).unwrap().command,
            cli::Command::Service { command: cli::ServiceCommand::Status { json: true } }
        ));
    }

    #[test]
    fn usage_errors() {
        assert_eq!(parse(&["stop"]).unwrap_err().kind(), ErrorKind::MissingRequiredArgument);
        assert_eq!(parse(&["bogus"]).unwrap_err().kind(), ErrorKind::InvalidSubcommand);
        assert!(usage_of(&["sources", "add"]).starts_with("Usage: omaf sources add"));
        assert!(usage_of(&["new"]).starts_with("Usage: omaf new"));
    }
}
