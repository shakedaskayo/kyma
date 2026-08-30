import type { Meta, StoryObj } from "@storybook/react";
import { PensieveQueryEditor } from "./PensieveQueryEditor";

const meta: Meta<typeof PensieveQueryEditor> = {
  title: "Components/PensieveQueryEditor",
  component: PensieveQueryEditor,
  argTypes: {
    language: {
      control: { type: "select" },
      options: ["kql", "sql"],
      description: "Query language",
    },
    showSchemaBrowser: {
      control: "boolean",
      description: "Show schema browser panel",
    },
    showResults: {
      control: "boolean",
      description: "Show results grid below editor",
    },
    showChart: {
      control: "boolean",
      description: "Show chart panel when results are plottable",
    },
    showTimeRange: {
      control: "boolean",
      description: "Show time-range picker in toolbar",
    },
    readOnly: {
      control: "boolean",
      description: "Editor and results are read-only; Run button is hidden",
    },
    defaultQuery: {
      control: "text",
      description: "Initial query text",
    },
  },
  args: {
    language: "kql",
    showSchemaBrowser: true,
    showResults: true,
    showChart: true,
    showTimeRange: true,
    readOnly: false,
    defaultQuery: "events | take 10",
    style: { height: "100vh" },
  },
};

export default meta;
type Story = StoryObj<typeof PensieveQueryEditor>;

/**
 * Default editor: KQL mode, schema browser open, run to see 8 rows of fixture data.
 */
export const Default: Story = {
  args: {},
};

/**
 * Read-only view: the Run button is hidden and the editor is locked.
 * Useful for embedding a "view query" pane in a dashboard or drill-through.
 */
export const ReadOnly: Story = {
  args: {
    readOnly: true,
    defaultQuery: "events | where level == 'error' | take 20",
  },
};

/**
 * No schema browser — more compact layout for space-constrained host apps.
 */
export const NoSchemaBrowser: Story = {
  args: {
    showSchemaBrowser: false,
  },
};

/**
 * SQL mode — uses Monaco's built-in SQL grammar; time-range injection is
 * disabled and the user writes their own WHERE clause.
 */
export const SqlMode: Story = {
  args: {
    language: "sql",
    showTimeRange: false,
    defaultQuery: "SELECT ts, level, service, msg FROM events LIMIT 10",
  },
};

/**
 * Editor only — no results or chart, just the text input + schema browser.
 */
export const EditorOnly: Story = {
  args: {
    showResults: false,
    showChart: false,
  },
};
