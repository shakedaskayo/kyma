/**
 * ConsumerBeamsLayer — animated overlay that draws live consumers as nodes on
 * the canvas rim with beams to the memory/data nodes they read or write.
 *
 * Same compositing technique as HullsLayer/Minimap: a 2D canvas pinned over the
 * WebGL surface, projecting graph coords with `sigma.graphToViewport` so beams
 * track camera moves. Unlike the hulls it animates, so it also runs a
 * requestAnimationFrame loop while beams are in flight (self-stopping at rest).
 *
 * Beams carry a RAW node id (`memory:<uuid>` / resource id); the loaded graph
 * keys nodes by `${namespace}::${id}`, so we resolve raw → composite once per
 * graph change and skip beams whose node isn't on the canvas (it still shows in
 * the dock). WebGL-only — the canvas fallback degrades to the dock alone.
 */
import { useEffect, useRef } from "react";
import type Sigma from "sigma";
import { useGraphStore } from "./graph-store";
import { usePensieveContext } from "../provider/context";
import { consumerColor } from "./consumer-style";
import type { ConsumerBeam, LiveConsumer } from "./consumer-types";

type Pt = { x: number; y: number };

// ── pure helpers (exported for tests) ───────────────────────────────────────

/** Deterministic angle (radians) for a consumer's rim anchor from its id, so a
 *  consumer always emanates from the same point. */
/**
 * Anchor active consumers in a tidy vertical column near the LEFT edge — clear
 * of every chrome zone (top-centre command bar, bottom-centre legend, the
 * minimap + zoom in the bottom corners, the dock on the right). Deterministic
 * (sorted by id) so a consumer keeps its slot frame-to-frame; beams reach
 * rightward into the graph from there. Returns a stable per-id position map.
 */
export function computeAnchors(ids: string[], w: number, h: number): Map<string, Pt> {
  const m = new Map<string, Pt>();
  const sorted = [...ids].sort();
  const n = sorted.length;
  const x = Math.min(76, w * 0.09);
  const top = h * 0.24;
  const bottom = h * 0.76;
  const span = bottom - top;
  sorted.forEach((id, i) => {
    const y = n <= 1 ? h * 0.34 : top + (span * i) / (n - 1);
    m.set(id, { x, y });
  });
  return m;
}

function hexA(hex: string, alpha: number): string {
  const n = parseInt(hex.slice(1), 16);
  return `rgba(${(n >> 16) & 255},${(n >> 8) & 255},${n & 255},${alpha})`;
}

const easeOut = (t: number) => 1 - Math.pow(1 - t, 3);

// ── component ────────────────────────────────────────────────────────────────

export function ConsumerBeamsLayer({ sigma }: { sigma: Sigma | null }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const beams = useGraphStore((s) => s.consumerBeams);
  const consumers = useGraphStore((s) => s.liveConsumers);
  const hoveredConsumerId = useGraphStore((s) => s.hoveredConsumerId);
  const pinnedConsumerId = useGraphStore((s) => s.pinnedConsumerId);
  const { isDark } = usePensieveContext();

  // Live values for the animation loop, refreshed every render so the loop
  // never needs restarting when the data ticks.
  const beamsRef = useRef(beams);
  beamsRef.current = beams;
  const consumersRef = useRef(consumers);
  consumersRef.current = consumers;
  const hoveredRef = useRef(hoveredConsumerId);
  hoveredRef.current = hoveredConsumerId;
  const pinnedRef = useRef(pinnedConsumerId);
  pinnedRef.current = pinnedConsumerId;
  const isDarkRef = useRef(isDark);
  isDarkRef.current = isDark;

  // raw-id → composite node key, rebuilt only when the graph's node count moves.
  const keyByRaw = useRef<Map<string, string>>(new Map());
  const lastOrder = useRef(-1);
  const rafRef = useRef<number | null>(null);
  const drawRef = useRef<(() => number) | null>(null);
  const tickRef = useRef<(() => void) | null>(null);

  useEffect(() => {
    if (!sigma) return;
    const canvas = canvasRef.current;
    if (!canvas) return;

    const draw = () => {
      const { width, height } = sigma.getDimensions();
      const dpr = window.devicePixelRatio || 1;
      canvas.width = width * dpr;
      canvas.height = height * dpr;
      canvas.style.width = `${width}px`;
      canvas.style.height = `${height}px`;
      const ctx = canvas.getContext("2d");
      if (!ctx) return 0;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, width, height);

      const graph = sigma.getGraph();
      // Rebuild the raw→key index only when nodes were added/removed.
      if (graph.order !== lastOrder.current) {
        const m = new Map<string, string>();
        graph.forEachNode((key) => {
          const raw = key.split("::").pop();
          if (raw) m.set(raw, key);
        });
        keyByRaw.current = m;
        lastOrder.current = graph.order;
      }

      const consumerById = new Map<string, LiveConsumer>();
      for (const c of consumersRef.current) consumerById.set(c.id, c);

      // Anchor active (or pinned) consumers in a tidy left-edge column.
      const anchored = consumersRef.current.filter(
        (c) => c.active || c.id === pinnedRef.current,
      );
      const anchors = computeAnchors(
        anchored.map((c) => c.id),
        width,
        height,
      );
      const fallbackAnchor: Pt = { x: Math.min(76, width * 0.09), y: height * 0.12 };

      const now = Date.now();
      const liveBeams: ConsumerBeam[] = [];

      // ── beams + node pulses ────────────────────────────────────────────────
      for (const beam of beamsRef.current) {
        const t = (now - beam.startTs) / beam.ttl;
        if (t >= 1) continue;
        liveBeams.push(beam);
        const key = keyByRaw.current.get(beam.rawNodeId);
        if (!key || !graph.hasNode(key)) continue;
        const attrs = graph.getNodeAttributes(key) as { x: number; y: number };
        const B = sigma.graphToViewport({ x: attrs.x, y: attrs.y });
        const A = anchors.get(beam.consumerId) ?? fallbackAnchor;
        const consumer = consumerById.get(beam.consumerId);
        const color = consumerColor(consumer?.kind ?? "unknown");
        const focused =
          beam.consumerId === hoveredRef.current || beam.consumerId === pinnedRef.current;
        const fade = 1 - easeOut(t);

        // gentle arc: control point = midpoint pulled toward the canvas centre
        const mx = (A.x + B.x) / 2;
        const my = (A.y + B.y) / 2;
        const cx = mx + (width / 2 - mx) * 0.22;
        const cy = my + (height / 2 - my) * 0.22;

        // trail
        ctx.beginPath();
        ctx.moveTo(A.x, A.y);
        ctx.quadraticCurveTo(cx, cy, B.x, B.y);
        ctx.strokeStyle = hexA(color, (focused ? 0.4 : 0.24) * fade);
        ctx.lineWidth = focused ? 2.4 : 1.4;
        ctx.stroke();

        // travelling pulse
        const u = easeOut(t);
        const px = (1 - u) * (1 - u) * A.x + 2 * (1 - u) * u * cx + u * u * B.x;
        const py = (1 - u) * (1 - u) * A.y + 2 * (1 - u) * u * cy + u * u * B.y;
        ctx.beginPath();
        ctx.arc(px, py, focused ? 3.4 : 2.6, 0, Math.PI * 2);
        ctx.fillStyle = hexA(color, 0.95 * fade);
        ctx.fill();

        // expanding ring on the touched node
        ctx.beginPath();
        ctx.arc(B.x, B.y, 6 + easeOut(t) * 16, 0, Math.PI * 2);
        ctx.strokeStyle = hexA(color, 0.5 * fade);
        ctx.lineWidth = 2;
        ctx.stroke();
      }

      // ── consumer markers — the left-edge column (active/pinned only) ────────
      for (const c of anchored) {
        const A = anchors.get(c.id);
        if (!A) continue;
        const color = consumerColor(c.kind);
        const focused = c.id === hoveredRef.current || c.id === pinnedRef.current;
        // soft halo so the marker reads on both themes
        ctx.beginPath();
        ctx.arc(A.x, A.y, focused ? 13 : 10, 0, Math.PI * 2);
        ctx.fillStyle = hexA(color, isDarkRef.current ? 0.16 : 0.12);
        ctx.fill();
        // diamond
        ctx.save();
        ctx.translate(A.x, A.y);
        ctx.rotate(Math.PI / 4);
        const r = focused ? 6 : 5;
        ctx.fillStyle = hexA(color, 0.95);
        ctx.fillRect(-r, -r, r * 2, r * 2);
        ctx.restore();
        // label, always reading rightward into the canvas
        ctx.font = "600 10px ui-sans-serif, system-ui, sans-serif";
        ctx.textAlign = "left";
        ctx.textBaseline = "middle";
        ctx.fillStyle = isDarkRef.current ? hexA(color, 0.95) : hexA(color, 0.9);
        ctx.fillText(c.label.slice(0, 20), A.x + 12, A.y);
      }

      return liveBeams.length;
    };

    const tick = () => {
      const live = draw();
      rafRef.current = live > 0 ? requestAnimationFrame(tick) : null;
    };
    drawRef.current = draw;
    tickRef.current = tick;

    draw();
    if (rafRef.current == null) rafRef.current = requestAnimationFrame(tick);
    sigma.on("afterRender", draw);
    return () => {
      sigma.off("afterRender", draw);
      if (rafRef.current != null) cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
      drawRef.current = null;
      tickRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sigma]);

  // New beams arrived → (re)start the animation loop (it self-stops at rest).
  useEffect(() => {
    if (rafRef.current == null && beams.length > 0 && tickRef.current) {
      rafRef.current = requestAnimationFrame(tickRef.current);
    }
  }, [beams]);

  // Consumer set / hover / pin changed → one redraw of the static rim markers.
  useEffect(() => {
    drawRef.current?.();
  }, [consumers, hoveredConsumerId, pinnedConsumerId]);

  return (
    <canvas
      ref={canvasRef}
      className="pv-pointer-events-none pv-absolute pv-inset-0 pv-z-10"
    />
  );
}
