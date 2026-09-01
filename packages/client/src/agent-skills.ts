//! Typed client for /v1/agent/skills + /v1/agent/skills/enabled.

import type { PensieveTransport } from "./transport";
import { errorFromResponse } from "./errors";

export type SkillSource = "project" | "user" | "plugin";

export interface SkillRow {
  name: string;
  description: string;
  source: SkillSource;
  path: string;
  enabled: boolean;
  preview: string;
}

async function handleResponse<T>(res: Response): Promise<T> {
  if (!res.ok) throw await errorFromResponse(res);
  return res.json() as Promise<T>;
}

export async function listSkills(t: PensieveTransport): Promise<SkillRow[]> {
  return handleResponse<SkillRow[]>(await t.request("/v1/agent/skills"));
}

export async function putEnabledSkills(
  t: PensieveTransport,
  skills: string[],
): Promise<string[]> {
  return handleResponse<string[]>(
    await t.request("/v1/agent/skills/enabled", {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ skills }),
    }),
  );
}
