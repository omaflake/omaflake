# omaflake – design tokens and artwork copies

The brand kit – the marks, the generator and every export – lives in its own repository,
[omaflake/brand](https://github.com/omaflake/brand). This folder holds what the app is built with:

- **Copies of the artwork** that omaflake uses, written by omaflake/brand's `copy.py` (with the checkout `../brand`
  next to this one). Never edit them here: change the artwork there, run its `generate.py` and `copy.py`, then
  `python3 brand/tokens.py` here. They are under CC BY-ND 4.0 (`REUSE.toml`).
- **The design tokens** (`tokens.toml`, `tokens.py`), which are code of this repository.

| Copy | Used by |
|---|---|
| `macos/omaflake.icns` | the macOS app bundle (`scripts/bundle-app.sh`) |
| `png/icon/`, `svg/omaflake-icon.svg` | the Linux package's icons (`linux/package.nix`), the Linux app's window and tray, QEMU's Dock icon, `BrandAssets.swift` |
| `png/menubar/` | the menu bar image (`BrandAssets.swift`) |
| `png/glyph-small/omaflake-glyph-small-white-48.png` | the Linux app's header |
| `png/loading/omaflake-loading.png` | the first frame QEMU's window shows until a NixOS guest draws (embedded in omaf) |
| `png/social/omaflake-social-1280x640.png` | this repository's README |
| `../nix/modules/plymouth/` (images, `layout.script`) | the NixOS guests' loading screen |
| `../docs/assets/` (favicons, mark) | omaflake.dev |

## Design tokens

The interfaces (the SwiftUI app on macOS, the iced app on Linux, plan B119) share one look through design tokens:
`tokens.toml` holds the brand palette (a copy of omaflake/brand's `palette.toml`), semantic colours (window, card,
accent, danger, warning, success, text …), opacities, spacing, corner radii, glow radii, sizes, icon (symbol) sizes,
font families, named fonts and the text styles' sizes.

To change a token: edit `tokens.toml`, then run `python3 tokens.py` (Python 3.11+, nothing else). It writes

- `Sources/OmaflakeApp/BrandTokens.swift` – `BrandTokens.Palette`, `.Colors`, `.Opacity`, `.Spacing`, `.Radius`,
  `.Glow`, `.Size`, `.Icon`, `.FontFamily`, `.Fonts`, `.TextStyle`; the app's `BrandStyle` is a set of shorthands for
  them,
- `crates/omaflake-brand/src/tokens.rs` – modules `palette`, `color` (`Color { rgba8, rgba }`), `opacity`, `spacing`,
  `radius`, `glow`, `size`, `icon`, `font`, `text` (`FontStyle`), plain values with no UI toolkit, and `ALL`,
- `brand/omaflake-look.toml` – the guests' omaflake look (plan B135) as look data (schema look/v1, plan B141), its
  colours from `[look]`; copy it into `omaflake/catalog` as `looks/omaflake/omaflake/look.toml` (no look is built
  into the image flake),
- `Sources/OmaflakeKit/Brand/BrandAssets.swift` – the app icon and the menu bar image from `png/` as base64, so the
  runner and the app need no resource bundle.

Never edit the generated files. `python3 tokens.py --check` (CI) and the crate's test `cargo test -p omaflake-brand`
fail when they are out of date. Colours with `system = "…"` stay the system colour in the macOS app (it follows the
system's accessibility settings); their hex value, macOS' dark-appearance colour, is for the other interfaces. Add a
token for a value that recurs or that another interface needs too, not for every one-off.

## Name and brand

omaflake's name and brand belong to falkOSS – Falcon Lake Open Source Software gUG (haftungsbeschränkt) i. G., the
publisher of omaflake; see omaflake/brand for how the artwork may be used.
