/**
 * ArtifactSourceViewer — read the source file a graph node references, by paging
 * the stored artifact at its `object_path` through the existing
 * `/v1/artifacts/by-path` API (client.artifacts.fetchArtifactByPath). Monospace,
 * wrap toggle, copy-all, byte counter, "Load more" until EOF, error + retry.
 */
import { useEffect, useRef, useState } from "react";
import { Copy, WrapText } from "lucide-react";
import { Button } from "../internal/ui/button";
import { usePensieveClient } from "../provider/context";

export function ArtifactSourceViewer({ path }: { path: string }) {
  const client = usePensieveClient();
  const [text, setText] = useState("");
  const [offset, setOffset] = useState(0);
  const [size, setSize] = useState<number | null>(null);
  const [eof, setEof] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [wrap, setWrap] = useState(true);
  // Bumped whenever `path` changes. Lets an in-flight fetch detect that it is
  // stale (the user selected a different node) and drop its result instead of
  // clobbering the new path's state.
  const reqRef = useRef(0);

  async function fetchWindow(atOffset: number, append: boolean) {
    const req = reqRef.current;
    setLoading(true);
    setError(null);
    try {
      const w = await client.artifacts.fetchArtifactByPath(path, { offset: atOffset });
      if (req !== reqRef.current) return; // stale — a newer path superseded this fetch
      setText((prev) => (append ? prev + w.content : w.content));
      setOffset(w.offset + w.returned_bytes);
      setSize(w.size_bytes);
      setEof(w.eof);
    } catch (e) {
      if (req !== reqRef.current) return;
      setError(e instanceof Error ? e.message : "Failed to load source file.");
    } finally {
      if (req === reqRef.current) setLoading(false);
    }
  }

  // Reset and load the first window whenever the path changes. Bumping reqRef
  // invalidates any fetch still in flight for the previous path.
  useEffect(() => {
    reqRef.current += 1;
    setText("");
    setOffset(0);
    setSize(null);
    setEof(false);
    setError(null);
    void fetchWindow(0, false);
    // fetchWindow is intentionally omitted from deps: it is recreated each render
    // and closes over the current path/client; the effect's real deps are below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [path, client]);

  return (
    <div className="pv-space-y-2">
      <div className="pv-flex pv-items-center pv-justify-between pv-gap-2">
        <div className="pv-truncate pv-font-mono pv-text-2xs pv-text-muted-foreground" title={path}>
          {path}
        </div>
        <div className="pv-flex pv-items-center pv-gap-1">
          <Button variant="ghost" size="xs" onClick={() => setWrap((w) => !w)}>
            <WrapText className="pv-h-3.5 pv-w-3.5" /> {wrap ? "No wrap" : "Wrap"}
          </Button>
          <Button
            variant="ghost"
            size="xs"
            disabled={!text}
            onClick={() => void navigator.clipboard.writeText(text)}
          >
            <Copy className="pv-h-3.5 pv-w-3.5" /> Copy
          </Button>
        </div>
      </div>

      {error ? (
        <div className="pv-rounded pv-border pv-border-destructive/40 pv-bg-destructive/10 pv-p-2 pv-text-xs pv-text-destructive">
          <div className="pv-mb-1">{error}</div>
          <Button variant="outline" size="xs" onClick={() => void fetchWindow(offset, true)}>
            Retry
          </Button>
        </div>
      ) : (
        <>
          <pre
            className={`pv-max-h-80 pv-overflow-auto pv-rounded pv-bg-muted pv-p-2 pv-font-mono pv-text-2xs ${
              wrap ? "pv-whitespace-pre-wrap pv-break-words" : "pv-whitespace-pre"
            }`}
          >
            {text || (loading ? "Loading…" : "No content.")}
          </pre>
          <div className="pv-flex pv-items-center pv-justify-between pv-text-2xs pv-text-muted-foreground">
            <span>{size != null ? `${Math.min(offset, size)} / ${size} bytes` : ""}</span>
            {!eof && !!text && (
              <Button
                variant="outline"
                size="xs"
                disabled={loading}
                onClick={() => void fetchWindow(offset, true)}
              >
                {loading ? "Loading…" : "Load more"}
              </Button>
            )}
          </div>
        </>
      )}
    </div>
  );
}
