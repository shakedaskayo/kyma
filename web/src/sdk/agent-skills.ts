//! Typed client for /v1/agent/skills + /v1/agent/skills/enabled.

export type SkillSource = "project" | "user" | "plugin";

export interface SkillRow {
  name: string;
  description: string;
  source: SkillSource;
  path: string;
  enabled: boolean;
  preview: string;
}

type Args = { endpoint: string; token: string };

function base(endpoint: string) {
  return endpoint.replace(/\/$/, "");
}
function headers(token: string): Record<string, string> {
  return {
    authorization: `Bearer ${token}`,
    "content-type": "application/json",
  };
}

export async function listSkills(a: Args): Promise<SkillRow[]> {
  const res = await fetch(`${base(a.endpoint)}/v1/agent/skills`, {
    headers: headers(a.token),
  });
  if (!res.ok) {
    const t = await res.text().catch(() => "");
    throw new Error(`skills: ${res.status}${t ? ` — ${t}` : ""}`);
  }
  return res.json();
}

export async function putEnabledSkills(
  a: Args,
  skills: string[],
): Promise<string[]> {
  const res = await fetch(`${base(a.endpoint)}/v1/agent/skills/enabled`, {
    method: "PUT",
    headers: headers(a.token),
    body: JSON.stringify({ skills }),
  });
  if (!res.ok) {
    const t = await res.text().catch(() => "");
    throw new Error(`save skills: ${res.status}${t ? ` — ${t}` : ""}`);
  }
  return res.json();
}
