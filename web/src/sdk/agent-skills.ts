// Shim: backwards-compatible wrappers around @kyma-ai/client agent-skills functions.
// Old API: listSkills(a), putEnabledSkills(a, skills)

export type { SkillSource, SkillRow } from "@kyma-ai/client";

import {
  listSkills as _listSkills,
  putEnabledSkills as _putEnabledSkills,
} from "@kyma-ai/client";
import { transportFor } from "./compat";

type Args = { endpoint: string; token: string; database?: string };

export function listSkills(a: Args) {
  return _listSkills(transportFor(a));
}

export function putEnabledSkills(a: Args, skills: string[]) {
  return _putEnabledSkills(transportFor(a), skills);
}
