# omaflake – brand assets

Two marks:

- **Hero** (chaotic, RGB-split snowflake on near-black) – app icon, GitHub, website, social.
- **Glyph** (the hero flake in one colour: same tilt, no RGB split, no glitch bands, no background) –
  monochrome use. **Small glyph** (same tilt, one chevron per arm, heavy stroke) – menu bar, favicon, 16–48 px.

Regenerate everything from code (including `Sources/OmaflakeKit/Brand/BrandAssets.swift` and the design tokens): `python3 generate.py` (needs `rsvg-convert` from Homebrew `librsvg`
and macOS `iconutil`). Edit `generate.py`, not the exported files.

omaflake's name and brand belong to falkOSS – Falcon Lake Open Source Software gUG (haftungsbeschränkt) i. G., the
publisher of omaflake. The look stays omaflake's own – falkOSS's design is not used here; project pages say "a falkOSS
project" at most. The artwork may be used to refer to omaflake; not for other products or in a way that suggests
endorsement.

## Where to use what

| Use | File |
|---|---|
| macOS app icon | `macos/omaflake.icns` (16/32 px slots: small glyph on the icon background); copied into the bundle by `scripts/bundle-app.sh` |
| Swift targets (menu bar, runner/QEMU Dock icon) | `Sources/OmaflakeKit/Brand/BrandAssets.swift` – generated, embeds `png/icon/omaflake-icon-512.png` and `png/menubar/`; API in `Brand.swift` |
| Menu bar (template image, macOS tints it) | `png/menubar/omaflakeTemplate.png`, `@2x`, `@3x` or `svg/omaflake-menubarTemplate.svg` |
| GitHub org avatar | `social/github-avatar-500.png` |
| GitHub social preview (repo settings) | `social/omaflake-social-1280x640.png` |
| Developer docs (omaflake.dev) | `docs/assets/` – copies of `web/favicon.ico`, `web/favicon.svg` and `svg/omaflake-mark-on-dark.svg` (GitHub Pages serves only `docs/`); copy them again after `generate.py` |
| Website `<head>` | `web/` – favicon.ico, favicon.svg (transparent, adapts to light/dark), apple-touch-icon.png, icon-192/512.png, site.webmanifest |
| README / docs header | `png/mark/omaflake-mark-on-dark-*.png` or `-on-light-*.png` (transparent) |
| Rounded app icon as image | `png/icon/omaflake-icon-*.png` (transparent corners, macOS grid) |
| Square, edge-to-edge | `png/icon-square/omaflake-icon-square-*.png` |
| Monochrome (stickers, print, single colour) | `svg/omaflake-glyph-black.svg`, `-white.svg`, `png/glyph/` |
| Monochrome, tiny (16–48 px) | `svg/omaflake-glyph-small-black.svg`, `-white.svg`, `png/glyph-small/` |

Website head:

```html
<link rel="icon" href="/favicon.ico" sizes="any">
<link rel="icon" href="/favicon.svg" type="image/svg+xml">
<link rel="apple-touch-icon" href="/apple-touch-icon.png">
<link rel="manifest" href="/site.webmanifest">
```

## Design tokens

The interfaces (the SwiftUI app on macOS, the iced app on Linux, plan B119) share one look through design tokens:
`tokens.toml` holds the brand palette (also used by `generate.py` for the artwork), semantic colours (window, card,
accent, danger, warning, success, text …), opacities, spacing, corner radii, glow radii, sizes, icon (symbol) sizes,
font families, named fonts and the text styles' sizes.

To change a token: edit `tokens.toml`, then run `python3 tokens.py` (Python 3.11+, nothing else; `generate.py` runs it
too). It writes

- `Sources/OmaflakeApp/BrandTokens.swift` – `BrandTokens.Palette`, `.Colors`, `.Opacity`, `.Spacing`, `.Radius`,
  `.Glow`, `.Size`, `.Icon`, `.FontFamily`, `.Fonts`, `.TextStyle`; the app's `BrandStyle` is a set of shorthands for
  them,
- `crates/omaflake-brand/src/tokens.rs` – modules `palette`, `color` (`Color { rgba8, rgba }`), `opacity`, `spacing`,
  `radius`, `glow`, `size`, `icon`, `font`, `text` (`FontStyle`), plain values with no UI toolkit, and `ALL`.
- `nix/looks/omaflake.toml` – the guests' omaflake look (plan B135) as look data (schema look/v1, plan B141), its
  colours from `[look]`; the image flake cannot read `brand/`, so it gets them generated. The copy in
  `omaflake/catalog` (`looks/omaflake/omaflake/look.toml`) is the same file.

Never edit the generated files. `python3 tokens.py --check` (CI) and the crate's test `cargo test -p omaflake-brand`
fail when they are out of date. Colours with `system = "…"` stay the system colour in the macOS app (it follows the
system's accessibility settings); their hex value, macOS' dark-appearance colour, is for the other interfaces. Add a
token for a value that recurs or that another interface needs too, not for every one-off.

## Colours

| Name | Hex |
|---|---|
| Background | `#121212` |
| Red | `#FF3B3B` |
| Cyan | `#00E5FF` |
| Yellow | `#F5FF3B` |
| White | `#FFFFFF` |

Source: `[palette]` in `tokens.toml`.

Wordmark: `omaflake` in lowercase, Menlo Bold (monospace), with the same red/cyan offset as the hero.

## Direction: icons for the things in the app (2026-10-18, later)

The app uses SF Symbols today (monitor = VM, package = image, book = recipe; actions as a small cyan sub-symbol).
Own icons may take up the snow of the logo: VM = snowman, image = snowball, recipe = snow; the flake stays the Nix
flake behind it all (not shown in the app). Same rules as the glyph: one weight, readable at 13 pt, red/cyan only as
accents.
