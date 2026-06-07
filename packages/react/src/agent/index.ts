// Primary public export
export { KymaAgentChat } from "./KymaAgentChat";
export type { KymaAgentChatProps } from "./KymaAgentChat";

// Secondary export: the full console panel (database label, thinking toggle)
export { AgentConsole } from "./AgentConsole";
export type { AgentConsoleProps } from "./AgentConsole";

// Chat element primitives — exported so the host app can render persisted
// agent conversations (e.g. dreaming-run drilldowns) with the same look as
// the live console.
export { Conversation, ConversationContent } from "./elements/conversation";
export { Message, MessageContent } from "./elements/message";
export { Response } from "./elements/response";
export { Reasoning, ReasoningTrigger, ReasoningContent } from "./elements/reasoning";
export { Tool, ToolHeader, ToolContent, ToolInput, ToolOutput } from "./elements/tool";
export type { ToolState } from "./elements/tool";
export { Loader } from "./elements/loader";
