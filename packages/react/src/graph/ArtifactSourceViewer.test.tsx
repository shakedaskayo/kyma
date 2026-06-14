import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { KymaProvider } from "../provider/KymaProvider";
import { ArtifactSourceViewer } from "./ArtifactSourceViewer";

function windowResponse(offset: number, content: string, eof: boolean, size: number) {
  return new Response(
    JSON.stringify({
      object_path: "artifacts/t/logs/build.log",
      size_bytes: size,
      offset,
      returned_bytes: content.length,
      eof,
      content,
    }),
    { status: 200, headers: new Headers({ "content-type": "application/json" }) },
  );
}

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("ArtifactSourceViewer", () => {
  it("loads the first window and pages with Load more until eof", async () => {
    const fetchMock = vi.fn().mockImplementation((url: string) => {
      if (url.includes("/v1/artifacts/by-path")) {
        const m = url.match(/offset=(\d+)/);
        const offset = m ? Number(m[1]) : 0;
        return Promise.resolve(
          offset === 0
            ? windowResponse(0, "first-chunk", false, 23)
            : windowResponse(11, "second-chunk", true, 23),
        );
      }
      return Promise.resolve(new Response("", { status: 404 }));
    });
    vi.stubGlobal("fetch", fetchMock);

    render(
      <KymaProvider endpoint="https://kyma.test" auth={{ token: "tok" }}>
        <ArtifactSourceViewer path="artifacts/t/logs/build.log" />
      </KymaProvider>,
    );

    await waitFor(() =>
      expect(screen.getByText((c) => c.includes("first-chunk"))).toBeTruthy(),
    );

    fireEvent.click(screen.getByRole("button", { name: /load more/i }));

    await waitFor(() =>
      expect(screen.getByText((c) => c.includes("first-chunksecond-chunk"))).toBeTruthy(),
    );
    expect(screen.queryByRole("button", { name: /load more/i })).toBeNull();
  });

  it("shows an error with retry when the fetch fails", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response("nope", { status: 500 })));

    render(
      <KymaProvider endpoint="https://kyma.test" auth={{ token: "tok" }}>
        <ArtifactSourceViewer path="artifacts/t/logs/build.log" />
      </KymaProvider>,
    );

    await waitFor(() => expect(screen.getByRole("button", { name: /retry/i })).toBeTruthy());
  });
});
