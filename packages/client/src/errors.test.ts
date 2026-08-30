import { describe, expect, it } from "vitest";
import { PensieveApiError, PensieveAuthError, errorFromResponse } from "./errors";

describe("errorFromResponse", () => {
  it("maps 4xx/5xx with JSON body to PensieveApiError", async () => {
    const res = new Response(JSON.stringify({ error: "bad query" }), {
      status: 400,
      headers: { "content-type": "application/json", "x-request-id": "r-1" },
    });
    const err = await errorFromResponse(res);
    expect(err).toBeInstanceOf(PensieveApiError);
    expect(err.status).toBe(400);
    expect(err.message).toContain("bad query");
    expect(err.requestId).toBe("r-1");
  });

  it("maps 401/403 to PensieveAuthError (subtype of PensieveApiError)", async () => {
    const err = await errorFromResponse(new Response("nope", { status: 401 }));
    expect(err).toBeInstanceOf(PensieveAuthError);
    expect(err).toBeInstanceOf(PensieveApiError);
  });

  it("truncates long text bodies", async () => {
    const err = await errorFromResponse(new Response("x".repeat(500), { status: 500 }));
    expect(err.message.length).toBeLessThan(300);
  });

  // Fix 4: code field on PensieveApiError
  it("populates code from JSON body code field", async () => {
    const res = new Response(JSON.stringify({ error: "x", code: "SCOPE_DENIED" }), {
      status: 403,
      headers: { "content-type": "application/json" },
    });
    const err = await errorFromResponse(res);
    expect(err.code).toBe("SCOPE_DENIED");
  });

  it("code is undefined when not in response body", async () => {
    const res = new Response(JSON.stringify({ error: "bad query" }), {
      status: 400,
      headers: { "content-type": "application/json" },
    });
    const err = await errorFromResponse(res);
    expect(err.code).toBeUndefined();
  });
});
