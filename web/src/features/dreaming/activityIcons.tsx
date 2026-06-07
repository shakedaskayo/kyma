import {
  Activity,
  Archive,
  Brain,
  CheckCircle2,
  Cloud,
  Database,
  GitMerge,
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
 * Maps the free-form `icon` slug carried on a progress activity entry to a
 * lucide glyph. Falls back to a generic Activity dot for unknown slugs so a new
 * server-side icon never breaks the feed.
 */
const ICONS: Record<string, LucideIcon> = {
  activity: Activity,
  archive: Archive,
  brain: Brain,
  check: CheckCircle2,
  cloud: Cloud,
  connector: Cloud,
  database: Database,
  created: Plus,
  create: Plus,
  entity: Link2,
  judge: Scale,
  judgement: Scale,
  link: Link2,
  merge: GitMerge,
  merged: GitMerge,
  moon: Moon,
  read: Cloud,
  rescore: Star,
  search: Search,
  sparkles: Sparkles,
  star: Star,
  summary: Sparkles,
  tool: Wrench,
  write: Plus,
};

export function activityIcon(slug: string | null | undefined): LucideIcon {
  if (!slug) return Activity;
  return ICONS[slug.toLowerCase()] ?? Activity;
}
