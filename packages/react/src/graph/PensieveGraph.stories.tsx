import type { Meta, StoryObj } from "@storybook/react";
import { KymaGraph } from "./KymaGraph";
import type { LayoutAlgorithm } from "@kyma-ai/client";

const meta: Meta<typeof KymaGraph> = {
  title: "Components/KymaGraph",
  component: KymaGraph,
  argTypes: {
    layout: {
      control: { type: "select" },
      options: ["force", "tree", "grid", "radial"] satisfies LayoutAlgorithm[],
      description: "Graph layout algorithm",
    },
    sidebar: {
      control: "boolean",
      description: "Show sidebar (controls + inspector)",
    },
    toolbar: {
      control: "boolean",
      description: "Show toolbar (same DOM region as sidebar)",
    },
    height: {
      control: { type: "text" },
      description: "Container height",
    },
  },
  args: {
    layout: "force",
    sidebar: true,
    toolbar: true,
    height: "100vh",
  },
};

export default meta;
type Story = StoryObj<typeof KymaGraph>;

/**
 * Default view: loads from the fixture /v1/graph endpoint and renders a 12-node
 * service-mesh graph with the sidebar open.
 */
export const Default: Story = {
  args: {},
};

/**
 * No sidebar — full-bleed canvas, useful for embedding inside a host panel.
 */
export const NoSidebar: Story = {
  args: {
    sidebar: false,
    toolbar: false,
  },
};

/**
 * Pre-seeded with a focus query — the graph starts with the search bar
 * pre-filled and highlights matching nodes immediately.
 */
export const WithFocusQuery: Story = {
  args: {
    focusQuery: "auth",
  },
};

/**
 * Radial layout — alternative algorithm that places nodes in a radial tree.
 */
export const RadialLayout: Story = {
  args: {
    layout: "radial",
  },
};

/**
 * Explicit graph targets — renders only the "services" named graph
 * instead of all graphs.
 */
export const ExplicitGraph: Story = {
  args: {
    graphs: [{ graph: "services" }],
  },
};
