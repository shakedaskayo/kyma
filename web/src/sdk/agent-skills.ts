// Shim: backwards-compatible wrappers around @pensieve-ai/client agent-skills functions.

export type { SkillSource, SkillRow } from "@pensieve-ai/client";

import { sessionClient } from "./client";

type Args = { endpoint: string; token: string; database?: string };

export function listSkills(_a: Args) {
  return sessionClient().agent.listSkills();
}

export function putEnabledSkills(_a: Args, skills: string[]) {
  return sessionClient().agent.putEnabledSkills(skills);
}
