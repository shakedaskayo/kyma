import type { ComponentType } from "react";
import { Key, KeyRound, Globe, Lock, Cloud, User as UserIcon, ShieldCheck, Boxes, Building2 } from "lucide-react";

// One glyph per kind — a quick visual cue in the list. Brand marks belong on
// the *credential consumer* (the data source), not here; this just types the
// secret material itself. `any` props: lucide's prop type uses `string | number`
// for `size` which fights a narrower constraint — runtime is fine.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const MAP: Record<string, ComponentType<any>> = {
  pat: Key,
  basic: UserIcon,
  oauth2: ShieldCheck,
  url: Globe,
  aws_creds: Cloud,
  api_key: KeyRound,
  github_app: Boxes,
  service_principal: Building2,
};
const FALLBACK = Lock;

export function CredentialIcon({
  kind,
  size = 18,
  className,
}: {
  kind: string;
  size?: number;
  className?: string;
}) {
  const Icon = MAP[kind] ?? FALLBACK;
  return <Icon size={size} className={className} />;
}
