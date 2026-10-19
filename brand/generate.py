#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Generates the omaflake brand bundle: master SVGs, PNG exports, .icns, favicon.ico and
Sources/OmaflakeCore/BrandAssets.swift (icons embedded for the Swift targets).

Needs rsvg-convert (Homebrew: librsvg) and iconutil (macOS). Run from anywhere:
    python3 generate.py
"""
import base64, os, random, shutil, struct, subprocess

HERE = os.path.dirname(os.path.abspath(__file__))
SVG, PNG = os.path.join(HERE, "svg"), os.path.join(HERE, "png")

BG = "#121212"          # icon background
RED, CYAN, YELLOW, WHITE = "#FF3B3B", "#00E5FF", "#F5FF3B", "#FFFFFF"
TILT = 14               # rotation of the hero flake in degrees


def svg(view_box, body, size=None):
    w = h = size or ""
    dims = f' width="{w}" height="{h}"' if size else ""
    return f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{view_box}"{dims}>\n{body}\n</svg>\n'


def six(ref, cx=512, cy=512, attrs=""):
    return "".join(f'<use href="#{ref}" transform="rotate({a} {cx} {cy})"{attrs}/>' for a in range(0, 360, 60))


# --- chaotic hero ---------------------------------------------------------------------------

ARM = ('<line x1="512" y1="440" x2="512" y2="230"/>'
       '<polyline points="446,356 512,296 578,356"/><polyline points="470,262 512,224 554,262"/>')


def hero_defs(p):
    """Defs for the chaotic flake; p is an id prefix so several can share one document."""
    return (f'<g id="{p}arm" fill="none" stroke-width="30" stroke-linecap="round" stroke-linejoin="round">{ARM}</g>'
            f'<g id="{p}flake">{six(p + "arm")}</g>'
            f'<mask id="{p}bands" maskUnits="userSpaceOnUse" x="-2000" y="-2000" width="5000" height="5000">'
            f'<rect x="-2000" y="-2000" width="5000" height="5000" fill="#fff"/>'
            f'<rect x="-2000" y="470" width="5000" height="22" fill="#000"/>'
            f'<rect x="-2000" y="600" width="5000" height="10" fill="#000"/></mask>')


def hero_flake(p, core=WHITE):
    """RGB-split flake with glitch bands cut out."""
    t = f"rotate({TILT} 512 512)"
    return (f'<g mask="url(#{p}bands)">'
            f'<use href="#{p}flake" stroke="{RED}" transform="translate(-16 6) {t}" opacity="0.85"/>'
            f'<use href="#{p}flake" stroke="{CYAN}" transform="translate(16 -6) {t}" opacity="0.85"/>'
            f'<use href="#{p}flake" stroke="{core}" transform="{t}"/></g>')


def hero_noise(p):
    """Scattered pixels and mini flakes around the hero."""
    rnd = random.Random(7)
    pixels = "".join(
        f'<rect x="{rnd.randint(120, 880)}" y="{rnd.randint(120, 880)}" width="{rnd.choice([10, 18, 26])}" '
        f'height="{rnd.choice([10, 18])}" fill="{rnd.choice([RED, CYAN, WHITE, YELLOW])}" opacity="{rnd.choice([0.5, 0.8, 1])}"/>'
        for _ in range(40))
    minis = "".join(
        f'<use href="#{p}flake" transform="translate({x} {y}) scale({s}) rotate({r} 512 512)" stroke="{c}" opacity="0.7"/>'
        for x, y, s, r, c in [(130, 120, .22, 20, YELLOW), (690, 170, .18, -12, CYAN), (150, 690, .2, 33, RED),
                              (720, 700, .16, 5, WHITE), (600, 90, .12, 40, RED)])
    return f'<g mask="url(#{p}bands)">{pixels}{minis}</g>'


def hero_scene(p):
    return hero_noise(p) + hero_flake(p)


# macOS icon grid: 824x824 rounded square centred on a 1024 canvas
ICON_RECT = 'x="100" y="100" width="824" height="824" rx="185"'


def app_icon():
    return svg("0 0 1024 1024",
               f'<defs>{hero_defs("h-")}<clipPath id="clip"><rect {ICON_RECT}/></clipPath></defs>'
               f'<rect {ICON_RECT} fill="{BG}"/><g clip-path="url(#clip)">{hero_scene("h-")}</g>', 1024)


def fullbleed():
    """Square, edge to edge: GitHub avatar, social, PWA, apple-touch-icon."""
    return svg("100 100 824 824",
               f'<defs>{hero_defs("h-")}</defs><rect x="100" y="100" width="824" height="824" fill="{BG}"/>{hero_scene("h-")}',
               1024)


MARK_BOX = "150 150 724 724"


def mark(core):
    """The hero flake alone, transparent background."""
    return svg(MARK_BOX, f'<defs>{hero_defs("h-")}</defs>{hero_flake("h-", core)}', 1024)


# --- glyph: the hero flake, one colour, no split, no bands ------------------------------------

# Full glyph: exactly the hero's arm and tilt, for 64 px and up.
GLYPH_BOX = MARK_BOX


def glyph_body(p, color, width=34):
    return (f'<defs><g id="{p}arm" fill="none" stroke="{color}" stroke-width="{width}" stroke-linecap="round" '
            f'stroke-linejoin="round">{ARM}</g></defs><g transform="rotate({TILT} 512 512)">{six(p + "arm")}</g>')


def glyph(color):
    return svg(GLYPH_BOX, glyph_body("g-", color), 1024)


# Small glyph: same tilt and open core, but one wide chevron per arm and a heavy stroke,
# so it stays readable at 16-18 px (menu bar, favicon).
SMALL_ARM = '<line x1="512" y1="424" x2="512" y2="250"/><polyline points="446,316 512,250 578,316"/>'
SMALL_BOX = "206 206 612 612"


def small_body(p, color, width=60):
    return (f'<defs><g id="{p}arm" fill="none" stroke="{color}" stroke-width="{width}" stroke-linecap="round" '
            f'stroke-linejoin="round">{SMALL_ARM}</g></defs><g transform="rotate({TILT} 512 512)">{six(p + "arm")}</g>')


def menubar():
    """Template image: black on transparent, macOS tints it for light/dark."""
    return svg(SMALL_BOX, small_body("m-", "#000"), 18)


def glyph_small(color):
    return svg(SMALL_BOX, small_body("s-", color), 1024)


def favicon():
    """Transparent; dark flake on light browser chrome, light flake on dark (SVG only)."""
    return svg(SMALL_BOX,
               f'<style>.f{{color:{BG}}} @media (prefers-color-scheme: dark){{.f{{color:{WHITE}}}}}</style>'
               f'<g class="f">{small_body("f-", "currentColor")}</g>', 32)


def app_icon_small():
    """App icon for the 16 and 32 px slots: small glyph on the icon background."""
    return svg("0 0 1024 1024",
               f'<rect {ICON_RECT} fill="{BG}"/>'
               f'<g transform="translate(512 512) scale(1.25) translate(-512 -512)">{small_body("s-", WHITE)}</g>', 1024)


def social():
    rnd = random.Random(11)
    pixels = "".join(
        f'<rect x="{rnd.randint(0, 1270)}" y="{rnd.randint(0, 630)}" width="{rnd.choice([8, 14, 20])}" height="{rnd.choice([8, 14])}" '
        f'fill="{rnd.choice([RED, CYAN, WHITE, YELLOW])}" opacity="{rnd.choice([0.3, 0.6, 0.9])}"/>' for _ in range(70))
    word = 'font-family="Menlo, monospace" font-weight="bold" font-size="128"'
    tag = 'font-family="Menlo, monospace" font-size="27" fill="#B8C0CC"'
    return svg("0 0 1280 640",
               f'<defs>{hero_defs("h-")}</defs><rect width="1280" height="640" fill="{BG}"/>{pixels}'
               f'<g transform="translate(270 320) scale(0.62) translate(-512 -512)">{hero_flake("h-")}</g>'
               f'<text x="526" y="320" {word} fill="{RED}" opacity="0.85">omaflake</text>'
               f'<text x="546" y="312" {word} fill="{CYAN}" opacity="0.85">omaflake</text>'
               f'<text x="536" y="316" {word} fill="{WHITE}">omaflake</text>'
               f'<text x="540" y="392" {tag}>Opinionated, disposable NixOS and</text>'
               f'<text x="540" y="430" {tag}>macOS VMs on Apple silicon.</text>', 1280)


# --- export ----------------------------------------------------------------------------------

def write(path, content):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(content)


def render(src, dst, w, h=None):
    os.makedirs(os.path.dirname(dst), exist_ok=True)
    subprocess.run(["rsvg-convert", "-w", str(w), "-h", str(h or w), src, "-o", dst], check=True)


def ico(dst, pngs):
    """favicon.ico with embedded PNGs."""
    data = [open(p, "rb").read() for _, p in pngs]
    out = struct.pack("<HHH", 0, 1, len(pngs))
    offset = 6 + 16 * len(pngs)
    for (size, _), d in zip(pngs, data):
        out += struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(d), offset)
        offset += len(d)
    with open(dst, "wb") as f:
        f.write(out + b"".join(data))


def swift_assets(dst):
    """Icon PNGs embedded as base64 for OmaflakeCore (Brand.swift), so CLI, runner and app need no resource bundle."""
    def b64(path):
        data = base64.b64encode(open(path, "rb").read()).decode()
        return "\n".join(data[i:i + 120] for i in range(0, len(data), 120))

    def literal(name, path):
        return f'    static let {name} = """\n{b64(path)}\n"""\n'

    body = ("// SPDX-License-Identifier: AGPL-3.0-or-later\n// Generated by brand/generate.py – do not edit.\n\n"
            "enum BrandAssets {\n"
            "    /// App icon, 512 px (png/icon/omaflake-icon-512.png)\n"
            + literal("appIcon512", f"{PNG}/icon/omaflake-icon-512.png")
            + "    /// Menu bar template image, 18 pt at 1x/2x/3x (png/menubar/)\n"
            + literal("menuBar1x", f"{PNG}/menubar/omaflakeTemplate.png")
            + literal("menuBar2x", f"{PNG}/menubar/omaflakeTemplate@2x.png")
            + literal("menuBar3x", f"{PNG}/menubar/omaflakeTemplate@3x.png")
            + "}\n")
    write(dst, body)


def main():
    masters = {
        "omaflake-icon.svg": app_icon(),
        "omaflake-icon-small.svg": app_icon_small(),
        "omaflake-icon-square.svg": fullbleed(),
        "omaflake-mark-on-dark.svg": mark(WHITE),
        "omaflake-mark-on-light.svg": mark(BG),
        "omaflake-glyph-black.svg": glyph("#000000"),
        "omaflake-glyph-white.svg": glyph(WHITE),
        "omaflake-glyph-small-black.svg": glyph_small("#000000"),
        "omaflake-glyph-small-white.svg": glyph_small(WHITE),
        "omaflake-menubarTemplate.svg": menubar(),
        "favicon.svg": favicon(),
        "omaflake-social.svg": social(),
    }
    for name, content in masters.items():
        write(os.path.join(SVG, name), content)
    s = lambda n: os.path.join(SVG, n)

    for n in (16, 32, 64, 128, 256, 512, 1024):
        render(s("omaflake-icon-small.svg" if n <= 32 else "omaflake-icon.svg"), f"{PNG}/icon/omaflake-icon-{n}.png", n)
    for n in (128, 256, 512, 1024):
        render(s("omaflake-icon-square.svg"), f"{PNG}/icon-square/omaflake-icon-square-{n}.png", n)
    for v in ("on-dark", "on-light"):
        for n in (64, 128, 256, 512, 1024):
            render(s(f"omaflake-mark-{v}.svg"), f"{PNG}/mark/omaflake-mark-{v}-{n}.png", n)
    for v in ("black", "white"):
        for n in (32, 64, 128, 256, 512, 1024):
            render(s(f"omaflake-glyph-{v}.svg"), f"{PNG}/glyph/omaflake-glyph-{v}-{n}.png", n)
        for n in (16, 18, 24, 32, 48):
            render(s(f"omaflake-glyph-small-{v}.svg"), f"{PNG}/glyph-small/omaflake-glyph-small-{v}-{n}.png", n)
    for scale, suffix in ((1, ""), (2, "@2x"), (3, "@3x")):
        render(s("omaflake-menubarTemplate.svg"), f"{PNG}/menubar/omaflakeTemplate{suffix}.png", 18 * scale)

    web = os.path.join(HERE, "web")
    for n in (16, 32, 48):
        render(s("favicon.svg"), f"{web}/favicon-{n}.png", n)
    ico(f"{web}/favicon.ico", [(n, f"{web}/favicon-{n}.png") for n in (16, 32, 48)])
    shutil.copy(s("favicon.svg"), f"{web}/favicon.svg")
    render(s("omaflake-icon-square.svg"), f"{web}/apple-touch-icon.png", 180)
    for n in (192, 512):
        render(s("omaflake-icon-square.svg"), f"{web}/icon-{n}.png", n)
    write(f"{web}/site.webmanifest",
          '{\n  "name": "omaflake",\n  "short_name": "omaflake",\n  "icons": [\n'
          '    { "src": "/icon-192.png", "sizes": "192x192", "type": "image/png" },\n'
          '    { "src": "/icon-512.png", "sizes": "512x512", "type": "image/png" }\n  ],\n'
          f'  "theme_color": "{BG}",\n  "background_color": "{BG}",\n  "display": "standalone"\n}}\n')

    social_dir = os.path.join(HERE, "social")
    render(s("omaflake-social.svg"), f"{social_dir}/omaflake-social-1280x640.png", 1280, 640)
    render(s("omaflake-icon-square.svg"), f"{social_dir}/github-avatar-500.png", 500)

    # macOS app icon
    iconset = os.path.join(HERE, "macos", "omaflake.iconset")
    os.makedirs(iconset, exist_ok=True)
    for base in (16, 32, 128, 256, 512):
        for scale in (1, 2):
            px = base * scale
            src = "omaflake-icon-small.svg" if px <= 32 else "omaflake-icon.svg"
            render(s(src), f"{iconset}/icon_{base}x{base}{'@2x' if scale == 2 else ''}.png", px)
    subprocess.run(["iconutil", "-c", "icns", iconset, "-o", os.path.join(HERE, "macos", "omaflake.icns")], check=True)
    # The macOS lab recipe's test app uses the same icon: omaflake/recipes keeps a copy of macos/omaflake.icns
    # (recipes/omaflake/macos/lab/omaflake-test.icns) – copy it there by hand when the icon changes

    swift_assets(os.path.join(HERE, "..", "Sources", "OmaflakeCore", "BrandAssets.swift"))
    print("ok")


if __name__ == "__main__":
    main()
