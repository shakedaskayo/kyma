/**
 * Storybook preview — global decorator + fixture fetch + theme toolbar.
 *
 * FIXTURE FETCH
 * =============
 * We override window.fetch once inside this module (the Storybook sandbox
 * iframe). This is intentional and acceptable here: every story runs in an
 * isolated iframe that is fully controlled by Storybook, so overriding fetch
 * does not pollute the host page. The override routes known Pensieve API paths to
 * in-memory fixture data so no real server is required.
 *
 * URL routing covers:
 *   /v1/capabilities        → capabilities fixture (JSON)
 *   /v1/graph               → graph list fixture  (JSON)
 *   /v1/graph/:g/export     → positioned export page (JSON)
 *   /v1/graph/:db/:g/overview  → graph fixture    (JSON)
 *   /v1/graph/:db/:g/neighbors → empty neighbors  (JSON)
 *   /v1/graph/:db/:g/search    → search fixture   (JSON)
 *   /v1/catalog/schema      → schema fixture      (JSON)
 *   /v1/query               → NDJSON rows         (NDJSON stream)
 *   /v1/explore/search      → discover NDJSON     (NDJSON stream)
 *   /v1/dashboards          → dashboards list     (JSON)
 *   /v1/dashboards/:id      → dashboard with panels (JSON)
 *   /v1/agent/ask           → SSE UIMessageStream (text/event-stream)
 */

import React from "react";
import type { Preview, Decorator } from "@storybook/react";
// Import from the package root so tooling that maps the package entry to the
// built bundle resolves the SAME PensieveProvider instance the components use —
// a deep-path import compiles a second copy whose React context the bundled
// components can't see.
import { PensieveProvider, pensieveDark, pensieveLight } from "../src";
import type { PensieveTheme } from "../src/theme/tokens";

// fixtures
import { CAPABILITIES_FIXTURE } from "../src/__fixtures__/capabilities";
import { GRAPH_EXPORT_FIXTURE, GRAPH_FIXTURE, GRAPH_LIST_FIXTURE, NEIGHBORS_FIXTURE, SEARCH_FIXTURE } from "../src/__fixtures__/graph";
import { SCHEMA_FIXTURE } from "../src/__fixtures__/schema";
import { QUERY_ROWS, makeNdjsonBody } from "../src/__fixtures__/query";
import { DISCOVER_FRAMES } from "../src/__fixtures__/discover";
import { DASHBOARDS_FIXTURE, DASHBOARD_WITH_PANELS_FIXTURE } from "../src/__fixtures__/dashboards";
import { buildSseResponse, CANNED_TABLES_RESPONSE } from "../src/__fixtures__/agent";

import "../src/styles/storybook.css";

// ── Install fixture fetch override ─────────────────────────────────────────────

const realFetch = window.fetch.bind(window);

function jsonResp(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: new Headers({ "content-type": "application/json" }),
  });
}

function ndjsonResp(rows: object[]): Response {
  const body = makeNdjsonBody(rows);
  return new Response(body, {
    status: 200,
    headers: new Headers({ "content-type": "application/x-ndjson" }),
  });
}

function discoverStream(frames: object[]): Response {
  const enc = new TextEncoder();
  const readable = new ReadableStream<Uint8Array>({
    start(ctrl) {
      for (const f of frames) {
        ctrl.enqueue(enc.encode(JSON.stringify(f) + "\n"));
      }
      ctrl.close();
    },
  });
  return new Response(readable, {
    status: 200,
    headers: new Headers({ "content-type": "application/x-ndjson" }),
  });
}

window.fetch = (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
  const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;

  if (url.includes("/v1/capabilities")) {
    return Promise.resolve(jsonResp(CAPABILITIES_FIXTURE));
  }

  if (url.includes("/v1/catalog/schema")) {
    return Promise.resolve(jsonResp(SCHEMA_FIXTURE));
  }

  if (url.includes("/v1/graph")) {
    // /v1/graph/:name/export — paged, server-laid-out graph (GraphView's load path)
    if (url.includes("/export")) {
      return Promise.resolve(jsonResp(GRAPH_EXPORT_FIXTURE));
    }
    // /v1/graph/:db/:name/neighbors
    if (url.includes("/neighbors")) {
      return Promise.resolve(jsonResp(NEIGHBORS_FIXTURE));
    }
    // /v1/graph/:db/:name/search
    if (url.includes("/search")) {
      return Promise.resolve(jsonResp(SEARCH_FIXTURE));
    }
    // /v1/graph/:db/:name/overview
    if (url.includes("/overview")) {
      return Promise.resolve(jsonResp(GRAPH_FIXTURE));
    }
    // /v1/graph (list)
    return Promise.resolve(jsonResp(GRAPH_LIST_FIXTURE));
  }

  if (url.includes("/v1/query")) {
    return Promise.resolve(ndjsonResp(QUERY_ROWS));
  }

  if (url.includes("/v1/explore/search")) {
    return Promise.resolve(discoverStream(DISCOVER_FRAMES));
  }

  if (url.includes("/v1/dashboards")) {
    // /v1/dashboards/:id
    const idMatch = /\/v1\/dashboards\/([^/?]+)/.exec(url);
    if (idMatch) {
      return Promise.resolve(jsonResp(DASHBOARD_WITH_PANELS_FIXTURE));
    }
    return Promise.resolve(jsonResp(DASHBOARDS_FIXTURE));
  }

  if (url.includes("/v1/agent/ask")) {
    return Promise.resolve(buildSseResponse(CANNED_TABLES_RESPONSE));
  }

  // Unknown URL — fall through to real fetch (will fail gracefully offline)
  return realFetch(input, init);
};

// ── Theme toolbar ───────────────────────────────────────────────────────────────

export const globalTypes = {
  pensieveTheme: {
    name: "Theme",
    description: "Pensieve colour theme",
    defaultValue: "dark",
    toolbar: {
      icon: "paintbrush",
      items: [
        { value: "dark",   title: "Dark (default)" },
        { value: "light",  title: "Light" },
        { value: "custom", title: "Custom (teal accent)" },
      ],
      showName: true,
      dynamicTitle: true,
    },
  },
};

function resolveTheme(value: string): Partial<PensieveTheme> {
  if (value === "light") return pensieveLight;
  if (value === "custom") return { ...pensieveDark, accent: "185 80% 40%", primary: "185 80% 40%" };
  return pensieveDark;
}

// ── Global decorator ────────────────────────────────────────────────────────────

const withPensieveProvider: Decorator = (Story, context) => {
  const themeKey: string = (context.globals as Record<string, string>)["pensieveTheme"] ?? "dark";
  const theme = resolveTheme(themeKey);

  return (
    <div style={{ height: "100vh", overflow: "hidden", background: "#0a0f14" }}>
      <PensieveProvider
        endpoint="https://pensieve.demo"
        auth={{ token: "storybook" }}
        theme={theme}
      >
        <Story />
      </PensieveProvider>
    </div>
  );
};

const preview: Preview = {
  decorators: [withPensieveProvider],
  parameters: {
    layout: "fullscreen",
  },
};

export default preview;
