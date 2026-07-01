import {
  Activity,
  AlertTriangle,
  Archive,
  Brain,
  CheckCircle2,
  Cloud,
  Database,
  GitMerge,
  Layers,
  Link2,
  Moon,
  Plus,
  Scale,
  Search,
  Sparkles,
  Star,
  Wrench,
  type LucideIcon,
} from "lucide-react";

/**
 * Maps the free-form `icon` carried on a progress activity entry to a lucide
 * glyph. Falls back to a generic Activity dot for unmapped values so a new
 * server-side icon never breaks the feed.
 *
 * The server (`dreaming.rs::observe_tool_call`) actually emits literal emoji,
 * not slugs — the emoji keys below are the ones that matter in production.
 * The English-slug keys are kept too (additive, harmless) in case a future
 * emitter prefers them.
 */
const ICONS: Record<string, LucideIcon> = {
  // Emoji keys — what the server actually sends today.
  "🧠": Brain, // memory save
  "🧩": Layers, // procedure/schema save (M8.4 schema induction)
  "⚖️": Scale, // merge / judge
  "📦": Archive, // archive / status update
  "📈": Star, // importance rescore
  "🔗": Link2, // entity link
  "🔌": Cloud, // data source read/list
  "🔍": Search, // search / recall
  "🔧": Wrench, // generic tool fallback
  "⚠️": AlertTriangle, // abnormal run end

  // English slugs — additive fallback, not currently emitted.
  activity: Activity,
  archive: Archive,
  brain: Brain,
  check: CheckCircle2,
  cloud: Cloud,
  data_source: Cloud,
  database: Database,
  created: Plus,
  create: Plus,
  entity: Link2,
  judge: Scale,
  judgement: Scale,
  layers: Layers,
  link: Link2,
  merge: GitMerge,
  merged: GitMerge,
  moon: Moon,
  procedure: Layers,
  read: Cloud,
  rescore: Star,
  search: Search,
  sparkles: Sparkles,
  star: Star,
  summary: Sparkles,
  tool: Wrench,
  warning: AlertTriangle,
  write: Plus,
};

export function activityIcon(slug: string | null | undefined): LucideIcon {
  if (!slug) return Activity;
  return ICONS[slug.toLowerCase()] ?? Activity;
}
