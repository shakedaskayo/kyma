import type { Meta, StoryObj } from "@storybook/react";
import { PensieveAgentChat } from "./PensieveAgentChat";

const meta: Meta<typeof PensieveAgentChat> = {
  title: "Components/PensieveAgentChat",
  component: PensieveAgentChat,
  argTypes: {
    placeholder: {
      control: "text",
      description: "Placeholder text in the input area",
    },
    database: {
      control: "text",
      description: "Override the database sent with each request",
    },
  },
  args: {
    style: { height: "100vh" },
  },
};

export default meta;
type Story = StoryObj<typeof PensieveAgentChat>;

/**
 * Default chat: type a question and submit to see the canned fixture response.
 * The fixture fetch intercepts /v1/agent/ask and streams back a
 * UIMessageStream SSE response with a table of available tables.
 *
 * Try: "What tables exist?" or press Enter to submit.
 */
export const Default: Story = {
  args: {
    placeholder: "Ask anything about your data…",
  },
};

/**
 * Scoped to a specific database — the "production" database name is sent
 * with each request body.
 */
export const DatabaseScoped: Story = {
  args: {
    database: "production",
    placeholder: "Ask about production data…",
  },
};

/**
 * Compact height — useful for embedding in a side panel or drawer.
 */
export const Compact: Story = {
  args: {
    style: { height: 400 },
  },
};
