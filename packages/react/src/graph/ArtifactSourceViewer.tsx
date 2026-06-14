/**
 * ArtifactSourceViewer — read the source file a graph node references, by paging
 * the stored artifact at its `object_path` through the existing
 * `/v1/artifacts/by-path` API (client.artifacts.fetchArtifactByPath). Monospace,
 * wrap toggle, copy-all, byte counter, "Load more" until EOF, error + retry.
 */
import { useEffect, useRef, useState } from "react";
import { Copy, WrapText } from "lucide-react";
import { Button } from "../internal/ui/button";
import { useKymaClient } from "../provider/context";

export function ArtifactSourceViewer({ path }: { path: string }) {
  const client = useKymaClient();
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
    <div className="ky-space-y-2">
      <div className="ky-flex ky-items-center ky-justify-between ky-gap-2">
        <div className="ky-truncate ky-font-mono ky-text-2xs ky-text-muted-foreground" title={path}>
          {path}
        </div>
        <div className="ky-flex ky-items-center ky-gap-1">
          <Button variant="ghost" size="xs" onClick={() => setWrap((w) => !w)}>
            <WrapText className="ky-h-3.5 ky-w-3.5" /> {wrap ? "No wrap" : "Wrap"}
          </Button>
          <Button
            variant="ghost"
            size="xs"
            disabled={!text}
            onClick={() => void navigator.clipboard.writeText(text)}
          >
            <Copy className="ky-h-3.5 ky-w-3.5" /> Copy
          </Button>
        </div>
      </div>

      {error ? (
        <div className="ky-rounded ky-border ky-border-destructive/40 ky-bg-destructive/10 ky-p-2 ky-text-xs ky-text-destructive">
          <div className="ky-mb-1">{error}</div>
          <Button variant="outline" size="xs" onClick={() => void fetchWindow(offset, true)}>
            Retry
          </Button>
        </div>
      ) : (
        <>
          <pre
            className={`ky-max-h-80 ky-overflow-auto ky-rounded ky-bg-muted ky-p-2 ky-font-mono ky-text-2xs ${
              wrap ? "ky-whitespace-pre-wrap ky-break-words" : "ky-whitespace-pre"
            }`}
          >
            {text || (loading ? "Loading…" : "No content.")}
          </pre>
          <div className="ky-flex ky-items-center ky-justify-between ky-text-2xs ky-text-muted-foreground">
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
