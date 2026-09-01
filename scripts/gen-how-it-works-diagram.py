#!/usr/bin/env python3
"""gen-how-it-works-diagram.py — draw docs/images/how-it-works.svg.

architecture.svg is the internals schematic — two lanes, the stateless spine,
the five invariants. It is accurate and worth keeping, but it explains the
engine rather than the product, so it is the wrong thing to lead with.

This is the product diagram that leads instead. It tells the Pensieve story:
your code, your systems and your team's chatter go into the basin; memories
rise out of it; your coding agents draw from them — and every session those
agents run flows straight back in. That return arc is the whole point, so it
is drawn rather than described.

Generated rather than hand-authored for two reasons. The vortex is a computed
spiral — the mark's own, scaled up. And every vendor glyph is lifted out of
docs/site/public/icons/brand/*.svg at build time, so the marks stay exactly as
authored instead of being hand-copied and drifting.

Layout rules, asserted rather than eyeballed, because early drafts broke all
three: sub-labels may not overflow their card, the bowl must clear the footer
rule, and beams keep to channels that avoid text.

Usage: python3 scripts/gen-how-it-works-diagram.py
"""

from __future__ import annotations

import math
import pathlib
import re
import sys

W, H = 1080, 812

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
AMBER = "#F2AE17"     # the return loop, used once so it reads as its own idea

BASIN_CX, BASIN_CY = 540, 596
BASIN_RX, BASIN_RY = 138, 32
BOWL_DEPTH = 86
FOOTER_RULE_Y = 744

CARD_X, CARD_W, CARD_PAD = 36, 250, 18
AGENT_X, AGENT_W = 794, 250
SHEET_X, SHEET_W = 402, 276

MONO_ADVANCE = 6.6  # 11px ui-monospace advance, measured

ICON_DIR = pathlib.Path("docs/site/public/icons/brand")

_ID = re.compile(r'\bid="([^"]+)"')


def glyph(name: str, size: float, tx: float, ty: float, tint: str | None = None) -> str:
    """Inline a brand icon at `size` px, top-left at (tx, ty).

    General on purpose: these icons are not uniform. viewBoxes run 24x24,
    48x48, 180x180 and 304x182, and the bodies contain paths, circles, rects,
    lines, groups and masks. Assuming "one 24x24 path" silently mangles most of
    them, so read the viewBox and scale from it, and carry the whole body.

    Internal ids are namespaced per instance: two icons inlined into one
    document would otherwise collide on `id="mask0_..."` and the second would
    render through the first's mask.
    """
    src = (ICON_DIR / f"{name}.svg").read_text()

    vb = re.search(r'viewBox="([-\d.]+)\s+([-\d.]+)\s+([-\d.]+)\s+([-\d.]+)"', src)
    if not vb:
        raise SystemExit(f"{name}.svg has no parsable viewBox; cannot place it accurately")
    minx, miny, vw, vh = (float(g) for g in vb.groups())

    body = src[src.index(">", src.index("<svg")) + 1: src.rindex("</svg>")]
    body = re.sub(r"<title>.*?</title>", "", body, flags=re.S).strip()

    # Namespace every internal id, and every reference to one.
    for ident in set(_ID.findall(body)):
        safe = f"{name}_{ident}"
        body = body.replace(f'id="{ident}"', f'id="{safe}"')
        body = body.replace(f"url(#{ident})", f"url(#{safe})")
        body = body.replace(f'href="#{ident}"', f'href="#{safe}"')

    if tint:
        body = re.sub(r'fill="(?!none)[^"]*"', f'fill="{tint}"', body)

    s = size / max(vw, vh)
    # Centre the shorter axis inside the square box.
    ox = (size - vw * s) / 2
    oy = (size - vh * s) / 2
    return (f'<g transform="translate({tx + ox:.2f} {ty + oy:.2f}) scale({s:.4f}) '
            f'translate({-minx} {-miny})">{body}</g>')


def vortex_path(y_top: float = 250.0) -> str:
    """The mark's spiral, scaled to rise out of the basin."""
    y0 = BASIN_CY - 20
    turns, steps, r0, r1 = 1.55, 260, 108.0, 13.0
    pts = []
    for i in range(steps + 1):
        t = i / steps
        th = math.pi / 2 + turns * 2 * math.pi * t
        r = r0 + (r1 - r0) * (t ** 0.78)
        yc = y0 - (y0 - y_top) * (t ** 1.12)
        pts.append((BASIN_CX + r * math.cos(th), yc + r * 0.28 * math.sin(th)))
    d = "M %.1f %.1f" % pts[0]
    for x, y in pts[1::4]:
        d += " L %.1f %.1f" % (x, y)
    return d + " L %.1f %.1f" % pts[-1]


def fits(sub: str, width: int) -> str:
    if len(sub) * MONO_ADVANCE > width - 2 * CARD_PAD:
        raise SystemExit(
            f"sub-label overflows its card ({len(sub)} chars, max "
            f"{int((width - 2 * CARD_PAD) / MONO_ADVANCE)}): {sub!r}")
    return sub


def source_card(y, title, sub, accent, icons):
    h = 92
    out = [f'<rect x="{CARD_X}" y="{y}" width="{CARD_W}" height="{h}" rx="12" fill="{PANEL}" '
           f'stroke="{RULE}"/>',
           f'<rect x="{CARD_X}" y="{y + 14}" width="3" height="{h - 28}" rx="1.5" fill="{accent}"/>',
           f'<text x="{CARD_X + 18}" y="{y + 28}" font-size="15" font-weight="650" '
           f'fill="{INK}">{title}</text>',
           f'<text x="{CARD_X + 18}" y="{y + 48}" font-size="11" '
           f'font-family="ui-monospace,SFMono-Regular,Menlo,monospace" fill="{DIM}">{sub}</text>']
    for i, ic in enumerate(icons):
        out.append(glyph(ic, 21, CARD_X + 18 + i * 29, y + 60))
    return "\n  ".join(out)


def agent_card(y, label, inner):
    h = 64
    return "\n  ".join([
        f'<rect x="{AGENT_X}" y="{y}" width="{AGENT_W}" height="{h}" rx="12" fill="{PANEL}" '
        f'stroke="{RULE}"/>',
        inner,
        f'<text x="{AGENT_X + 58}" y="{y + h / 2 + 5}" font-size="14.5" font-weight="600" '
        f'fill="{INK}">{label}</text>',
    ])


def build() -> str:
    p = []
    a = p.append

    a(f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" role="img" '
      'aria-label="How Pensieve works: your code, systems and team chatter flow into the '
      'basin; memories, live data and the graph rise out of it; Claude Code, Codex, Cursor '
      'and any MCP client draw from them, and every session flows back in">')

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
    for mid, col in (("aIn", BLUE), ("aOut", VIOLET), ("aBack", AMBER)):
        a(f'<marker id="{mid}" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" '
          f'markerHeight="7" orient="auto-start-reverse">'
          f'<path d="M0,0 L10,5 L0,10 Z" fill="{col}"/></marker>')
    a("</defs>")

    a(f'<rect width="{W}" height="{H}" fill="{BG}"/>')
    a(f'<ellipse cx="{BASIN_CX}" cy="450" rx="350" ry="290" fill="url(#pvGlow)"/>')

    # ── heading ─────────────────────────────────────────────────────────────
    a(f'<text x="{W/2}" y="46" text-anchor="middle" font-size="11" letter-spacing="3.4" '
      f'fill="{FAINT}">HOW IT WORKS</text>')
    a(f'<text x="{W/2}" y="85" text-anchor="middle" font-size="28" font-weight="700" '
      f'fill="{INK}">Put a memory in. Draw it out from any agent.</text>')
    a(f'<text x="{W/2}" y="114" text-anchor="middle" font-size="14" fill="{DIM}">'
      'Everything your stack and your team know — in one basin every coding agent can reach.</text>')

    for cx, label in ((CARD_X + CARD_W / 2, "WHAT GOES IN"),
                      (BASIN_CX, "THE PENSIEVE"),
                      (AGENT_X + AGENT_W / 2, "WHO DRAWS FROM IT")):
        a(f'<text x="{cx}" y="162" text-anchor="middle" font-size="10.5" letter-spacing="2.6" '
          f'fill="{FAINT}">{label}</text>')

    # ── beams first, so cards and sheets sit over them ──────────────────────
    for i in range(3):
        y = 242 + i * 112
        a(f'<path d="M {CARD_X + CARD_W + 4} {y} C 348 {y} 358 {536 - i * 8} '
          f'{BASIN_CX - BASIN_RX + 24} {BASIN_CY - 22}" fill="none" stroke="{BLUE}" '
          'stroke-opacity="0.55" stroke-width="1.7" marker-end="url(#aIn)"/>')

    for i in range(4):
        sy = 262 + i * 58
        ay = 213 + i * 84
        a(f'<path d="M {SHEET_X + SHEET_W + 6} {sy} C 740 {sy} 752 {ay} {AGENT_X - 4} {ay}" '
          f'fill="none" stroke="{VIOLET}" stroke-opacity="0.55" stroke-width="1.7" '
          'marker-end="url(#aOut)"/>')

    # The return loop: sessions the agents run flow straight back into the basin.
    # It must LEAVE something — anchored to the bottom edge of the last agent
    # card, not floating in the gap below the column, or it reads as a stray
    # dashed line rather than a connection.
    agents_bottom = 182 + 3 * 84 + 64
    a(f'<path d="M {AGENT_X + AGENT_W / 2} {agents_bottom} '
      f'C {AGENT_X + AGENT_W / 2 + 26} {agents_bottom + 128} '
      f'{BASIN_CX + 232} {agents_bottom + 196} '
      f'{BASIN_CX + BASIN_RX - 30} {BASIN_CY + 34}" fill="none" '
      f'stroke="{AMBER}" stroke-opacity="0.65" stroke-width="1.8" stroke-dasharray="5 4" '
      'marker-end="url(#aBack)"/>')
    a(f'<circle cx="{AGENT_X + AGENT_W / 2}" cy="{agents_bottom}" r="3.2" fill="{AMBER}"/>')
    a(f'<text x="{BASIN_CX + 250}" y="{agents_bottom + 186}" text-anchor="middle" '
      f'font-size="11.5" font-family="ui-monospace,SFMono-Regular,Menlo,monospace" '
      f'fill="{AMBER}" fill-opacity="0.95">every session flows back in</text>')

    # ── left: the sources, with the marks that actually feed them ───────────
    for i, (t, s, c, icons) in enumerate([
        ("Your code", fits("repos · files · CI", CARD_W), BLUE, ["github", "jenkins"]),
        ("Your systems", fits("logs · traces · deploys", CARD_W), CYAN,
         ["kubernetes", "postgresql", "datadog", "otel", "kafka"]),
        ("Your team", fits("chatter · alerts · errors", CARD_W), VIOLET,
         ["slack", "pagerduty", "sentry"]),
    ]):
        a(source_card(196 + i * 112, t, s, c, icons))

    # ── centre: vortex behind, sheets in front ──────────────────────────────
    a(f'<path d="{vortex_path()}" fill="none" stroke="url(#pvVortex)" stroke-width="5.5" '
      'stroke-linecap="round" stroke-linejoin="round" opacity="0.92"/>')

    sheet_glyphs = {
        "Memories": f'<circle cx="10" cy="10" r="7" fill="none" stroke="{PALE}" stroke-width="1.8"/>'
                    f'<circle cx="10" cy="10" r="2.4" fill="{PALE}"/>',
        "Live data": f'<path d="M2 16 L7 9 L11 13 L18 4" fill="none" stroke="{CYAN}" '
                     'stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"/>',
        "The graph": f'<circle cx="4" cy="5" r="2.6" fill="{VIOLET}"/>'
                     f'<circle cx="16" cy="4" r="2.6" fill="{VIOLET}"/>'
                     f'<circle cx="10" cy="16" r="2.6" fill="{VIOLET}"/>'
                     f'<path d="M4 5 L16 4 M4 5 L10 16 M16 4 L10 16" stroke="{VIOLET}" '
                     'stroke-width="1.4" opacity="0.7"/>',
    }
    for y, t, s in [
        (228, "Memories", "facts · decisions · preferences"),
        (314, "Live data", "logs · traces · code — KQL or SQL"),
        (400, "The graph", "how every one of them connects"),
    ]:
        a(f'<rect x="{SHEET_X}" y="{y}" width="{SHEET_W}" height="62" rx="12" fill="{PANEL_HI}"/>')
        a(f'<rect x="{SHEET_X}" y="{y}" width="{SHEET_W}" height="62" rx="12" fill="url(#pvSheet)" '
          f'stroke="{BLUE}" stroke-opacity="0.5"/>')
        a(f'<g transform="translate({SHEET_X + 18} {y + 14})">{sheet_glyphs[t]}</g>')
        a(f'<text x="{SHEET_X + 50}" y="{y + 26}" font-size="15" font-weight="650" '
          f'fill="{INK}">{t}</text>')
        a(f'<text x="{SHEET_X + 50}" y="{y + 46}" font-size="11" '
          f'font-family="ui-monospace,SFMono-Regular,Menlo,monospace" fill="{DIM}">{s}</text>')

    # ── the basin ───────────────────────────────────────────────────────────
    bowl_bottom = BASIN_CY + BOWL_DEPTH
    assert bowl_bottom + 8 < FOOTER_RULE_Y, "bowl would collide with the footer rule"
    a(f'<path d="M {BASIN_CX - BASIN_RX} {BASIN_CY} '
      f'C {BASIN_CX - BASIN_RX + 7} {bowl_bottom - 20} {BASIN_CX - 64} {bowl_bottom} '
      f'{BASIN_CX} {bowl_bottom} '
      f'C {BASIN_CX + 64} {bowl_bottom} {BASIN_CX + BASIN_RX - 7} {bowl_bottom - 20} '
      f'{BASIN_CX + BASIN_RX} {BASIN_CY} Z" fill="{PANEL}" stroke="{BLUE}" stroke-width="4" '
      'stroke-linejoin="round"/>')
    a(f'<ellipse cx="{BASIN_CX}" cy="{BASIN_CY}" rx="{BASIN_RX}" ry="{BASIN_RY}" '
      f'fill="{BG}" stroke="{BLUE}" stroke-width="4"/>')
    a(f'<ellipse cx="{BASIN_CX}" cy="{BASIN_CY}" rx="{BASIN_RX - 17}" ry="{BASIN_RY - 8}" '
      f'fill="none" stroke="{BLUE}" stroke-opacity="0.35" stroke-width="1.5"/>')
    for rx, ry, dx, op in ((94, 15, -10, 0.55), (60, 9, 8, 0.38)):
        a(f'<path d="M {BASIN_CX - rx + dx} {BASIN_CY + 2} a {rx} {ry} 0 1 0 {2 * rx} 0" '
          f'fill="none" stroke="{PALE}" stroke-opacity="{op}" stroke-width="2" '
          'stroke-linecap="round"/>')
    a(f'<text x="{BASIN_CX}" y="{BASIN_CY + 62}" text-anchor="middle" font-size="15" '
      f'font-weight="700" fill="{PALE}" letter-spacing="0.8">pensieve</text>')

    # ── right: the agents ───────────────────────────────────────────────────
    cursor = (f'<g transform="translate({AGENT_X + 18} {0})">'
              f'<path d="M4 3.6 20 12 13.2 13.6 10.8 20.4Z" fill="none" stroke="{CYAN}" '
              'stroke-width="1.8" stroke-linejoin="round"/></g>')
    for i, (label, kind) in enumerate([
        ("Claude Code", "anthropic"),
        ("Codex", "openai"),
        ("Cursor · Windsurf", "cursor"),
        ("Any MCP client", "mcp"),
    ]):
        y = 182 + i * 84
        if kind == "cursor":
            inner = cursor.replace("translate(%d 0)" % (AGENT_X + 18),
                                   f"translate({AGENT_X + 18} {y + 20})")
            inner = (f'<g transform="translate({AGENT_X + 18} {y + 20})">'
                     f'<path d="M4 3.6 20 12 13.2 13.6 10.8 20.4Z" fill="none" stroke="{CYAN}" '
                     'stroke-width="1.8" stroke-linejoin="round"/></g>')
        elif kind == "openai":
            inner = glyph("openai", 24, AGENT_X + 18, y + 20, tint=INK)
        else:
            inner = glyph(kind, 24, AGENT_X + 18, y + 20)
        a(agent_card(y, label, inner))

    # ── footer ──────────────────────────────────────────────────────────────
    # Two anchored texts, not tspans: runs of spaces inside a tspan collapse.
    a(f'<line x1="36" y1="{FOOTER_RULE_Y}" x2="{W - 36}" y2="{FOOTER_RULE_Y}" stroke="{RULE}"/>')
    fy = FOOTER_RULE_Y + 30
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
