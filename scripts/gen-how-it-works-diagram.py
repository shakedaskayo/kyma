#!/usr/bin/env python3
"""gen-how-it-works-diagram.py — draw docs/images/how-it-works.svg.

architecture.svg is the internals schematic — two lanes, the stateless spine,
the five invariants. It is accurate and worth keeping, but it explains the
engine rather than the product, so it is the wrong thing to lead with.

This is the product diagram that leads instead. It tells the Pensieve story. You put memories into the basin; your agents
draw them back out. Deposits run down a clear channel on the left into the bowl,
memories rise as a vortex, and agents pull from the risen memories on the right.

Generated rather than hand-authored for two reasons: the vortex is a computed
spiral (the same one the brand mark uses, scaled up), and the vendor glyphs are
extracted from docs/site/public/icons/brand/*.svg at build time so they stay
byte-exact instead of being hand-copied and drifting.

Layout rule that matters: nothing crosses anything. Deposit beams stay in the
x≈300-360 channel, left of the memory sheets; recall beams live in x≈660-770,
right of them. Text is never overdrawn.

Usage: python3 scripts/gen-how-it-works-diagram.py
"""

from __future__ import annotations

import math
import pathlib
import re
import sys

W, H = 1040, 720

# Mono sub-labels are clipped to what actually fits inside a card. At 11px the
# mono advance is ~6.6px, and a card gives 236 - 36 = 200px of inner width, so
# anything past ~30 characters overflows the card and collides with the beams
# routed alongside it. Enforced in build() rather than trusted.
MONO_ADVANCE = 6.6
CARD_W, CARD_PAD = 236, 18

INK = "#e6edf3"
DIM = "#8da2b6"
FAINT = "#5d6f80"
RULE = "#243140"
PANEL = "#141b24"
PANEL_HI = "#18212e"
BG = "#0b1220"

BLUE = "#4C8DFF"      # deposits — what goes in
VIOLET = "#7C4DFF"    # recall — what agents draw out
PALE = "#BFE4FF"
CYAN = "#54CDF4"

BASIN_CX, BASIN_CY = 520, 548
BASIN_RX, BASIN_RY = 134, 31
BOWL_DEPTH = 84          # bowl bottom = BASIN_CY + BOWL_DEPTH, must clear the rule
FOOTER_RULE_Y = 664

SHEET_X, SHEET_W = 386, 268
SHEETS = [
    (214, "Memories", "facts · decisions · preferences"),
    (300, "Live data", "logs · traces · code — in KQL or SQL"),
    (386, "The graph", "how every one of them connects"),
]

ICON_DIR = pathlib.Path("docs/site/public/icons/brand")


def vendor_glyph(name: str, recolour: str | None = None) -> str:
    """Lift the drawing elements out of a brand icon, exactly as authored.

    Hand-copying these paths is how they drift, so read the real file. The
    icons are 24x24 single-path marks; anything else is rejected loudly rather
    than silently rendering wrong.
    """
    src = (ICON_DIR / f"{name}.svg").read_text()
    if 'viewBox="0 0 24 24"' not in src:
        raise SystemExit(f"{name}.svg is not a 24x24 icon; refusing to guess a transform")
    body = "".join(re.findall(r"<path\b[^>]*/>", src))
    if not body:
        raise SystemExit(f"{name}.svg has no self-closing <path>; extract it by hand")
    if recolour:
        body = re.sub(r'fill="[^"]*"', f'fill="{recolour}"', body)
    return body


def vortex_path(y_top: float = 236.0, turns: float = 1.55, r0: float = 104.0,
                phase: float = 0.0) -> str:
    """The mark's spiral, scaled to rise out of the basin.

    `phase` offsets the start angle so a second, fainter pass can be drawn
    behind the first — one ribbon alone reads as an S rather than a vortex.
    """
    y0 = BASIN_CY - 20
    steps, r1 = 260, 13.0
    pts = []
    for i in range(steps + 1):
        t = i / steps
        th = math.pi / 2 + phase + turns * 2 * math.pi * t
        r = r0 + (r1 - r0) * (t ** 0.78)
        yc = y0 - (y0 - y_top) * (t ** 1.12)
        pts.append((BASIN_CX + r * math.cos(th), yc + r * 0.28 * math.sin(th)))
    d = "M %.1f %.1f" % pts[0]
    for x, y in pts[1::4]:
        d += " L %.1f %.1f" % (x, y)
    return d + " L %.1f %.1f" % pts[-1]


def fits(sub: str) -> str:
    """Refuse to emit a sub-label that would overflow its card."""
    if len(sub) * MONO_ADVANCE > CARD_W - 2 * CARD_PAD:
        raise SystemExit(
            f"sub-label overflows its card ({len(sub)} chars, max "
            f"{int((CARD_W - 2 * CARD_PAD) / MONO_ADVANCE)}): {sub!r}"
        )
    return sub


def card(x, y, w, h, title, sub, accent):
    return f"""
  <g>
    <rect x="{x}" y="{y}" width="{w}" height="{h}" rx="11" fill="{PANEL}" stroke="{RULE}"/>
    <rect x="{x}" y="{y + 13}" width="3" height="{h - 26}" rx="1.5" fill="{accent}"/>
    <text x="{x + 18}" y="{y + 30}" font-size="15" font-weight="650" fill="{INK}">{title}</text>
    <text x="{x + 18}" y="{y + 51}" font-size="11" font-family="ui-monospace,SFMono-Regular,Menlo,monospace"
          fill="{DIM}">{sub}</text>
  </g>"""


def agent_card(x, y, w, h, label, glyph):
    return f"""
  <g>
    <rect x="{x}" y="{y}" width="{w}" height="{h}" rx="11" fill="{PANEL}" stroke="{RULE}"/>
    <g transform="translate({x + 17} {y + h / 2 - 12})">{glyph}</g>
    <text x="{x + 55}" y="{y + h / 2 + 5}" font-size="14.5" font-weight="600" fill="{INK}">{label}</text>
  </g>"""


def build() -> str:
    p = []
    a = p.append

    a(f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" role="img" '
      'aria-label="Pensieve: your sessions, code and systems go into the basin; your coding '
      'agents draw the memories back out">')

    # ── defs ────────────────────────────────────────────────────────────────
    a("<defs>")
    a('<linearGradient id="pvVortex" x1="0" y1="1" x2="0.55" y2="0">'
      f'<stop offset="0" stop-color="{PALE}"/><stop offset="0.42" stop-color="#7CB0FF"/>'
      f'<stop offset="1" stop-color="{VIOLET}"/></linearGradient>')
    a('<radialGradient id="pvGlow" cx="50%" cy="50%" r="50%">'
      f'<stop offset="0" stop-color="{BLUE}" stop-opacity="0.17"/>'
      f'<stop offset="60%" stop-color="{BLUE}" stop-opacity="0.045"/>'
      f'<stop offset="100%" stop-color="{BLUE}" stop-opacity="0"/></radialGradient>')
    a('<linearGradient id="pvSheet" x1="0" y1="0" x2="1" y2="0.35">'
      f'<stop offset="0" stop-color="{BLUE}" stop-opacity="0.20"/>'
      f'<stop offset="100%" stop-color="{VIOLET}" stop-opacity="0.20"/></linearGradient>')
    a('<marker id="aIn" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" '
      f'orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 Z" fill="{BLUE}"/></marker>')
    a('<marker id="aOut" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" '
      f'orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 Z" fill="{VIOLET}"/></marker>')
    a("</defs>")

    a(f'<rect width="{W}" height="{H}" fill="{BG}"/>')
    a(f'<ellipse cx="{BASIN_CX}" cy="430" rx="330" ry="270" fill="url(#pvGlow)"/>')

    # ── heading ─────────────────────────────────────────────────────────────
    a(f'<text x="{W/2}" y="46" text-anchor="middle" font-size="11" letter-spacing="3.4" '
      f'fill="{FAINT}">HOW IT WORKS</text>')
    a(f'<text x="{W/2}" y="85" text-anchor="middle" font-size="28" font-weight="700" '
      f'fill="{INK}">Put a memory in. Draw it out from any agent.</text>')
    a(f'<text x="{W/2}" y="114" text-anchor="middle" font-size="14" fill="{DIM}">'
      'Your sessions, your code, and what your systems actually did — one basin every '
      'coding agent can reach.</text>')

    for cx, label in ((154, "WHAT GOES IN"), (520, "THE PENSIEVE"), (886, "WHO DRAWS FROM IT")):
        a(f'<text x="{cx}" y="162" text-anchor="middle" font-size="10.5" letter-spacing="2.6" '
          f'fill="{FAINT}">{label}</text>')

    # ── deposit beams — drawn first so cards and sheets sit over them ───────
    # They run down the clear x≈300-370 channel, left of the sheets at x=386.
    for i in range(3):
        y = 234 + i * 96
        a(f'<path d="M 272 {y} C 330 {y} 344 {520 - i * 8} {BASIN_CX - BASIN_RX + 22} '
          f'{BASIN_CY - 20}" fill="none" stroke="{BLUE}" stroke-opacity="0.55" '
          'stroke-width="1.7" marker-end="url(#aIn)"/>')

    # ── recall beams — the x≈660-768 channel, right of the sheets ───────────
    for i in range(4):
        sy = 246 + i * 58
        ay = 211 + i * 82
        a(f'<path d="M {SHEET_X + SHEET_W + 6} {sy} C 712 {sy} 724 {ay} 768 {ay}" fill="none" '
          f'stroke="{VIOLET}" stroke-opacity="0.55" stroke-width="1.7" marker-end="url(#aOut)"/>')

    # ── left: deposits ──────────────────────────────────────────────────────
    for i, (t, s, c) in enumerate([
        ("Your sessions", fits("decisions · corrections"), BLUE),
        ("Your code", fits("repos · files · structure"), CYAN),
        ("Your systems", fits("logs · traces · deploys"), VIOLET),
    ]):
        a(card(36, 196 + i * 96, CARD_W, 76, t, s, c))

    # ── centre: vortex behind, sheets in front ──────────────────────────────
    a(f'<path d="{vortex_path()}" fill="none" stroke="url(#pvVortex)" stroke-width="5.5" '
      'stroke-linecap="round" stroke-linejoin="round" opacity="0.92"/>')

    for y, t, s in SHEETS:
        a(f'<rect x="{SHEET_X}" y="{y}" width="{SHEET_W}" height="60" rx="11" fill="{PANEL_HI}"/>')
        a(f'<rect x="{SHEET_X}" y="{y}" width="{SHEET_W}" height="60" rx="11" fill="url(#pvSheet)" '
          f'stroke="{BLUE}" stroke-opacity="0.5"/>')
        a(f'<text x="{BASIN_CX}" y="{y + 25}" text-anchor="middle" font-size="15" '
          f'font-weight="650" fill="{INK}">{t}</text>')
        a(f'<text x="{BASIN_CX}" y="{y + 45}" text-anchor="middle" font-size="11" '
          f'font-family="ui-monospace,SFMono-Regular,Menlo,monospace" fill="{DIM}">{s}</text>')

    # ── the basin: rim with depth, then walls ───────────────────────────────
    bowl_bottom = BASIN_CY + BOWL_DEPTH
    assert bowl_bottom + 8 < FOOTER_RULE_Y, "bowl would collide with the footer rule"
    a(f'<path d="M {BASIN_CX - BASIN_RX} {BASIN_CY} '
      f'C {BASIN_CX - BASIN_RX + 7} {bowl_bottom - 20} {BASIN_CX - 62} {bowl_bottom} '
      f'{BASIN_CX} {bowl_bottom} '
      f'C {BASIN_CX + 62} {bowl_bottom} {BASIN_CX + BASIN_RX - 7} {bowl_bottom - 20} '
      f'{BASIN_CX + BASIN_RX} {BASIN_CY} Z" fill="{PANEL}" stroke="{BLUE}" stroke-width="4" '
      'stroke-linejoin="round"/>')
    a(f'<ellipse cx="{BASIN_CX}" cy="{BASIN_CY}" rx="{BASIN_RX}" ry="{BASIN_RY}" '
      f'fill="{BG}" stroke="{BLUE}" stroke-width="4"/>')
    a(f'<ellipse cx="{BASIN_CX}" cy="{BASIN_CY}" rx="{BASIN_RX - 17}" ry="{BASIN_RY - 8}" '
      f'fill="none" stroke="{BLUE}" stroke-opacity="0.35" stroke-width="1.5"/>')
    # Surface swirl — two offset partial ellipses inside the rim. Reads as the
    # memory turning in the basin, and unlike a second full spiral it stays
    # inside the composition instead of trailing off into empty canvas.
    for rx, ry, dx, op in ((92, 15, -10, 0.55), (58, 9, 8, 0.38)):
        a(f'<path d="M {BASIN_CX - rx + dx} {BASIN_CY + 2} '
          f'a {rx} {ry} 0 1 0 {2 * rx} 0" fill="none" stroke="{PALE}" '
          f'stroke-opacity="{op}" stroke-width="2" stroke-linecap="round"/>')
    a(f'<text x="{BASIN_CX}" y="{BASIN_CY + 62}" text-anchor="middle" font-size="15" '
      f'font-weight="700" fill="{PALE}" letter-spacing="0.8">pensieve</text>')

    # ── right: the agents ───────────────────────────────────────────────────
    cursor = (f'<path d="M4 3.6 20 12 13.2 13.6 10.8 20.4Z" fill="none" stroke="{CYAN}" '
              'stroke-width="1.8" stroke-linejoin="round"/>')
    mcp = (f'<circle cx="12" cy="12" r="8.4" fill="none" stroke="{PALE}" stroke-width="1.7"/>'
           f'<path d="M12 3.6v16.8M3.6 12h16.8" stroke="{PALE}" stroke-width="1.3" opacity="0.55"/>')
    for i, (label, glyph) in enumerate([
        ("Claude Code", vendor_glyph("anthropic")),
        ("Codex", vendor_glyph("openai", recolour=INK)),
        ("Cursor · Windsurf", cursor),
        ("Any MCP client", mcp),
    ]):
        a(agent_card(768, 180 + i * 82, 236, 62, label, glyph))

    # ── footer ──────────────────────────────────────────────────────────────
    # Two anchored texts rather than tspans: runs of spaces inside a tspan get
    # collapsed, which ran "MCP" straight into "out".
    a(f'<line x1="36" y1="{FOOTER_RULE_Y}" x2="{W - 36}" y2="{FOOTER_RULE_Y}" stroke="{RULE}"/>')
    fy = FOOTER_RULE_Y + 28
    mono = 'font-family="ui-monospace,SFMono-Regular,Menlo,monospace"'
    a(f'<text x="{BASIN_CX - 24}" y="{fy}" text-anchor="end" font-size="12.5" {mono} fill="{DIM}">'
      f'<tspan fill="{BLUE}" font-weight="700">in</tspan>'
      '<tspan> ▸ a plugin · the CLI · MCP</tspan></text>')
    a(f'<text x="{BASIN_CX + 24}" y="{fy}" text-anchor="start" font-size="12.5" {mono} fill="{DIM}">'
      f'<tspan fill="{VIOLET}" font-weight="700">out</tspan>'
      '<tspan> ▸ recall · query · traverse the graph</tspan></text>')

    a("</svg>")
    return "\n".join(p) + "\n"


if __name__ == "__main__":
    if not ICON_DIR.is_dir():
        raise SystemExit(f"run from the repo root; {ICON_DIR} not found")
    svg = build()
    for out in ("docs/images/how-it-works.svg", "docs/site/public/diagrams/how-it-works.svg"):
        pathlib.Path(out).write_text(svg)
        print(f"wrote {out}", file=sys.stderr)
