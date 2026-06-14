import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { Markdown } from "./markdown";

afterEach(cleanup);

describe("Markdown", () => {
  it("renders headings, bold, inline code, links, lists and fenced code", () => {
    const md = [
      "# Title",
      "",
      "Some **bold** and `code` and a [link](https://x.test).",
      "",
      "- one",
      "- two",
      "",
      "```",
      "fenced text",
      "```",
    ].join("\n");
    render(<Markdown source={md} />);

    expect(screen.getByRole("heading", { level: 1, name: "Title" })).toBeTruthy();
    expect(screen.getByText("bold").tagName).toBe("STRONG");
    expect(screen.getByText("code").tagName).toBe("CODE");
    const link = screen.getByRole("link", { name: "link" });
    expect(link.getAttribute("href")).toBe("https://x.test");
    expect(screen.getByText("one")).toBeTruthy();
    expect(screen.getByText("two")).toBeTruthy();
    expect(screen.getByText("fenced text")).toBeTruthy();
  });
});
