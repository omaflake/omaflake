# Custom QEMU build with 3D (spec M2, option A)

**Two ways to the same build:**
- **Nix package (for users, spec §11.2):** `nix profile install github:omaflake/omaflake#qemu`, or
  `packages.aarch64-darwin.qemu` of this flake in a nix-darwin configuration. `package.nix` overrides nixpkgs' QEMU,
  virglrenderer and libepoxy with the sources and patches below – no Homebrew. Binary cache: OPEN.
  Differences to `build.sh`: startergo's virglrenderer patches come as one clean re-diffed patch
  (`patches/startergo/`, GNU patch rejects some originals); D-Bus display and rutabaga are disabled (plan B29).
- **`build.sh` (for development):** builds into `~/.omaflake/qemu/<tag>/` with Homebrew's runtime libraries.

Homebrew QEMU lacks `virtio-gpu-gl-pci` (plan B6). `build.sh` therefore builds:

| Part | Version | Source | Patch origin |
|---|---|---|---|
| ANGLE | nixpkgs `angle` | built from source, BSD-3-Clause (package.nix, plan B55); `build.sh` still uses startergo's prebuilt bottle | – |
| libepoxy | `1b6d7db` | github.com/anholt/libepoxy | EGL 1.5 patch (akihikodaki) from startergo/homebrew-libepoxy v1.0.5 |
| virglrenderer | 1.3.0 | gitlab.freedesktop.org | 21 patches from startergo/homebrew-virglrenderer v1.0.42 + `virgl-native-opengl.patch` (try-omarchy) |
| QEMU | 11.1.2 | download.qemu.org | 6 patches from [try-omarchy](https://github.com/omacom/try-omarchy) (`patches/try-omarchy/`, MIT) |

All downloads are pinned by sha256 (ANGLE, virgl and the taps identical to the pins in try-omarchy).
Runtime libraries glib, pixman, libslirp, dtc come from Homebrew – **local use only**, not for redistribution.
License of the startergo patches: no LICENSE file, but README and formulae state MIT – used under MIT, see `patches/startergo/NOTICE`.

## Build

```sh
brew install meson ninja pkgconf glib pixman libslirp dtc
qemu/build.sh                       # ~2 min on an M1 Max → ~/.omaflake/qemu/<tag>/, symlink current
QEMU_BUILD_BASE=~/.omaflake/qemu-verify qemu/build.sh   # verification build, leaves the one in use untouched
omaf config --qemu ~/.omaflake/qemu/current/bin/qemu-system-aarch64
```

The result is signed ad hoc, with only `com.apple.security.hypervisor` (`entitlements.plist`).
A finished build has `<tag>/.complete`; leftovers of aborted runs are moved to `~/.omaflake/_to_delete`.

## Patches (QEMU, selection from try-omarchy, commit `6166ca0`)

| Patch | Purpose |
|---|---|
| `qemu-texture-borrowing-11.1` | virgl scanout textures straight to Cocoa (core of 3D output) |
| `qemu-gpu-spike-resolution-fix` | Cocoa: resolution jumps |
| `qemu-9p-guest-owner` | 9p `security_model=none,guest_owner_uid/gid`: Mac files appear owned by the guest user (plan B18) |
| `qemu-hvf-free-page-reclaim`, `qemu-hvf-mapped-sections` | HVF memory management |
| `qemu-darwin-gpu-fence-poll` | virtio-gpu fences on macOS |
| `qemu-cocoa-dynamic-display` (p2) | window size and Retina factor → guest resolution/EDID DPI; 16:9 initial window |

Own patches (`patches/omaflake/`):

| Patch | Purpose |
|---|---|
| `qemu-cocoa-initial-size` (p3) | with `zoom-to-fit`, the window follows the guest resolution until you resize it (otherwise it stays at ~640×480) |
| `qemu-cocoa-dock-icon` (p4) | Dock icon from the image file in `OMAFLAKE_DOCK_ICON` (set by `omaf` and the app); without it the bare binary shows the generic icon |

Not included: Cocoa comfort patches (branding, immersive, pinch, scroll, keyboard), SDL audio, USB –
they build on each other (`qemu-cocoa-full-grab-focus` does not apply without the previous ones).

## License

Unlike the rest of the repository (AGPL-3.0-or-later), the QEMU patches in `patches/` modify QEMU and are
GPL-2.0-or-later like QEMU itself. The try-omarchy patches keep their upstream notice in
`patches/try-omarchy/LICENSE`; `virgl-native-opengl.patch` modifies virglrenderer (MIT).

## Usage

`omaf start <name> --window` automatically uses 3D (`virtio-gpu-gl-pci`, `-display cocoa,gl=on`)
when the binary supports it. Default is native macOS OpenGL (GL 4.1 in the guest); `--gl es` selects ANGLE –
then the guest only sees GL 3.0 and e.g. kitty does not start (plan B22). `--no-gl` forces 2D.
Keyboard: Cmd = SUPER once the mouse is grabbed (click into the window; release with Ctrl+Alt+G).
Note: QMP `screendump` returns a black image for GL scanouts; take screenshots in the guest with `grim`.
