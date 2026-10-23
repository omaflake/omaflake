// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of SSHCommand.swift: ssh invocation for an instance – shared by the CLI (`omaf ssh`) and the app
//! ("Open Terminal").

use crate::models::InstanceConfig;
use crate::paths::Paths;

/// Swift `enum SSHCommand` (namespace)
pub enum SshCommand {}

impl SshCommand {
    /// Swift's default target: NixOS port forwarding on 127.0.0.1
    pub const DEFAULT_HOST: &'static str = "127.0.0.1";

    /// Arguments including `/usr/bin/ssh` in first position. Default target: NixOS port forwarding on 127.0.0.1
    /// (`host` = `DEFAULT_HOST`, `port` None = the instance's SSH port); macOS guests pass their NAT address and port 22.
    pub fn arguments(
        c: &InstanceConfig,
        paths: &Paths,
        host: &str,
        port: Option<u16>,
        command: &[String],
    ) -> Vec<String> {
        // Host keys are new per instance → own known_hosts, accept on first connect.
        // Ephemeral instances generate new keys on every start → remember nothing (127.0.0.1 only).
        let host_keys: Vec<String> = if c.ephemeral {
            ["-o", "UserKnownHostsFile=/dev/null", "-o", "StrictHostKeyChecking=no", "-o", "LogLevel=ERROR"]
                .map(String::from)
                .to_vec()
        } else {
            vec![
                "-o".into(),
                format!("UserKnownHostsFile={}", paths.instance(&c.name).known_hosts().display()),
                "-o".into(),
                "StrictHostKeyChecking=accept-new".into(),
            ]
        };
        let mut args = vec!["/usr/bin/ssh".to_string(), "-p".into(), port.unwrap_or(c.ssh_port).to_string()];
        args.extend(host_keys);
        args.push(format!("{}@{host}", c.guest_user()));
        args.extend(command.iter().cloned());
        args
    }

    /// As a single shell line (for Terminal.app), arguments single-quoted.
    pub fn shell_line(arguments: &[String]) -> String {
        arguments.iter().map(|a| format!("'{}'", a.replace('\'', "'\\''"))).collect::<Vec<_>>().join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::tests::config;

    #[test]
    fn arguments_and_shell_line() {
        let p = Paths::new("/d", None);
        let mut c = config("t1");
        c.user = Some("me".into());
        let a = SshCommand::arguments(&c, &p, SshCommand::DEFAULT_HOST, None, &["uname".into()]);
        assert_eq!(
            a,
            vec![
                "/usr/bin/ssh",
                "-p",
                "2230",
                "-o",
                "UserKnownHostsFile=/d/instances/t1.omaflake-vm/known_hosts",
                "-o",
                "StrictHostKeyChecking=accept-new",
                "me@127.0.0.1",
                "uname"
            ]
        );
        c.ephemeral = true;
        let a = SshCommand::arguments(&c, &p, "192.168.64.2", Some(22), &[]);
        assert_eq!(a[2], "22");
        assert!(a.contains(&"UserKnownHostsFile=/dev/null".to_string()) && a.last().unwrap() == "me@192.168.64.2");
        assert_eq!(SshCommand::shell_line(&["a b".into(), "it's".into()]), "'a b' 'it'\\''s'");
    }
}
