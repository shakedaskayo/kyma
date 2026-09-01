#!/usr/bin/env python3
"""rebrand-diagrams.py — bring the hand-drawn docs diagrams onto the Pensieve brand.

Two things the Kyma->Pensieve text codemod could not do, because both are
geometry and colour rather than words:

1. The diagrams embed the OLD kyma mark — a three-stroke "K" glyph — directly
   as SVG, in the same 96x96 coordinate space as the standalone mark. The text
   pass renamed the wordmark under it to "pensieve" and the colour pass turned
   the accents blue, which left a blue kyma "K" labelled "pensieve". This swaps
   the glyph for the basin-and-rising-memory mark from
   web/public/icons/pensieve-mark.svg.

2. The diagram palette was never the phosphor green the colour sweep targeted —
   it is a teal/sky "synapse" system. Teal carries FLOW IN (sources -> engine)
   and sky carries FLOW OUT (engine -> agents), a directional contrast worth
   keeping, so they map onto the brand's own two-hue pair rather than collapsing
   into one blue.

Idempotent: re-running finds no old glyph and no unmapped colours.

Usage: python3 scripts/rebrand-diagrams.py [--check]
"""

import pathlib
import re
import sys

TARGETS = sorted(
    set(pathlib.Path(".").glob("docs/images/*.svg"))
    | set(pathlib.Path(".").glob("docs/site/public/diagrams/*.svg"))
)

# ── Palette ──────────────────────────────────────────────────────────────────
# Sampled brand: primary #4C8DFF, violet #7C4DFF, cyan glow #54CDF4,
# pale #BFE4FF, deep #1F5FD6.
PALETTE = {
    "#2dd4bf": "#4C8DFF",  # teal-400  — flow IN / brand gradient / glow
    "#38bdf8": "#7C4DFF",  # sky-400   — flow OUT (keeps in/out distinguishable)
    "#34d399": "#54CDF4",  # emerald   — tertiary accent
    "#6ee7b7": "#BFE4FF",  # emerald light
    "#047857": "#1F5FD6",  # emerald dark — drop-shadow flood
    "#2a2e36": "#1d2740",  # mark core stroke, to match the standalone mark
    "#0d1015": "#080d18",  # mark core fill
    "#11151b": "#101a30",
}

# ── The old glyph, as emitted into every diagram ─────────────────────────────
OLD_GLYPH = re.compile(
    r'<g stroke="#e8e6e0"[^>]*>\s*'
    r'<line x1="34" y1="32" x2="34" y2="64"\s*/>\s*'
    r'<line x1="34" y1="48" x2="58" y2="34"\s*/>\s*'
    r'<line x1="34" y1="48" x2="58" y2="62"\s*/>\s*</g>\s*'
    r'<circle cx="58" cy="34" r="2\.5"[^>]*/>\s*'
    r'<circle cx="58" cy="62" r="2\.5"[^>]*/>',
    re.S,
)

SPIRAL = (
    "M 48.0 60.3 L 47.5 60.2 L 44.9 59.2 L 42.7 58.1 L 40.8 56.9 L 39.3 55.5 "
    "L 38.2 54.0 L 37.6 52.5 L 37.5 51.0 L 37.8 49.5 L 38.5 48.1 L 39.5 46.7 "
    "L 40.9 45.4 L 42.4 44.2 L 44.1 43.1 L 45.9 42.1 L 47.7 41.2 L 49.4 40.5 "
    "L 50.9 39.8 L 52.2 39.2 L 53.2 38.7 L 54.0 38.2 L 54.5 37.8 L 54.6 37.3 "
    "L 54.5 36.8 L 54.1 36.3 L 53.5 35.8 L 52.7 35.1 L 51.8 34.4 L 50.7 33.7 "
    "L 49.6 32.8 L 48.6 31.9 L 47.6 30.9 L 46.7 29.8 L 45.9 28.7 L 45.3 27.6 "
    "L 44.9 26.4 L 44.6 25.2 L 44.6 24.0 L 44.6 22.8 L 44.9 21.6 L 45.1 20.7"
)

NEW_GLYPH = (
    '<ellipse cx="48" cy="66" rx="20" ry="5.5" fill="none" stroke="#4C8DFF" stroke-width="3.2"/>'
    '<path d="M28 66 Q48 81 68 66" fill="none" stroke="#4C8DFF" stroke-width="3.2" '
    'stroke-linecap="round"/>'
    f'<path d="{SPIRAL}" fill="none" stroke="url(#pvVortex)" stroke-width="3.6" '
    'stroke-linecap="round" stroke-linejoin="round"/>'
)

VORTEX_DEF = (
    '<linearGradient id="pvVortex" x1="0" y1="1" x2="0.55" y2="0">'
    '<stop offset="0" stop-color="#BFE4FF"/>'
    '<stop offset="0.42" stop-color="#7CB0FF"/>'
    '<stop offset="1" stop-color="#7C4DFF"/>'
    "</linearGradient>"
)


def convert(text: str) -> tuple[str, bool, int]:
    swapped = False
    if OLD_GLYPH.search(text):
        text = OLD_GLYPH.sub(NEW_GLYPH, text)
        swapped = True
        # The spiral needs its gradient; add it once, right after <defs>.
        if "pvVortex" in text and 'id="pvVortex"' not in text.split("</defs>")[0]:
            text = text.replace("<defs>", "<defs>\n" + VORTEX_DEF, 1)

    recoloured = 0
    for old, new in PALETTE.items():
        for variant in (old, old.upper()):
            n = text.count(variant)
            if n:
                text = text.replace(variant, new)
                recoloured += n
    return text, swapped, recoloured


def main() -> int:
    check = "--check" in sys.argv
    dirty = 0
    for p in TARGETS:
        src = p.read_text()
        out, swapped, recoloured = convert(src)
        if out == src:
            continue
        dirty += 1
        marks = "glyph+" if swapped else ""
        print(f"{p}: {marks}{recoloured} colours")
        if not check:
            p.write_text(out)
    if check:
        print(f"{dirty} file(s) would change")
        return 1 if dirty else 0
    print(f"{dirty} file(s) rewritten")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
