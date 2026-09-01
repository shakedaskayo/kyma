/**
 * Minimap — downsampled whole-graph dot map with the camera viewport
 * rectangle. Click or drag moves the camera. Redraws on sigma afterRender
 * (throttled by rAF coalescing in sigma itself).
 *
 * Camera centering math:
 *   1. Convert minimap canvas coords → graph-space (x, y)
 *   2. graphToViewport to get the screen pixel location
 *   3. viewportToFramedGraph to get the framed-graph coords the camera uses
 *   4. animate the camera to the framed coords (preserving current ratio)
 */
import { useEffect, useRef } from "react";
import type Sigma from "sigma";

const W = 180;
const H = 120;

export function Minimap({ sigma }: { sigma: Sigma | null }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !sigma) return;
    const graph = sigma.getGraph();

    // Graph-space extent (positions are static — compute once per graph).
    let minX = Infinity, maxX = -Infinity, minY = Infinity, maxY = -Infinity;
    graph.forEachNode((_, a) => {
      minX = Math.min(minX, a.x as number); maxX = Math.max(maxX, a.x as number);
      minY = Math.min(minY, a.y as number); maxY = Math.max(maxY, a.y as number);
    });
    const spanX = Math.max(maxX - minX, 1e-6);
    const spanY = Math.max(maxY - minY, 1e-6);
    const toMini = (x: number, y: number) => ({
      x: ((x - minX) / spanX) * (W - 8) + 4,
      y: ((y - minY) / spanY) * (H - 8) + 4,
    });

    const draw = () => {
      const ctx = canvas.getContext("2d");
      if (!ctx) return;
      const dpr = window.devicePixelRatio;
      canvas.width = W * dpr;
      canvas.height = H * dpr;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, W, H);
      ctx.fillStyle = "rgba(148,163,184,0.55)";
      const step = Math.max(1, Math.floor(graph.order / 1500)); // ≤1500 dots
      let i = 0;
      graph.forEachNode((_, a) => {
        if (i++ % step !== 0) return;
        const p = toMini(a.x as number, a.y as number);
        ctx.fillRect(p.x, p.y, 1.5, 1.5);
      });
      // Viewport rect: corners of the screen mapped into graph space.
      const { width: sw, height: sh } = sigma.getDimensions();
      const tl = sigma.viewportToGraph({ x: 0, y: 0 });
      const br = sigma.viewportToGraph({ x: sw, y: sh });
      const a = toMini(tl.x, tl.y);
      const b = toMini(br.x, br.y);
      ctx.strokeStyle = "#22d3ee";
      ctx.lineWidth = 1.2;
      ctx.strokeRect(a.x, a.y, b.x - a.x, b.y - a.y);
    };

    const moveCamera = (clientX: number, clientY: number) => {
      const rect = canvas.getBoundingClientRect();
      // Convert canvas pixel → graph-space
      const gx = ((clientX - rect.left - 4) / (W - 8)) * spanX + minX;
      const gy = ((clientY - rect.top - 4) / (H - 8)) * spanY + minY;
      // graph → viewport → framed-graph (what the camera's x/y refers to)
      const vp = sigma.graphToViewport({ x: gx, y: gy });
      const framed = sigma.viewportToFramedGraph(vp);
      const camState = sigma.getCamera().getState();
      sigma.getCamera().animate({ ...camState, x: framed.x, y: framed.y }, { duration: 150 });
    };

    let dragging = false;
    const down = (e: MouseEvent) => { dragging = true; moveCamera(e.clientX, e.clientY); };
    const move = (e: MouseEvent) => { if (dragging) moveCamera(e.clientX, e.clientY); };
    const up = () => { dragging = false; };
    canvas.addEventListener("mousedown", down);
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
    draw();
    sigma.on("afterRender", draw);
    return () => {
      sigma.off("afterRender", draw);
      canvas.removeEventListener("mousedown", down);
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
  }, [sigma]);

  return (
    <div className="pv-absolute pv-bottom-4 pv-left-4 pv-z-20 pv-rounded-lg pv-glass pv-border pv-border-border pv-p-1">
      <canvas ref={canvasRef} style={{ width: W, height: H }} className="pv-cursor-crosshair pv-rounded" />
    </div>
  );
}
