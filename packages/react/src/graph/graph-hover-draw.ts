/**
 * Theme-aware canvas drawing for sigma hover state. Sigma's default hover
 * renderer paints an opaque WHITE label box — blinding on the dark galaxy
 * canvas. This replaces it with a glass pill + a soft focus ring, tuned per
 * theme.
 */
import type { Attributes } from "graphology-types";
import type { NodeHoverDrawingFunction } from "sigma/rendering";

function roundRect(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
  r: number,
): void {
  ctx.beginPath();
  ctx.moveTo(x + r, y);
  ctx.arcTo(x + w, y, x + w, y + h, r);
  ctx.arcTo(x + w, y + h, x, y + h, r);
  ctx.arcTo(x, y + h, x, y, r);
  ctx.arcTo(x, y, x + w, y, r);
  ctx.closePath();
}

export function makeDrawNodeHover(
  isDark: boolean,
): NodeHoverDrawingFunction<Attributes, Attributes, Attributes> {
  return (ctx, data, settings) => {
    // Guard sigma's shared 2D context — we mutate font/fill/stroke state.
    ctx.save();
    // Soft focus ring around the node.
    ctx.beginPath();
    ctx.arc(data.x, data.y, data.size + 3, 0, Math.PI * 2);
    ctx.strokeStyle = isDark ? "rgba(226,232,240,0.8)" : "rgba(15,23,42,0.55)";
    ctx.lineWidth = 1.5;
    ctx.stroke();

    if (!data.label) {
      ctx.restore();
      return;
    }
    const fontSize = settings.labelSize;
    ctx.font = `500 ${fontSize}px ${settings.labelFont}`;
    const textW = ctx.measureText(data.label).width;
    const padX = 8;
    const pillW = textW + padX * 2;
    const pillH = fontSize + 10;
    const pillX = data.x + data.size + 7;
    const pillY = data.y - pillH / 2;

    roundRect(ctx, pillX, pillY, pillW, pillH, pillH / 2);
    ctx.fillStyle = isDark ? "rgba(9,14,26,0.88)" : "rgba(255,255,255,0.92)";
    ctx.fill();
    ctx.strokeStyle = isDark ? "rgba(148,163,184,0.35)" : "rgba(100,116,139,0.28)";
    ctx.lineWidth = 1;
    ctx.stroke();

    ctx.fillStyle = isDark ? "#e2e8f0" : "#0f172a";
    ctx.fillText(data.label, pillX + padX, data.y + fontSize * 0.34);
    ctx.restore();
  };
}
