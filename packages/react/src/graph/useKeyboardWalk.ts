/**
 * Keyboard edge-walking: with a node selected, Tab / ArrowRight cycles its
 * neighbors (highlighting the candidate via hoverNode), Shift+Tab / ArrowLeft
 * cycles backwards, Enter selects + flies to the candidate, Esc steps back
 * along the trail. Inactive while the command bar is open or focus is in an
 * input.
 */
import { useEffect, useRef } from "react";
import type Graph from "graphology";
import { useGraphStore } from "./graph-store";
import { sortedNeighbors } from "./graph-walk";

export function useKeyboardWalk(graphRef: React.RefObject<Graph | null>) {
  const selectedNodeId = useGraphStore((s) => s.selectedNodeId);
  const commandBarOpen = useGraphStore((s) => s.commandBarOpen);
  const hoverNode = useGraphStore((s) => s.hoverNode);
  const focusNode = useGraphStore((s) => s.focusNode);
  const pushTrail = useGraphStore((s) => s.pushTrail);
  const jumpTrail = useGraphStore((s) => s.jumpTrail);
  const trail = useGraphStore((s) => s.trail);
  const idx = useRef(-1);

  useEffect(() => {
    idx.current = -1; // reset cycle on selection change
  }, [selectedNodeId]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (commandBarOpen) return;
      const t = e.target as HTMLElement | null;
      if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable)) return;
      const graph = graphRef.current;
      if (!graph || !selectedNodeId) return;

      if (e.key === "Tab" || e.key === "ArrowRight" || e.key === "ArrowLeft") {
        e.preventDefault();
        const candidates = sortedNeighbors(graph, selectedNodeId);
        if (candidates.length === 0) return;
        const dir = e.key === "ArrowLeft" || (e.key === "Tab" && e.shiftKey) ? -1 : 1;
        idx.current = (idx.current + dir + candidates.length) % candidates.length;
        hoverNode(candidates[idx.current].nodeId); // lights up candidate + edge
      } else if (e.key === "Enter" && idx.current >= 0) {
        e.preventDefault();
        const candidates = sortedNeighbors(graph, selectedNodeId);
        const target = candidates[idx.current]?.nodeId;
        if (target) {
          hoverNode(null);
          pushTrail(target);
          focusNode(target); // selects + flies (focusSeq)
        }
      } else if (e.key === "Escape" && trail.length > 1) {
        jumpTrail(trail.length - 2);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [graphRef, selectedNodeId, commandBarOpen, trail, hoverNode, focusNode, pushTrail, jumpTrail]);
}
