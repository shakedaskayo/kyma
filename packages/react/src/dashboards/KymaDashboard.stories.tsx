import type { Meta, StoryObj } from "@storybook/react";
import { KymaDashboard } from "./KymaDashboard";

const meta: Meta<typeof KymaDashboard> = {
  title: "Components/KymaDashboard",
  component: KymaDashboard,
  argTypes: {
    dashboardId: {
      control: "text",
      description: "ID of the dashboard to load",
    },
    editable: {
      control: "boolean",
      description: "Allow panel add/edit/delete and drag-to-reorder",
    },
  },
  args: {
    dashboardId: "d1",
    editable: false,
    style: { height: "100vh" },
  },
};

export default meta;
type Story = StoryObj<typeof KymaDashboard>;

/**
 * Read-only view: 5-panel "Service Health" dashboard with chart, stat, markdown,
 * and table panels loaded from the fixture.
 */
export const Default: Story = {
  args: {
    editable: false,
  },
};

/**
 * Editable mode: panel drag-resize and the Add Panel / Edit / Save controls
 * are enabled. Saves are routed to the fixture fetch (no-op offline).
 */
export const Editable: Story = {
  args: {
    editable: true,
  },
};

/**
 * Custom time range override: the host passes a 7-day window regardless of
 * the dashboard's stored preset.
 */
export const CustomTimeRange: Story = {
  args: {
    editable: false,
    timeRange: { preset: "7d" },
  },
};
