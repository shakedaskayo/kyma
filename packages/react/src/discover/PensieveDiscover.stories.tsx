import type { Meta, StoryObj } from "@storybook/react";
import { KymaDiscover } from "./KymaDiscover";

const meta: Meta<typeof KymaDiscover> = {
  title: "Components/KymaDiscover",
  component: KymaDiscover,
  argTypes: {
    defaultQuery: {
      control: "text",
      description: "Initial search query (discover grammar)",
    },
  },
  args: {
    style: { height: "100vh" },
  },
};

export default meta;
type Story = StoryObj<typeof KymaDiscover>;

/**
 * Default Discover view. The fixture fetch serves NDJSON frames on
 * /v1/explore/search — rows appear after clicking the search button.
 */
export const Default: Story = {
  args: {},
};

/**
 * Pre-seeded with a search query — the page loads with "error" already in
 * the search bar so you can click Run and see filtered results immediately.
 */
export const WithQuery: Story = {
  args: {
    defaultQuery: "error",
  },
};

/**
 * Scoped to a specific source — renders with the sources rail pre-filtered
 * to "production.events".
 */
export const ScopedToEvents: Story = {
  args: {
    scope: { kind: "sources", sources: ["production.events"] },
  },
};
