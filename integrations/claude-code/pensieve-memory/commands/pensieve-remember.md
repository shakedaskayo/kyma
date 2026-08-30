---
description: Save a durable memory to Kyma, or distill the recent conversation into memories.
argument-hint: [optional explicit note to remember]
---

Persist durable memory to Kyma (your unified memory layer).

**If `$ARGUMENTS` is non-empty**, save it directly: call the MCP tool `save_memory`
(server `kyma`) with:
- `content`: "$ARGUMENTS"
- `memory_type`: infer one of fact | decision | preference | learning
- `realm`: the current project realm (working directory basename)
- `importance`: 0.4–0.8 based on how durable/load-bearing it is

**If `$ARGUMENTS` is empty**, distill this conversation: review what we did this
session and extract the small set of genuinely durable memories — decisions made,
preferences expressed, conventions established, non-obvious facts learned. Save each
with `save_memory` (deduplicate against what you can `recall_memory` first; skip
anything transient). For a long/complex session, delegate to the `memory-curator`
subagent.

Report back the list of memories saved (content + type), and skip anything already
remembered.
