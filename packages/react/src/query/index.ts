// ── Internal query sub-components ────────────────────────────────────────────
// These are not yet part of the public package surface. They are exported here
// so the upcoming KymaQueryEditor wrapper (Task 5.4) can import from a single
// barrel rather than referencing deep internal paths.

export { KqlEditor } from "./editor/KqlEditor";
export type { KqlEditorProps, KqlSchema } from "./editor/KqlEditor";

export { SchemaBrowser } from "./schema/SchemaBrowser";
export type { SchemaBrowserProps } from "./schema/SchemaBrowser";

export { KQL_LANG_ID, registerKql, registerKqlCompletions } from "./editor/kql-language";
