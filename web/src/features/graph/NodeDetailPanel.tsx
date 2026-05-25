'use client';

import { useState, useMemo } from 'react';
import { X, Copy, ArrowRight, ArrowLeft } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Badge } from '@/components/ui/badge';
import { ScrollArea } from '@/components/ui/scroll-area';
import { cn } from '@/lib/utils';
import { getLabelColor, getRelationshipColor } from '@/sdk/graph-layout';
import type { GraphNode, GraphRelationship } from '@/sdk/graph';

interface NodeDetailPanelProps {
  node: GraphNode;
  edges: GraphRelationship[];
  nodesById: Map<string, GraphNode>;
  onClose: () => void;
  onExpand: (id: string) => void;
  onSelect: (id: string) => void;
}

type ActiveTab = 'details' | 'connections';

interface GroupedConnection {
  relType: string;
  color: string;
  connections: {
    nodeId: string;
    nodeName: string;
    direction: 'outgoing' | 'incoming';
    relId: string;
  }[];
}

export function NodeDetailPanel({
  node,
  edges,
  nodesById,
  onClose,
  onExpand,
  onSelect,
}: NodeDetailPanelProps) {
  const [activeTab, setActiveTab] = useState<ActiveTab>('details');

  const displayName = (node.properties.name as string | undefined) ?? node.id;

  const groupedConnections = useMemo((): GroupedConnection[] => {
    if (edges.length === 0) return [];

    const groups = new Map<string, GroupedConnection>();

    for (const rel of edges) {
      let otherId: string | null = null;
      let direction: 'outgoing' | 'incoming' = 'outgoing';

      if (rel.source_id === node.id) {
        otherId = rel.target_id;
        direction = 'outgoing';
      } else if (rel.target_id === node.id) {
        otherId = rel.source_id;
        direction = 'incoming';
      }

      if (!otherId) continue;

      const otherNode = nodesById.get(otherId);
      const nodeName = otherNode
        ? ((otherNode.properties.name as string | undefined) ?? otherId)
        : otherId;

      if (!groups.has(rel.relationship_type)) {
        groups.set(rel.relationship_type, {
          relType: rel.relationship_type,
          color: getRelationshipColor(rel.relationship_type),
          connections: [],
        });
      }

      groups.get(rel.relationship_type)!.connections.push({
        nodeId: otherId,
        nodeName,
        direction,
        relId: rel.id,
      });
    }

    return Array.from(groups.values()).sort(
      (a, b) => b.connections.length - a.connections.length,
    );
  }, [node.id, edges, nodesById]);

  const totalConnections = groupedConnections.reduce(
    (sum, g) => sum + g.connections.length,
    0,
  );

  return (
    <div className="flex h-full w-80 flex-col border-l border-border bg-background">
      {/* Header */}
      <div className="flex shrink-0 items-start justify-between gap-2 border-b border-border px-4 py-3">
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-medium">{displayName}</p>
          <div className="mt-1.5 flex flex-wrap gap-1">
            {node.labels.map((label) => (
              <Badge
                key={label}
                variant="outline"
                className="text-[10px]"
                style={{ borderColor: getLabelColor(label) }}
              >
                {label}
              </Badge>
            ))}
          </div>
        </div>
        <Button
          variant="ghost"
          size="icon"
          className="h-7 w-7 shrink-0"
          onClick={onClose}
          aria-label="Close panel"
        >
          <X className="h-4 w-4" />
        </Button>
      </div>

      {/* Tab toggle */}
      <div className="flex shrink-0 border-b border-border">
        <button
          onClick={() => setActiveTab('details')}
          className={cn(
            'flex-1 px-3 py-2 text-xs font-medium transition-colors',
            activeTab === 'details'
              ? 'border-b-2 border-foreground text-foreground'
              : 'text-muted-foreground hover:text-foreground',
          )}
        >
          Details
        </button>
        <button
          onClick={() => setActiveTab('connections')}
          className={cn(
            'flex-1 px-3 py-2 text-xs font-medium transition-colors',
            activeTab === 'connections'
              ? 'border-b-2 border-foreground text-foreground'
              : 'text-muted-foreground hover:text-foreground',
          )}
        >
          Connections{totalConnections > 0 ? ` (${totalConnections})` : ''}
        </button>
      </div>

      {/* Tab content */}
      <div className="min-h-0 flex-1">
        {activeTab === 'details' ? (
          <ScrollArea className="h-full">
            <div className="space-y-4 p-4">
              {/* Properties */}
              <div>
                <p className="mb-2 text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
                  Properties
                </p>
                <div className="space-y-2">
                  {Object.entries(node.properties).map(([key, value]) => {
                    const strValue =
                      typeof value === 'object'
                        ? JSON.stringify(value)
                        : String(value);
                    const display =
                      strValue.length > 120
                        ? strValue.slice(0, 120) + '…'
                        : strValue;
                    return (
                      <div key={key}>
                        <p className="text-[10px] text-muted-foreground">{key}</p>
                        <button
                          onClick={() =>
                            navigator.clipboard.writeText(strValue)
                          }
                          className="group break-all text-left text-xs font-mono transition-colors hover:text-primary"
                          title="Click to copy"
                        >
                          {display}
                          <Copy className="ml-1 inline h-3 w-3 opacity-0 group-hover:opacity-50" />
                        </button>
                      </div>
                    );
                  })}
                </div>
              </div>

              {/* Metadata */}
              <div className="border-t border-border pt-4">
                <p className="mb-2 text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
                  Metadata
                </p>
                <div className="space-y-2">
                  {/* ID */}
                  <div>
                    <p className="text-[10px] text-muted-foreground">ID</p>
                    <div className="flex items-center gap-1">
                      <p className="truncate text-xs font-mono">{node.id}</p>
                      <button
                        onClick={() => navigator.clipboard.writeText(node.id)}
                        className="shrink-0"
                        title="Copy ID"
                      >
                        <Copy className="h-3 w-3 text-muted-foreground hover:text-foreground" />
                      </button>
                    </div>
                  </div>
                  {/* Realm */}
                  <div>
                    <p className="text-[10px] text-muted-foreground">Realm</p>
                    <p className="text-xs">{node.metadata.realm}</p>
                  </div>
                  {/* Source type */}
                  <div>
                    <p className="text-[10px] text-muted-foreground">
                      Source type
                    </p>
                    <p className="text-xs">
                      {node.metadata.source_type ?? '—'}
                    </p>
                  </div>
                  {/* Updated at */}
                  <div>
                    <p className="text-[10px] text-muted-foreground">
                      Updated
                    </p>
                    <p className="text-xs">
                      {new Date(node.metadata.updated_at).toLocaleString()}
                    </p>
                  </div>
                </div>
              </div>
            </div>
          </ScrollArea>
        ) : (
          <ScrollArea className="h-full">
            <div className="space-y-1 p-4">
              {groupedConnections.length === 0 ? (
                <p className="py-6 text-center text-xs text-muted-foreground">
                  No connections found
                </p>
              ) : (
                groupedConnections.map((group) => (
                  <div key={group.relType} className="mb-3">
                    {/* Group header */}
                    <div className="mb-1 flex items-center gap-2 px-1">
                      <span
                        className="h-[2px] w-3 rounded-full"
                        style={{ backgroundColor: group.color }}
                      />
                      <span
                        className="text-[10px] font-semibold uppercase tracking-wider"
                        style={{ color: group.color }}
                      >
                        {group.relType.replace(/_/g, ' ')}
                      </span>
                      <span className="ml-auto text-[10px] text-muted-foreground">
                        {group.connections.length}
                      </span>
                    </div>
                    {/* Connection rows */}
                    <div className="space-y-0.5">
                      {group.connections.map((conn) => (
                        <button
                          key={conn.relId}
                          onClick={() => onSelect(conn.nodeId)}
                          className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left transition-colors hover:bg-muted/50"
                        >
                          {conn.direction === 'outgoing' ? (
                            <ArrowRight className="h-3 w-3 shrink-0 text-muted-foreground" />
                          ) : (
                            <ArrowLeft className="h-3 w-3 shrink-0 text-muted-foreground" />
                          )}
                          <span
                            className="h-2 w-2 shrink-0 rounded-full"
                            style={{ backgroundColor: group.color }}
                          />
                          <span className="truncate text-xs text-foreground/80">
                            {conn.nodeName}
                          </span>
                        </button>
                      ))}
                    </div>
                  </div>
                ))
              )}
            </div>
          </ScrollArea>
        )}
      </div>

      {/* Footer */}
      <div className="shrink-0 border-t border-border p-3">
        <Button
          variant="outline"
          size="sm"
          className="w-full text-xs"
          onClick={() => onExpand(node.id)}
        >
          Expand neighbors
        </Button>
      </div>
    </div>
  );
}
