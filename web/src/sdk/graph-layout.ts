import type { GraphNode, GraphRelationship } from "./graph";

interface LayoutNode {
  id: string;
  x: number;
  y: number;
  vx: number;
  vy: number;
  label: string;
}

export function forceDirectedLayout(
  nodes: GraphNode[],
  relationships: GraphRelationship[],
  width: number,
  height: number,
): Map<string, { x: number; y: number }> {
  if (nodes.length === 0) return new Map();

  // Scale canvas based on node count for better spacing
  const n = nodes.length;
  const scaleFactor = Math.max(1, Math.sqrt(n / 40));
  const canvasW = width * scaleFactor;
  const canvasH = height * scaleFactor;

  // Group nodes by primary label for initial clustering
  const labelGroups = new Map<string, GraphNode[]>();
  nodes.forEach((node) => {
    const label = (node.labels && node.labels[0]) || "Unknown";
    if (!labelGroups.has(label)) labelGroups.set(label, []);
    labelGroups.get(label)!.push(node);
  });
  const labelList = Array.from(labelGroups.keys());

  // Build adjacency for connectivity-aware placement
  const adjacency = new Map<string, Set<string>>();
  relationships.forEach((r) => {
    if (!adjacency.has(r.source_id)) adjacency.set(r.source_id, new Set());
    if (!adjacency.has(r.target_id)) adjacency.set(r.target_id, new Set());
    adjacency.get(r.source_id)!.add(r.target_id);
    adjacency.get(r.target_id)!.add(r.source_id);
  });

  // Place nodes in a circle per label group, with groups spread around the center
  const layoutNodes: LayoutNode[] = nodes.map((node) => {
    const label = (node.labels && node.labels[0]) || "Unknown";
    const group = labelGroups.get(label)!;
    const groupIdx = labelList.indexOf(label);
    const groupCount = group.length;
    const memberIdx = group.indexOf(node);

    // Group angle around center — use golden angle for better distribution
    const groupAngle = (2 * Math.PI * groupIdx) / labelList.length;
    const groupRadius = Math.min(canvasW, canvasH) * 0.3;
    const cx = canvasW / 2 + Math.cos(groupAngle) * groupRadius;
    const cy = canvasH / 2 + Math.sin(groupAngle) * groupRadius;

    // Spread within group — use larger radius for bigger groups
    const memberAngle = (2 * Math.PI * memberIdx) / groupCount;
    const memberRadius = Math.sqrt(groupCount) * 35;

    // Deterministic jitter based on node index to avoid layout flickering
    const jitterSeed = (memberIdx * 7919 + groupIdx * 104729) % 1000;
    const jx = ((jitterSeed / 1000) - 0.5) * 15;
    const jy = (((jitterSeed * 3) % 1000) / 1000 - 0.5) * 15;

    return {
      id: node.id,
      x: cx + Math.cos(memberAngle) * memberRadius + jx,
      y: cy + Math.sin(memberAngle) * memberRadius + jy,
      vx: 0,
      vy: 0,
      label,
    };
  });

  const nodeMap = new Map(layoutNodes.map((ln) => [ln.id, ln]));

  // Scale forces based on node count — bigger graphs need stronger repulsion
  const ITERATIONS = Math.min(150, 60 + Math.floor(n * 0.5));
  const REPULSION = Math.max(4000, 10000 * Math.sqrt(60 / Math.max(n, 1)));
  const ATTRACTION = 0.006;
  const DAMPING = 0.85;
  const MIN_DIST = 65;
  const CENTER_GRAVITY = 0.008;
  const LABEL_COHESION = 0.004;

  for (let iter = 0; iter < ITERATIONS; iter++) {
    const temp = 1 - iter / ITERATIONS;
    const cooled = temp * temp; // quadratic cooling

    // Center gravity — pull everything toward center
    for (const node of layoutNodes) {
      const dx = canvasW / 2 - node.x;
      const dy = canvasH / 2 - node.y;
      node.vx += dx * CENTER_GRAVITY * cooled;
      node.vy += dy * CENTER_GRAVITY * cooled;
    }

    // Repulsion between all nodes (use Barnes-Hut-like cutoff for large graphs)
    const cutoffDist = n > 200 ? 600 : Infinity;
    for (let i = 0; i < layoutNodes.length; i++) {
      for (let j = i + 1; j < layoutNodes.length; j++) {
        const a = layoutNodes[i];
        const b = layoutNodes[j];
        const dx = a.x - b.x;
        const dy = a.y - b.y;
        const distSq = dx * dx + dy * dy;
        if (distSq > cutoffDist * cutoffDist) continue;
        const dist = Math.max(Math.sqrt(distSq), MIN_DIST);
        const force = (REPULSION * cooled) / (dist * dist);
        const fx = (dx / dist) * force;
        const fy = (dy / dist) * force;
        a.vx += fx;
        a.vy += fy;
        b.vx -= fx;
        b.vy -= fy;
      }
    }

    // Attraction along edges
    for (const rel of relationships) {
      const source = nodeMap.get(rel.source_id);
      const target = nodeMap.get(rel.target_id);
      if (!source || !target) continue;

      const dx = target.x - source.x;
      const dy = target.y - source.y;
      const dist = Math.sqrt(dx * dx + dy * dy);
      if (dist === 0) continue;

      // Ideal edge length — connected nodes should be at ~MIN_DIST*2
      const idealLen = MIN_DIST * 2.5;
      const displacement = dist - idealLen;
      const force = displacement * ATTRACTION * cooled;
      const fx = (dx / dist) * force;
      const fy = (dy / dist) * force;
      source.vx += fx;
      source.vy += fy;
      target.vx -= fx;
      target.vy -= fy;
    }

    // Same-label cohesion — gentle pull toward group centroid
    for (const label of labelList) {
      const members = layoutNodes.filter((ln) => ln.label === label);
      if (members.length < 2) continue;
      const avgX = members.reduce((s, m) => s + m.x, 0) / members.length;
      const avgY = members.reduce((s, m) => s + m.y, 0) / members.length;
      for (const m of members) {
        m.vx += (avgX - m.x) * LABEL_COHESION * cooled;
        m.vy += (avgY - m.y) * LABEL_COHESION * cooled;
      }
    }

    // Apply velocities
    for (const node of layoutNodes) {
      node.vx *= DAMPING;
      node.vy *= DAMPING;
      node.x += node.vx;
      node.y += node.vy;
    }
  }

  // Post-process: push apart any nodes that overlap (preserves overall shape)
  const OVERLAP_DIST = 80; // minimum pixel distance between node centers
  for (let pass = 0; pass < 15; pass++) {
    let anyOverlap = false;
    for (let i = 0; i < layoutNodes.length; i++) {
      for (let j = i + 1; j < layoutNodes.length; j++) {
        const a = layoutNodes[i];
        const b = layoutNodes[j];
        const dx = b.x - a.x;
        const dy = b.y - a.y;
        const dist = Math.sqrt(dx * dx + dy * dy);
        if (dist < OVERLAP_DIST) {
          anyOverlap = true;
          const push = (OVERLAP_DIST - dist) / 2 + 0.5;
          if (dist > 0.1) {
            const nx = dx / dist;
            const ny = dy / dist;
            a.x -= nx * push;
            a.y -= ny * push;
            b.x += nx * push;
            b.y += ny * push;
          } else {
            // Exactly overlapping — nudge horizontally
            a.x -= push;
            b.x += push;
          }
        }
      }
    }
    if (!anyOverlap) break;
  }

  // No hard boundary constraints — let fitView handle framing
  const positions = new Map<string, { x: number; y: number }>();
  for (const node of layoutNodes) {
    positions.set(node.id, { x: node.x, y: node.y });
  }
  return positions;
}

// Color scheme for node labels — kyma domain palette
const LABEL_COLORS: Record<string, string> = {
  // Kyma core domain
  Table: "#7ed957",
  Column: "#60a5fa",
  Database: "#a78bfa",
  Service: "#f59e0b",
  Trace: "#22d3ee",
  // Schema / SQL
  DatabaseTable: "#7ed957",
  DatabaseColumn: "#60a5fa",
  DatabaseView: "#818cf8",
  DatabaseFunction: "#a78bfa",
  DatabaseConstraint: "#f472b6",
  DatabaseExtension: "#94a3b8",
  DatabaseIndex: "#34d399",
  Index: "#34d399",
  Schema: "#e879f9",
  APIEndpoint: "#c084fc",
  OpenAPISpec: "#c084fc",
  // Infrastructure
  Server: "#10b981",
  InfraComponent: "#10b981",
  // General
  User: "#3b82f6",
  Namespace: "#06b6d4",
  Deployment: "#10b981",
  ConfigMap: "#34d399",
  Secret: "#6ee7b7",
  Endpoint: "#67e8f9",
  File: "#94a3b8",
  Directory: "#a8a29e",
  CsvImport: "#f472b6",
  JsonImport: "#c084fc",
};

// Generate a stable, visually distinct color from any string
function generateColor(str: string): string {
  let hash = 0;
  for (let i = 0; i < str.length; i++) {
    hash = ((hash << 5) - hash + str.charCodeAt(i)) | 0;
  }
  // Use golden angle for hue distribution, keep saturation/lightness vibrant
  const hue = (Math.abs(hash) * 137.508) % 360;
  return `hsl(${hue}, 65%, 55%)`;
}

export function getLabelColor(label: string): string {
  return LABEL_COLORS[label] || generateColor(label);
}

// Relationship type colors
const REL_COLORS: Record<string, string> = {
  // General
  OWNS: "#10b981",
  CONTAINS: "#3b82f6",
  DEPENDS_ON: "#f59e0b",
  CONNECTS_TO: "#10b981",
  HAS_PERMISSION: "#f97316",
  EXPOSES: "#ec4899",
  RUNS_IN: "#6366f1",
  REFERENCES: "#94a3b8",
  MANAGES: "#14b8a6",
  ACCESSES: "#f43f5e",
  MONITORS: "#84cc16",
  USES: "#f59e0b",
  RELATED: "#6b7280",
  PROVIDES: "#8b5cf6",
  // People / Membership
  MEMBER_OF: "#06b6d4",
  MEMBER_OF_GROUP: "#0891b2",
  BELONGS_TO: "#0ea5e9",
  CREATED_BY: "#34d399",
  AUTHORED: "#10b981",
  // SQL / Database
  HAS_TABLE: "#3b82f6",
  Has_Table: "#3b82f6",
  HAS_COLUMN: "#60a5fa",
  Has_Column: "#60a5fa",
  HAS_INDEX: "#34d399",
  Has_Index: "#34d399",
  HAS_FUNCTION: "#a78bfa",
  HAS_COLLECTION: "#13aa52",
  INDEXED_BY: "#34d399",
  CONSTRAINED_BY: "#f472b6",
  USES_EXTENSION: "#94a3b8",
  // Kubernetes
  RUNS_ON: "#22d3ee",
};

export function getRelationshipColor(type: string): string {
  return REL_COLORS[type] || REL_COLORS[type.toUpperCase()] || generateColor(type);
}

/**
 * Grid layout — places nodes in a rectangular grid. O(n), instant.
 */
export function gridLayout(
  nodeIds: string[],
  width: number,
  height: number,
): Map<string, { x: number; y: number }> {
  const positions = new Map<string, { x: number; y: number }>();
  const n = nodeIds.length;
  if (n === 0) return positions;

  const cols = Math.ceil(Math.sqrt(n * (width / height)));
  const rows = Math.ceil(n / cols);
  const cellW = width / (cols + 1);
  const cellH = height / (rows + 1);

  nodeIds.forEach((id, i) => {
    const col = i % cols;
    const row = Math.floor(i / cols);
    positions.set(id, {
      x: cellW * (col + 1),
      y: cellH * (row + 1),
    });
  });

  return positions;
}

/**
 * Radial layout — places nodes in concentric circles.
 * First node at center, then rings of increasing size.
 */
export function radialLayout(
  nodeIds: string[],
  width: number,
  height: number,
): Map<string, { x: number; y: number }> {
  const positions = new Map<string, { x: number; y: number }>();
  const n = nodeIds.length;
  if (n === 0) return positions;

  const cx = width / 2;
  const cy = height / 2;

  if (n === 1) {
    positions.set(nodeIds[0], { x: cx, y: cy });
    return positions;
  }

  positions.set(nodeIds[0], { x: cx, y: cy });

  let placed = 1;
  let ring = 1;
  const ringSpacing = Math.min(width, height) / (2 * (Math.ceil(Math.sqrt(n)) + 1));

  while (placed < n) {
    const radius = ring * ringSpacing;
    const circumference = 2 * Math.PI * radius;
    const nodesInRing = Math.min(
      Math.max(6, Math.floor(circumference / 80)),
      n - placed,
    );

    for (let i = 0; i < nodesInRing && placed < n; i++) {
      const angle = (2 * Math.PI * i) / nodesInRing - Math.PI / 2;
      positions.set(nodeIds[placed], {
        x: cx + radius * Math.cos(angle),
        y: cy + radius * Math.sin(angle),
      });
      placed++;
    }
    ring++;
  }

  return positions;
}

export type LayoutAlgorithm = "force" | "grid" | "radial";

/**
 * Dispatch to the appropriate layout algorithm.
 */
export function computeLayout(
  algorithm: LayoutAlgorithm,
  nodes: { id: string }[],
  relationships: { source_id: string; target_id: string }[],
  width: number,
  height: number,
): Map<string, { x: number; y: number }> {
  const ids = nodes.map((n) => n.id);
  switch (algorithm) {
    case "grid":
      return gridLayout(ids, width, height);
    case "radial":
      return radialLayout(ids, width, height);
    case "force":
    default:
      return forceDirectedLayout(nodes as unknown as GraphNode[], relationships as unknown as GraphRelationship[], width, height);
  }
}
