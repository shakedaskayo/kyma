/**
 * useGraphExport — loads the FULL graph (all pages, all coords) with
 * server-computed positions. Pages stream into an ExportAccumulator and the
 * hook re-renders per page so the canvas fills progressively. Streams that
 * report `layout_status: "computing"` are polled until ready.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import type { LayoutAlgorithm } from "@pensieve-ai/client";
import { usePensieveClient } from "../provider/context";
import { graphKey, useGraphCoords, type UsePensieveGraphArgs } from "./usePensieveGraph";
import {
  createExportAccumulator,
  mergeExportPage,
  type ExportAccumulator,
} from "../graph/graph-export-merge";

const COMPUTING_POLL_MS = 1500;
const PAGE_SIZE = 10_000;

export interface UseGraphExportResult {
  acc: ExportAccumulator;
  /** Bumped on every merged page — memo key for downstream consumers. */
  version: number;
  coords: ReturnType<typeof useGraphCoords>["coords"];
  /** True while any stream is still computing server-side layout. */
  layoutComputing: boolean;
  isLoading: boolean;
  isError: boolean;
  error: unknown;
  refetch: () => void;
}

export function useGraphExport(
  args: UsePensieveGraphArgs & { algorithm: LayoutAlgorithm },
): UseGraphExportResult {
  const client = usePensieveClient();
  const { coords, isLoading: discovering, error: discoveryError } = useGraphCoords(args);
  const [version, setVersion] = useState(0);
  const [layoutComputing, setLayoutComputing] = useState(false);
  const [streamsDone, setStreamsDone] = useState(0);
  const [error, setError] = useState<unknown>(null);
  const accRef = useRef<ExportAccumulator>(createExportAccumulator());
  const [generation, setGeneration] = useState(0);

  const coordsKey = coords.map((c) => graphKey(c.database, c.graph)).join(",");

  useEffect(() => {
    if (coords.length === 0) return;
    let cancelled = false;
    accRef.current = createExportAccumulator();
    setVersion((v) => v + 1);
    setStreamsDone(0);
    setError(null);

    const runStream = async (database: string, graph: string) => {
      const ns = graphKey(database, graph);
      let cursor: string | undefined;
      for (;;) {
        if (cancelled) return;
        const page = await client.withDatabase(database).graph.exportGraph({
          graph,
          realm: args.realm,
          algorithm: args.algorithm,
          cursor,
          pageSize: PAGE_SIZE,
        });
        if (cancelled) return;
        if (page.layout_status === "computing") {
          setLayoutComputing(true);
          await new Promise((r) => setTimeout(r, COMPUTING_POLL_MS));
          continue; // re-request first page
        }
        mergeExportPage(accRef.current, page, ns, database);
        setVersion((v) => v + 1);
        if (!page.next_cursor) return;
        cursor = page.next_cursor;
      }
    };

    void Promise.allSettled(
      coords.map((c) =>
        runStream(c.database, c.graph)
          .catch((e) => {
            setError((prev: unknown) => prev ?? e);
            throw e;
          })
          .finally(() => !cancelled && setStreamsDone((d) => d + 1)),
      ),
    ).then(() => !cancelled && setLayoutComputing(false));

    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [coordsKey, args.realm, args.algorithm, generation]);

  const refetch = useCallback(() => setGeneration((g) => g + 1), []);

  return {
    acc: accRef.current,
    version,
    coords,
    layoutComputing,
    isLoading: discovering || (coords.length > 0 && streamsDone < coords.length && version <= 1),
    isError: !discovering && Boolean(discoveryError ?? error) && accRef.current.nodes.length === 0,
    error: error ?? discoveryError,
    refetch,
  };
}
