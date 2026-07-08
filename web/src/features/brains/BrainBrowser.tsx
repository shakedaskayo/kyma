import { useEffect, useMemo, useState } from "react";
import {
  ChevronRight,
  CornerDownRight,
  FileText,
  Folder,
  Hash,
  Search,
  X,
} from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { SkeletonRows } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";
import { NoteMarkdown } from "./NoteMarkdown";
import { useBrainFile, useBrainTree } from "./useBrains";

/**
 * Obsidian-style vault browser: a searchable file tree, a reading-width note
 * pane with a title header + metadata, live `[[wikilinks]]`, and a right
 * rail of outgoing links + folder siblings. Read-only view over the brain's
 * exported HEAD (`/v1/brain/:name/{tree,file}`).
 */

// ── frontmatter + labels ──────────────────────────────────────────────────────

interface ParsedNote {
  front: Record<string, string>;
  tags: string[];
  title: string;
  body: string;
}

function parseNote(raw: string, fallbackTitle: string): ParsedNote {
  if (!raw.startsWith("---\n")) return { front: {}, tags: [], title: fallbackTitle, body: raw };
  const end = raw.indexOf("\n---\n", 4);
  if (end < 0) return { front: {}, tags: [], title: fallbackTitle, body: raw };
  const front: Record<string, string> = {};
  let tags: string[] = [];
  for (const line of raw.slice(4, end).split("\n")) {
    const m = line.match(/^([A-Za-z_][\w-]*):\s*(.*)$/);
    if (!m) continue;
    const [, k, v] = m;
    if (k === "tags" || k === "aliases") {
      if (k === "tags")
        tags = v
          .replace(/^\[|\]$/g, "")
          .split(",")
          .map((t) => t.trim())
          .filter(Boolean);
    } else {
      front[k] = v.replace(/^"|"$/g, "");
    }
  }
  let body = raw.slice(end + 5).replace(/^\n+/, "");
  // The H1 duplicates the title — drop it, we render the title in the header.
  body = body.replace(/^#\s+.*\n+/, "");
  return { front, tags, title: front.title ?? fallbackTitle, body };
}

function noteLabel(path: string): string {
  const stem = path.split("/").pop()!.replace(/\.md$/, "");
  return stem.replace(/-[0-9a-f]{8}$/, "").replace(/-/g, " ");
}

function slugify(s: string): string {
  return s.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "");
}

/** Accent color per note kind (folder or path). */
function kindColor(path: string): string {
  if (path.includes("/decisions/")) return "text-rose-400";
  if (path.includes("/learnings/")) return "text-sky-400";
  if (path.includes("/procedures/")) return "text-violet-400";
  if (path.includes("/preferences/")) return "text-amber-400";
  if (path.startsWith("entities/")) return "text-emerald-400";
  if (path.startsWith("wiki/")) return "text-yellow-400";
  return "text-muted-foreground";
}

// ── tree model ─────────────────────────────────────────────────────────────────

interface TreeDir {
  name: string;
  path: string;
  dirs: Map<string, TreeDir>;
  files: string[];
}

const HIDDEN = (p: string) =>
  p.startsWith(".kyma/") || p.startsWith(".obsidian/") || p === ".gitignore";

function buildTree(paths: string[]): TreeDir {
  const root: TreeDir = { name: "", path: "", dirs: new Map(), files: [] };
  for (const p of paths) {
    if (HIDDEN(p)) continue;
    const segs = p.split("/");
    let cur = root;
    let acc = "";
    for (const seg of segs.slice(0, -1)) {
      acc = acc ? `${acc}/${seg}` : seg;
      if (!cur.dirs.has(seg)) cur.dirs.set(seg, { name: seg, path: acc, dirs: new Map(), files: [] });
      cur = cur.dirs.get(seg)!;
    }
    cur.files.push(p);
  }
  return root;
}

function countFiles(dir: TreeDir): number {
  let n = dir.files.length;
  for (const d of dir.dirs.values()) n += countFiles(d);
  return n;
}

// ── tree rendering ─────────────────────────────────────────────────────────────

function FileRow({
  path,
  depth,
  active,
  onOpen,
}: {
  path: string;
  depth: number;
  active: boolean;
  onOpen: (p: string) => void;
}) {
  return (
    <button
      type="button"
      className={cn(
        "group flex w-full items-center gap-1.5 truncate rounded-md py-1 pr-2 text-left text-[13px] transition-colors",
        active ? "bg-primary/12 font-medium text-primary" : "text-foreground/80 hover:bg-muted/50",
      )}
      style={{ paddingLeft: `${depth * 14 + 8}px` }}
      onClick={() => onOpen(path)}
    >
      <FileText className={cn("h-3.5 w-3.5 shrink-0", active ? "text-primary" : kindColor(path))} />
      <span className="truncate capitalize">{noteLabel(path)}</span>
    </button>
  );
}

function DirRow({
  dir,
  depth,
  active,
  expanded,
  onOpen,
}: {
  dir: TreeDir;
  depth: number;
  active: string | null;
  expanded: Set<string>;
  onOpen: (p: string) => void;
}) {
  const [open, setOpen] = useState(depth < 1 || expanded.has(dir.path));
  useEffect(() => {
    if (expanded.has(dir.path)) setOpen(true);
  }, [expanded, dir.path]);
  return (
    <div>
      <button
        type="button"
        className="flex w-full items-center gap-1 rounded-md py-1 pr-2 text-left text-[11px] font-semibold uppercase tracking-wide text-muted-foreground/80 hover:bg-muted/40"
        style={{ paddingLeft: `${depth * 14 + 4}px` }}
        onClick={() => setOpen((o) => !o)}
      >
        <ChevronRight className={cn("h-3 w-3 transition-transform", open && "rotate-90")} />
        <Folder className="h-3.5 w-3.5 shrink-0 opacity-60" />
        <span className="truncate">{dir.name}</span>
        <span className="ml-auto font-normal normal-case text-muted-foreground/50">
          {countFiles(dir)}
        </span>
      </button>
      {open && (
        <div>
          {[...dir.dirs.values()]
            .sort((a, b) => a.name.localeCompare(b.name))
            .map((d) => (
              <DirRow
                key={d.path}
                dir={d}
                depth={depth + 1}
                active={active}
                expanded={expanded}
                onOpen={onOpen}
              />
            ))}
          {dir.files
            .slice()
            .sort()
            .map((f) => (
              <FileRow key={f} path={f} depth={depth + 1} active={active === f} onOpen={onOpen} />
            ))}
        </div>
      )}
    </div>
  );
}

// ── main ───────────────────────────────────────────────────────────────────────

export function BrainBrowser({ name }: { name: string }) {
  const { data: tree, isLoading, error } = useBrainTree(name);
  const [selected, setSelected] = useState<string | null>(null);
  const [query, setQuery] = useState("");

  const homePath = tree?.paths.includes("index.md") ? "index.md" : tree?.paths.find((p) => p.endsWith(".md")) ?? null;
  const current = selected ?? homePath;
  const { data: file, isLoading: fileLoading } = useBrainFile(name, current);

  // Wikilink resolution: exact path → path.md → title-slug against stems.
  const resolve = useMemo(() => {
    const paths = new Set(tree?.paths ?? []);
    const byStem = new Map<string, string>();
    for (const p of tree?.paths ?? []) {
      if (!p.endsWith(".md")) continue;
      const stem = p.split("/").pop()!.replace(/\.md$/, "");
      byStem.set(stem, p);
      byStem.set(stem.replace(/-[0-9a-f]{8}$/, ""), p);
    }
    return (target: string): string | null => {
      if (paths.has(target)) return target;
      if (paths.has(`${target}.md`)) return `${target}.md`;
      return byStem.get(slugify(target)) ?? null;
    };
  }, [tree?.paths]);

  // Search: flat filtered list; also auto-expands matching folders.
  const filtered = useMemo(() => {
    if (!tree || !query.trim()) return null;
    const q = query.toLowerCase();
    return tree.paths.filter((p) => !HIDDEN(p) && p.endsWith(".md") && noteLabel(p).toLowerCase().includes(q));
  }, [tree, query]);

  const parsed = useMemo(
    () => (file && current ? parseNote(file.content, noteLabel(current)) : null),
    [file, current],
  );

  // Outgoing links from the current note (body wikilinks that resolve).
  const outgoing = useMemo(() => {
    if (!parsed) return [];
    const seen = new Set<string>();
    const out: string[] = [];
    for (const m of parsed.body.matchAll(/\[\[([^\]]+)\]\]/g)) {
      const t = m[1].split("|")[0].split(/[#^]/)[0].trim();
      const r = resolve(t);
      if (r && r !== current && !seen.has(r)) {
        seen.add(r);
        out.push(r);
      }
    }
    return out;
  }, [parsed, resolve, current]);

  if (isLoading) return <SkeletonRows rows={5} className="py-2" />;
  if (error) {
    return (
      <div className="rounded-lg border bg-muted/10 py-12 text-center text-sm text-muted-foreground">
        {(error as Error).message.includes("no exports")
          ? "No exports yet — trigger one to browse the vault."
          : `Failed to load vault: ${(error as Error).message}`}
      </div>
    );
  }
  if (!tree) return null;
  const root = buildTree(tree.paths);
  const crumbs = current ? current.replace(/\.md$/, "").split("/") : [];

  return (
    <div className="flex h-full min-h-[440px] overflow-hidden rounded-xl border bg-background shadow-sm">
      {/* ── sidebar ── */}
      <aside className="flex w-64 shrink-0 flex-col border-r bg-muted/20">
        <div className="border-b p-2">
          <div className="relative">
            <Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search notes…"
              className="h-8 w-full rounded-md border border-input bg-background pl-8 pr-7 text-xs outline-none ring-primary/30 focus:ring-2"
            />
            {query && (
              <button
                type="button"
                className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                onClick={() => setQuery("")}
              >
                <X className="h-3.5 w-3.5" />
              </button>
            )}
          </div>
        </div>
        <div className="flex-1 overflow-y-auto p-1.5">
          {filtered ? (
            filtered.length === 0 ? (
              <p className="px-2 py-4 text-center text-xs text-muted-foreground">No matches.</p>
            ) : (
              filtered.map((f) => (
                <FileRow key={f} path={f} depth={0} active={current === f} onOpen={setSelected} />
              ))
            )
          ) : (
            <>
              {root.files
                .slice()
                .sort()
                .map((f) => (
                  <FileRow key={f} path={f} depth={0} active={current === f} onOpen={setSelected} />
                ))}
              {[...root.dirs.values()]
                .sort((a, b) => a.name.localeCompare(b.name))
                .map((d) => (
                  <DirRow
                    key={d.path}
                    dir={d}
                    depth={0}
                    active={current}
                    expanded={new Set(current?.split("/").slice(0, -1).map((_, i, a) => a.slice(0, i + 1).join("/")))}
                    onOpen={setSelected}
                  />
                ))}
            </>
          )}
        </div>
      </aside>

      {/* ── note ── */}
      <main className="min-w-0 flex-1 overflow-y-auto">
        {!current ? (
          <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
            Select a note.
          </div>
        ) : fileLoading || !parsed ? (
          <div className="p-8">
            <SkeletonRows rows={5} />
          </div>
        ) : (
          <article className="mx-auto max-w-3xl px-8 py-7">
            {/* breadcrumb */}
            <nav className="mb-4 flex items-center gap-1 text-xs text-muted-foreground">
              {crumbs.map((c, i) => (
                <span key={i} className="flex items-center gap-1">
                  {i > 0 && <ChevronRight className="h-3 w-3 opacity-50" />}
                  <span className={cn(i === crumbs.length - 1 && "text-foreground")}>
                    {i === crumbs.length - 1 ? parsed.title : c.replace(/-/g, " ")}
                  </span>
                </span>
              ))}
            </nav>

            <h1 className="text-2xl font-semibold tracking-tight">{parsed.title}</h1>

            {/* metadata */}
            <div className="mt-2.5 flex flex-wrap items-center gap-1.5">
              {parsed.front.type && (
                <Badge className={cn("border-transparent bg-muted text-2xs", kindColor(current))}>
                  {parsed.front.type}
                </Badge>
              )}
              {parsed.front.realm && (
                <Badge variant="outline" className="text-2xs">
                  {parsed.front.realm}
                </Badge>
              )}
              {parsed.front.status && parsed.front.status !== "active" && (
                <Badge variant="outline" className="text-2xs text-amber-500">
                  {parsed.front.status}
                </Badge>
              )}
              {parsed.tags.map((t) => (
                <span key={t} className="inline-flex items-center gap-0.5 rounded-full bg-muted px-1.5 py-0.5 text-2xs text-muted-foreground">
                  <Hash className="h-2.5 w-2.5" />
                  {t}
                </span>
              ))}
              {parsed.front.importance && (
                <span className="text-2xs text-muted-foreground">· importance {parsed.front.importance}</span>
              )}
              {parsed.front.updated && (
                <span className="text-2xs text-muted-foreground">· updated {parsed.front.updated.slice(0, 10)}</span>
              )}
            </div>

            <div className="mt-5 border-t pt-5">
              <NoteMarkdown source={parsed.body} resolveLink={resolve} onOpen={setSelected} />
            </div>

            {outgoing.length > 0 && (
              <div className="mt-8 rounded-lg border bg-muted/20 p-3">
                <p className="mb-2 flex items-center gap-1.5 text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
                  <CornerDownRight className="h-3 w-3" /> Links
                </p>
                <div className="flex flex-col gap-0.5">
                  {outgoing.map((p) => (
                    <button
                      key={p}
                      type="button"
                      className="flex items-center gap-1.5 rounded px-1.5 py-1 text-left text-xs text-foreground/80 hover:bg-muted/60"
                      onClick={() => setSelected(p)}
                    >
                      <FileText className={cn("h-3 w-3", kindColor(p))} />
                      <span className="capitalize">{noteLabel(p)}</span>
                    </button>
                  ))}
                </div>
              </div>
            )}
          </article>
        )}
      </main>
    </div>
  );
}
