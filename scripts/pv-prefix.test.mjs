/**
 * pv-prefix.test.mjs — tests for the Tailwind pv- prefix codemod
 * Run: node --test scripts/pv-prefix.test.mjs
 *      or: node --test scripts/
 */

import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { prefixClassList, prefixToken, rewriteFile } from "./pv-prefix.mjs";

// ─── prefixToken table-driven tests ──────────────────────────────────────────

describe("prefixToken", () => {
  /** @type {[string, string][]} [input, expected] */
  const cases = [
    // Plain utilities
    ["flex", "pv-flex"],
    ["px-2", "pv-px-2"],
    ["items-center", "pv-items-center"],
    ["rounded-md", "pv-rounded-md"],
    ["text-sm", "pv-text-sm"],

    // Variant chains
    ["hover:flex", "hover:pv-flex"],
    ["md:px-2", "md:pv-px-2"],
    ["md:dark:px-2", "md:dark:pv-px-2"],
    ["focus-visible:ring-2", "focus-visible:pv-ring-2"],
    ["lg:hover:bg-accent", "lg:hover:pv-bg-accent"],

    // Negative utilities
    ["-mt-2", "-pv-mt-2"],
    ["-mx-4", "-pv-mx-4"],

    // Variant + negative
    ["md:-mt-2", "md:-pv-mt-2"],
    ["hover:-translate-y-1", "hover:-pv-translate-y-1"],

    // Important modifier
    ["!px-2", "!pv-px-2"],
    ["!font-bold", "!pv-font-bold"],

    // Arbitrary values (brackets in utility part)
    ["w-[13px]", "pv-w-[13px]"],
    ["bg-[hsl(var(--pensieve-x))]", "pv-bg-[hsl(var(--pensieve-x))]"],
    ["text-[#ff0000]", "pv-text-[#ff0000]"],
    ["p-[calc(1rem+2px)]", "pv-p-[calc(1rem+2px)]"],

    // Arbitrary VARIANTS (brackets in variant part)
    ["[&>svg]:px-2", "[&>svg]:pv-px-2"],
    ["data-[state=open]:flex", "data-[state=open]:pv-flex"],
    ["[&_svg]:size-4", "[&_svg]:pv-size-4"],
    ["data-[highlighted]:bg-accent", "data-[highlighted]:pv-bg-accent"],

    // group / peer names
    ["group", "pv-group"],
    ["peer", "pv-peer"],
    ["group-hover:flex", "group-hover:pv-flex"],
    ["peer-focus:ring-2", "peer-focus:pv-ring-2"],

    // Idempotent — already prefixed
    ["pv-flex", "pv-flex"],
    ["hover:pv-flex", "hover:pv-flex"],
    ["md:dark:pv-px-2", "md:dark:pv-px-2"],
    ["-pv-mt-2", "-pv-mt-2"],
    ["!pv-px-2", "!pv-px-2"],

    // Preserve exact sentinel tokens
    ["pensieve-root", "pensieve-root"],
    ["pensieve-dark", "pensieve-dark"],

    // CSS custom-property names — non-class-like, preserve
    ["--pensieve-accent", "--pensieve-accent"],
    ["--primary", "--primary"],

    // Already prefixed variants
    ["[&>svg]:pv-px-2", "[&>svg]:pv-px-2"],
    ["data-[state=open]:pv-flex", "data-[state=open]:pv-flex"],

    // Opacity modifier (slash)
    ["bg-primary/90", "pv-bg-primary/90"],
    ["hover:bg-primary/90", "hover:pv-bg-primary/90"],

    // Responsive + dark + arbitrary
    ["dark:[&>svg]:size-3.5", "dark:[&>svg]:pv-size-3.5"],

    // Tokens with $ or { — non-class-like, preserve
    ["${someVar}", "${someVar}"],
    ["{some}", "{some}"],
  ];

  for (const [input, expected] of cases) {
    it(`prefixToken("${input}") → "${expected}"`, () => {
      assert.equal(prefixToken(input), expected);
    });
  }
});

// ─── prefixClassList table-driven tests ──────────────────────────────────────

describe("prefixClassList", () => {
  /** @type {[string, string][]} */
  const cases = [
    // Multiple plain tokens
    ["flex px-2 hover:bg-accent", "pv-flex pv-px-2 hover:pv-bg-accent"],

    // Mixed variants
    [
      "inline-flex items-center justify-center gap-2 rounded-md",
      "pv-inline-flex pv-items-center pv-justify-center pv-gap-2 pv-rounded-md",
    ],

    // Negative tokens in a list
    ["-mt-2 -mx-4", "-pv-mt-2 -pv-mx-4"],

    // Already prefixed — idempotent full class string
    [
      "pv-flex pv-px-2 hover:pv-bg-accent",
      "pv-flex pv-px-2 hover:pv-bg-accent",
    ],

    // Mixed: some prefixed, some not (should not double-prefix)
    ["flex pv-px-2 hover:bg-accent", "pv-flex pv-px-2 hover:pv-bg-accent"],

    // Preserves multi-whitespace / newlines
    ["flex\n  px-2", "pv-flex\n  pv-px-2"],

    // Sentinel preservation within a list
    ["pensieve-root flex", "pensieve-root pv-flex"],
    ["bg-accent pensieve-dark text-sm", "pv-bg-accent pensieve-dark pv-text-sm"],

    // Important + variant
    ["!px-2 hover:!font-bold", "!pv-px-2 hover:!pv-font-bold"],

    // Object key style strings (clsx map keys)
    ["px-2 flex", "pv-px-2 pv-flex"],

    // Arbitrary value with CSS variable
    ["shadow-[inset_0_1px_0_hsl(0_0%_100%/0.10)]", "pv-shadow-[inset_0_1px_0_hsl(0_0%_100%/0.10)]"],
  ];

  for (const [input, expected] of cases) {
    it(`prefixClassList(${JSON.stringify(input)})`, () => {
      assert.equal(prefixClassList(input), expected);
    });
  }
});

// ─── rewriteFile — file-level integration tests ───────────────────────────────

describe("rewriteFile – className attribute", () => {
  it("rewrites plain className string attribute", () => {
    const src = `<div className="flex px-2 hover:bg-accent" />`;
    const { code, count } = rewriteFile(src, "test.tsx");
    assert.equal(count, 1);
    assert.ok(
      code.includes(`className="pv-flex pv-px-2 hover:pv-bg-accent"`),
      `Got: ${code}`
    );
  });

  it("rewrites className={\"...\"} JSX expression", () => {
    const src = `<div className={"flex items-center"} />`;
    const { code, count } = rewriteFile(src, "test.tsx");
    assert.equal(count, 1);
    assert.ok(code.includes(`className={"pv-flex pv-items-center"}`), `Got: ${code}`);
  });

  it("does not double-prefix already-prefixed classes", () => {
    const src = `<div className="pv-flex pv-px-2 hover:pv-bg-accent" />`;
    const { code, count } = rewriteFile(src, "test.tsx");
    assert.equal(count, 0);
    assert.equal(code, src);
  });

  it("preserves pensieve-root and pensieve-dark sentinels", () => {
    const src = `<div className="pensieve-root flex" />`;
    const { code } = rewriteFile(src, "test.tsx");
    assert.ok(code.includes("pensieve-root"), `Got: ${code}`);
    assert.ok(code.includes("pv-flex"), `Got: ${code}`);
    assert.ok(!code.includes("pv-pensieve-root"), `Should not prefix sentinel. Got: ${code}`);
  });
});

describe("rewriteFile – cn() / cva() / clsx() / twMerge()", () => {
  it("rewrites string arguments in cn()", () => {
    const src = `const x = cn("flex px-2", "hover:bg-accent")`;
    const { code, count } = rewriteFile(src, "test.tsx");
    assert.ok(count >= 2, `Expected >= 2 rewrites, got ${count}`);
    assert.ok(code.includes(`"pv-flex pv-px-2"`), `Got: ${code}`);
    assert.ok(code.includes(`"hover:pv-bg-accent"`), `Got: ${code}`);
  });

  it("rewrites string arguments in cva()", () => {
    const src = `const v = cva("inline-flex items-center", { variants: { size: { sm: "h-9 px-3" } } })`;
    const { code, count } = rewriteFile(src, "test.tsx");
    assert.ok(count >= 2, `Expected >= 2 rewrites, got ${count}`);
    assert.ok(code.includes(`"pv-inline-flex pv-items-center"`), `Got: ${code}`);
    assert.ok(code.includes(`"pv-h-9 pv-px-3"`), `Got: ${code}`);
  });

  it("rewrites object KEYS in clsx() map", () => {
    // clsx({"px-2": cond}) — the key is a class string
    const src = `const cls = clsx({"px-2": isActive, "flex items-center": true})`;
    const { code, count } = rewriteFile(src, "test.tsx");
    assert.ok(count >= 2, `Expected >= 2 rewrites, got ${count}`);
    assert.ok(code.includes(`"pv-px-2"`), `Got: ${code}`);
    assert.ok(code.includes(`"pv-flex pv-items-center"`), `Got: ${code}`);
  });

  it("rewrites string in conditional expression inside cn()", () => {
    const src = `const cls = cn(cond && "px-2 flex", "bg-accent")`;
    const { code } = rewriteFile(src, "test.tsx");
    assert.ok(code.includes(`"pv-px-2 pv-flex"`), `Got: ${code}`);
    assert.ok(code.includes(`"pv-bg-accent"`), `Got: ${code}`);
  });

  it("rewrites twMerge() arguments", () => {
    const src = `const c = twMerge("flex px-4", "md:px-2")`;
    const { code } = rewriteFile(src, "test.tsx");
    assert.ok(code.includes(`"pv-flex pv-px-4"`), `Got: ${code}`);
    assert.ok(code.includes(`"md:pv-px-2"`), `Got: ${code}`);
  });
});

describe("rewriteFile – template literals", () => {
  it("emits a warning for template literal with interpolation in className", () => {
    const src = "const x = <div className={`flex ${cond ? 'px-2' : 'px-4'}`} />";
    const { warnings } = rewriteFile(src, "test.tsx");
    assert.ok(
      warnings.some((w) => w.includes("template literal with interpolation")),
      `Expected template literal warning. Got: ${JSON.stringify(warnings)}`
    );
  });

  it("rewrites static segments around interpolations in className template literals", () => {
    // Static part "flex " before interpolation should be prefixed
    const src = "const x = <div className={`flex ${extra}`} />";
    const { code } = rewriteFile(src, "test.tsx");
    // "flex " static part should become "pv-flex "
    assert.ok(code.includes("pv-flex"), `Got: ${code}`);
  });

  it("rewrites static-only template literal className without warning", () => {
    const src = "const x = <div className={`flex px-2 hover:bg-accent`} />";
    const { code, warnings } = rewriteFile(src, "test.tsx");
    assert.equal(warnings.length, 0);
    assert.ok(code.includes("pv-flex"), `Got: ${code}`);
    assert.ok(code.includes("pv-px-2"), `Got: ${code}`);
  });
});

describe("rewriteFile – className={identifier} warning", () => {
  it("emits a warning for className={someVariable}", () => {
    const src = `<div className={someStyles} />`;
    const { warnings } = rewriteFile(src, "test.tsx");
    assert.ok(
      warnings.some((w) => w.includes("someStyles")),
      `Expected identifier warning. Got: ${JSON.stringify(warnings)}`
    );
  });

  it("does NOT warn for className={'...'} (string literal)", () => {
    const src = `<div className={'flex px-2'} />`;
    const { warnings } = rewriteFile(src, "test.tsx");
    const identWarns = warnings.filter((w) => w.includes("variable assignment not rewritten"));
    assert.equal(identWarns.length, 0, `Should not warn for string literal. Got: ${JSON.stringify(warnings)}`);
  });
});

describe("rewriteFile – real-world cva pattern (button.tsx style)", () => {
  it("rewrites a realistic cva() call with variants and negative/arbitrary values", () => {
    const src = `
const buttonVariants = cva(
  "inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-md text-sm font-medium ring-offset-background transition-[color,background-color,border-color,box-shadow,transform] duration-150 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 active:scale-[0.98] disabled:pointer-events-none disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:size-4 [&_svg]:shrink-0",
  {
    variants: {
      variant: {
        default: "bg-primary text-primary-foreground shadow-elev-1 shadow-[inset_0_1px_0_hsl(0_0%_100%/0.10)] hover:bg-primary/90",
        outline: "border border-border-strong bg-background hover:bg-accent hover:text-accent-foreground",
        ghost: "hover:bg-accent hover:text-accent-foreground",
      },
      size: {
        default: "h-10 px-4 py-2",
        sm: "h-9 rounded-md px-3",
        xs: "h-7 rounded-md px-2.5 text-xs [&_svg]:size-3.5",
        icon: "h-10 w-10",
      },
    },
    defaultVariants: { variant: "default", size: "default" },
  }
)
`;
    const { code, count } = rewriteFile(src, "button.tsx");
    assert.ok(count > 0, `Expected rewrites, got ${count}`);
    // Check a few key tokens
    assert.ok(code.includes("pv-inline-flex"), `Got: ${code}`);
    assert.ok(code.includes("pv-items-center"), `Got: ${code}`);
    assert.ok(code.includes("[&_svg]:pv-pointer-events-none"), `Got: ${code}`);
    assert.ok(code.includes("[&_svg]:pv-size-4"), `Got: ${code}`);
    assert.ok(code.includes("hover:pv-bg-primary/90"), `Got: ${code}`);
    assert.ok(code.includes("focus-visible:pv-ring-2"), `Got: ${code}`);
    // Idempotent — run twice, same result
    const { code: code2, count: count2 } = rewriteFile(code, "button.tsx");
    assert.equal(count2, 0, `Second pass should have 0 rewrites (idempotent), got ${count2}`);
    assert.equal(code2, code);
  });
});

describe("rewriteFile – negative and important variants", () => {
  it("handles -mt-2 and !px-2 correctly in className", () => {
    const src = `<div className="-mt-2 !px-2 md:-mx-4" />`;
    const { code } = rewriteFile(src, "test.tsx");
    assert.ok(code.includes("-pv-mt-2"), `Got: ${code}`);
    assert.ok(code.includes("!pv-px-2"), `Got: ${code}`);
    assert.ok(code.includes("md:-pv-mx-4"), `Got: ${code}`);
  });
});

describe("rewriteFile – arbitrary variant tokens", () => {
  it("rewrites [&>svg]:px-2 correctly", () => {
    const src = `<div className="[&>svg]:px-2 data-[state=open]:flex" />`;
    const { code } = rewriteFile(src, "test.tsx");
    assert.ok(code.includes("[&>svg]:pv-px-2"), `Got: ${code}`);
    assert.ok(code.includes("data-[state=open]:pv-flex"), `Got: ${code}`);
  });
});
