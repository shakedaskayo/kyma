// ── Public surface: KymaQueryEditor ──────────────────────────────────────────
export { KymaQueryEditor } from "./KymaQueryEditor";
export type { KymaQueryEditorProps } from "./KymaQueryEditor";
export type { TimeRange } from "./time-range/time-range-types";

// ── Secondary public surface: sub-components ──────────────────────────────────
export { KqlEditor } from "./editor/KqlEditor";
export type { KqlEditorProps, KqlSchema } from "./editor/KqlEditor";

export { SchemaBrowser } from "./schema/SchemaBrowser";
export type { SchemaBrowserProps } from "./schema/SchemaBrowser";

export { KQL_LANG_ID, registerKql, registerKqlCompletions } from "./editor/kql-language";
