# Linux builder on macOS

NixOS guest images are `aarch64-linux` derivations. A Mac cannot build them natively, so Nix needs a
Linux builder. The simplest option is nix-darwin's `nix.linux-builder`: a small NixOS VM that nix-darwin
runs in the background and registers as a remote builder.

## Configuration

omaflake's image flake ships one: `darwinConfigurations.linux-builder` in
[nix/hosts/linux-builder.nix](../nix/hosts/linux-builder.nix) (machine-specific additions go into
`nix/local/darwin.nix`). In a flake of your own, the essential part is:

```nix
{ ... }: {
  nixpkgs.hostPlatform = "aarch64-darwin";
  nix.enable = true;
  nix.settings.experimental-features = [ "nix-command" "flakes" ];
  nix.linux-builder = {
    enable = true;
    ephemeral = false; # keep the builder's Nix store → fewer rebuilds
    config = {
      virtualisation.cores = 6;
      virtualisation.darwin-builder.memorySize = 12 * 1024; # MiB
      virtualisation.darwin-builder.diskSize = 100 * 1024;  # MiB
      # Every image build leaves several GB behind. When less than min-free is left during a build, Nix collects
      # garbage until max-free is free. The defaults (1 and 3 GiB) are too small for a ~9 GiB disk image (plan B38).
      virtualisation.darwin-builder.min-free = 20 * 1024 * 1024 * 1024; # bytes
      virtualisation.darwin-builder.max-free = 50 * 1024 * 1024 * 1024;
    };
  };
}
```

Adjust cores, memory and disk to your machine. `min-free`/`max-free` make the builder clean up after itself:
when a build leaves it less than 20 GiB, Nix deletes unused store paths until 50 GiB are free (everything it
deletes can be rebuilt or downloaded again). The builder's own defaults (1 and 3 GiB) are too small: a NixOS disk image alone needs ~9 GiB while it is written.

## Activation

The app does all of this by click (Doctor → Nix "Install …", nix-darwin "Set Up …"), the CLI with
`omaf builder setup`; by hand:

nix-darwin takes over `/etc/nix/nix.conf`; move the unmanaged one aside first (rename, don't delete).
Do the same for any other file the activation reports as "unexpected" in `/etc` (e.g. `/etc/zshrc`, `/etc/bashrc`):

```sh
sudo mv /etc/nix/nix.conf /etc/nix/nix.conf.before-nix-darwin
cd nix
sudo nix --extra-experimental-features 'nix-command flakes' \
  run nix-darwin/master#darwin-rebuild -- switch --flake "path:$PWD#linux-builder"
```

The extra flags are needed because without `nix.conf` nothing enables flakes. Later activations:
`sudo darwin-rebuild switch --flake "path:$PWD#linux-builder"`. Check with `sudo ssh linux-builder uname -m`
(expected: `aarch64`).

Afterwards, every NixOS image build (`omaf build`, the app) runs on the Linux builder transparently.

## Pitfalls

- **Default size is too small.** The default builder (1 core, 3 GiB RAM, 20 GiB disk) ran out of space on
  the first image build. Use the larger `config` above.
- **Two-stage setup.** A changed builder configuration is itself an `aarch64-linux` build, so it is built by
  the builder that is currently running. Activate with the defaults first, then switch to the larger config.
- **Disk full ("No space left on device").** Happens with the default `min-free`/`max-free` (above). One-off clean-up:
  the app's Doctor → Linux builder → "Clean Up …", `omaf builder clean-up`, or `sudo ssh linux-builder 'nix-store --gc'`.
  Changed builder settings: Doctor → "Apply Settings …" or `omaf builder apply` (both run the `darwin-rebuild` above).
- **Disk is not resized automatically.** The builder disk (`nixos.qcow2` in the builder's `workingDirectory`)
  is only created if it is missing. To apply a new `diskSize`, move the old disk away and restart the builder;
  it needs about a minute before it accepts connections:

  ```sh
  sudo mkdir -p ~/.omaflake/_to_delete
  sudo mv /var/lib/linux-builder/nixos.qcow2 ~/.omaflake/_to_delete/linux-builder-old.qcow2
  sudo launchctl kickstart -k system/org.nixos.linux-builder
  sudo ssh linux-builder 'nproc; free -g; df -h /'
  ```

  `sudo mkdir` makes `~/.omaflake` root-owned if it did not exist; give it back with `sudo chown -R "$USER":staff ~/.omaflake`.
- **`/etc/synthetic.conf` must exist.** Without it the activation fails with a `stat`/`chmod` error and `/nix`
  is missing after the next reboot. Expected content (check again after macOS updates):

  ```sh
  printf 'nix\nrun\tprivate/var/run\n' | sudo tee /etc/synthetic.conf
  sudo chmod 644 /etc/synthetic.conf
  ```
- **Existing Nix installation.** An existing multi-user Nix install (`nixbld` group, GID 350) is adopted by
  nix-darwin; no reinstall is needed.
- **No KVM.** There is no KVM inside the builder, so image creation runs under TCG emulation. It is slower,
  but the `minimal` example recipe (including Hyprland) still builds in a few minutes.
