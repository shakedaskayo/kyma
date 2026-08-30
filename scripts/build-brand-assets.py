#!/usr/bin/env python3
"""build-brand-assets.py — derive every raster brand asset from the vector mark.

Everything here is generated from web/public/icons/pensieve-mark.svg, never from
logo.png: the hero illustration embeds third-party marks (Slack, GitHub, Gmail,
Notion, Discord, …) and must not become an app icon or favicon.

Prereq: the mark rasterised to a 1024x1024 RGBA PNG. Headless Chrome does that:

    chrome --headless --disable-gpu --hide-scrollbars \
           --default-background-color=00000000 \
           --window-size=1024,1024 --screenshot=icon1024.png file://.../icon.html

Usage:  python3 scripts/build-brand-assets.py <icon1024.png> \
            [docs/images/pensieve-logo-source.png]

The second argument is the full-resolution hero illustration, used only for the
og:image and the README hero — never for an icon.
"""

import pathlib
import sys

from PIL import Image

GROUND = (5, 10, 24, 255)  # --brand-deep #050A18

# Tauri's bundle set. Names must match web/src-tauri/tauri.conf.json.
TAURI_ICONS = {
    "32x32.png": 32,
    "128x128.png": 128,
    "128x128@2x.png": 256,
    "Square30x30Logo.png": 30,
    "Square44x44Logo.png": 44,
    "Square71x71Logo.png": 71,
    "Square89x89Logo.png": 89,
    "Square107x107Logo.png": 107,
    "Square142x142Logo.png": 142,
    "Square150x150Logo.png": 150,
    "Square284x284Logo.png": 284,
    "Square310x310Logo.png": 310,
    "StoreLogo.png": 50,
    "icon.png": 512,
}


def on_ground(mark: Image.Image, size: int) -> Image.Image:
    """Composite the mark on the brand ground — for surfaces that flatten alpha."""
    bg = Image.new("RGBA", (size, size), GROUND)
    return Image.alpha_composite(bg, mark.resize((size, size), Image.LANCZOS))


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 2

    src = Image.open(sys.argv[1]).convert("RGBA")
    hero = pathlib.Path(sys.argv[2]) if len(sys.argv) > 2 else None

    # ── favicon.ico — multi-resolution, transparent ──────────────────────────
    src.save(
        "web/public/favicon.ico",
        format="ICO",
        sizes=[(16, 16), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
    )

    # ── apple-touch-icon — iOS composites on white, so bake the ground in ────
    on_ground(src, 180).convert("RGB").save(
        "web/public/apple-touch-icon.png", optimize=True
    )

    # ── Tauri bundle icons — these were stock Tauri defaults until now ───────
    tauri = pathlib.Path("web/src-tauri/icons")
    tauri.mkdir(parents=True, exist_ok=True)
    for name, size in TAURI_ICONS.items():
        src.resize((size, size), Image.LANCZOS).save(tauri / name, optimize=True)

    # .ico for Windows, .icns for macOS
    src.save(
        tauri / "icon.ico",
        format="ICO",
        sizes=[(16, 16), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
    )
    try:
        on_ground(src, 1024).convert("RGB").save(tauri / "icon.icns", format="ICNS")
    except Exception as exc:  # Pillow's ICNS writer is platform-sensitive
        print(f"  icon.icns skipped: {exc}", file=sys.stderr)

    # ── docs-site copies ────────────────────────────────────────────────────
    docs = pathlib.Path("docs/site/public")
    (docs / "icons").mkdir(parents=True, exist_ok=True)
    src.save(
        docs / "favicon.ico",
        format="ICO",
        sizes=[(16, 16), (32, 32), (48, 48), (64, 64)],
    )

    # ── og.png — 1200x630. Scrapers mostly refuse SVG, hence a real raster ──
    og = Image.new("RGBA", (1200, 630), GROUND)
    if hero and hero.exists():
        art = Image.open(hero).convert("RGBA")
        side = 560
        art = art.resize((side, side), Image.LANCZOS)
        og.alpha_composite(art, ((1200 - side) // 2, (630 - side) // 2))
    else:
        side = 420
        og.alpha_composite(
            src.resize((side, side), Image.LANCZOS),
            ((1200 - side) // 2, (630 - side) // 2),
        )
    og.convert("RGB").save(docs / "og.png", quality=92, optimize=True)

    # ── README / docs hero, downsized from the 2 MB source ──────────────────
    if hero and hero.exists():
        art = Image.open(hero).convert("RGBA")
        art.thumbnail((720, 720), Image.LANCZOS)
        pathlib.Path("docs/images").mkdir(parents=True, exist_ok=True)
        art.save("docs/images/pensieve-hero.png", optimize=True)

    print("brand assets written")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
