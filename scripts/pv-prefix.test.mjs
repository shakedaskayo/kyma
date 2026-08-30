/**
 * ky-prefix.test.mjs — tests for the Tailwind ky- prefix codemod
 * Run: node --test scripts/ky-prefix.test.mjs
 *      or: node --test scripts/
 */

import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { prefixClassList, prefixToken, rewriteFile } from "./ky-prefix.mjs";

// ─── prefixToken table-driven tests ──────────────────────────────────────────

describe("prefixToken", () => {
  /** @type {[string, string][]} [input, expected] */
  const cases = [
    // Plain utilities
    ["flex", "ky-flex"],
    ["px-2", "ky-px-2"],
    ["items-center", "ky-items-center"],
    ["rounded-md", "ky-rounded-md"],
    ["text-sm", "ky-text-sm"],

    // Variant chains
    ["hover:flex", "hover:ky-flex"],
    ["md:px-2", "md:ky-px-2"],
    ["md:dark:px-2", "md:dark:ky-px-2"],
    ["focus-visible:ring-2", "focus-visible:ky-ring-2"],
    ["lg:hover:bg-accent", "lg:hover:ky-bg-accent"],

    // Negative utilities
    ["-mt-2", "-ky-mt-2"],
    ["-mx-4", "-ky-mx-4"],

    // Variant + negative
    ["md:-mt-2", "md:-ky-mt-2"],
    ["hover:-translate-y-1", "hover:-ky-translate-y-1"],

    // Important modifier
    ["!px-2", "!ky-px-2"],
    ["!font-bold", "!ky-font-bold"],

    // Arbitrary values (brackets in utility part)
    ["w-[13px]", "ky-w-[13px]"],
    ["bg-[hsl(var(--kyma-x))]", "ky-bg-[hsl(var(--kyma-x))]"],
    ["text-[#ff0000]", "ky-text-[#ff0000]"],
    ["p-[calc(1rem+2px)]", "ky-p-[calc(1rem+2px)]"],

    // Arbitrary VARIANTS (brackets in variant part)
    ["[&>svg]:px-2", "[&>svg]:ky-px-2"],
    ["data-[state=open]:flex", "data-[state=open]:ky-flex"],
    ["[&_svg]:size-4", "[&_svg]:ky-size-4"],
    ["data-[highlighted]:bg-accent", "data-[highlighted]:ky-bg-accent"],

    // group / peer names
    ["group", "ky-group"],
    ["peer", "ky-peer"],
    ["group-hover:flex", "group-hover:ky-flex"],
    ["peer-focus:ring-2", "peer-focus:ky-ring-2"],

    // Idempotent — already prefixed
    ["ky-flex", "ky-flex"],
    ["hover:ky-flex", "hover:ky-flex"],
    ["md:dark:ky-px-2", "md:dark:ky-px-2"],
    ["-ky-mt-2", "-ky-mt-2"],
    ["!ky-px-2", "!ky-px-2"],

    // Preserve exact sentinel tokens
    ["kyma-root", "kyma-root"],
    ["kyma-dark", "kyma-dark"],

    // CSS custom-property names — non-class-like, preserve
    ["--kyma-accent", "--kyma-accent"],
    ["--primary", "--primary"],

    // Already prefixed variants
    ["[&>svg]:ky-px-2", "[&>svg]:ky-px-2"],
    ["data-[state=open]:ky-flex", "data-[state=open]:ky-flex"],

    // Opacity modifier (slash)
    ["bg-primary/90", "ky-bg-primary/90"],
    ["hover:bg-primary/90", "hover:ky-bg-primary/90"],

    // Responsive + dark + arbitrary
    ["dark:[&>svg]:size-3.5", "dark:[&>svg]:ky-size-3.5"],

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
    ["flex px-2 hover:bg-accent", "ky-flex ky-px-2 hover:ky-bg-accent"],

    // Mixed variants
    [
      "inline-flex items-center justify-center gap-2 rounded-md",
      "ky-inline-flex ky-items-center ky-justify-center ky-gap-2 ky-rounded-md",
    ],

    // Negative tokens in a list
    ["-mt-2 -mx-4", "-ky-mt-2 -ky-mx-4"],

    // Already prefixed — idempotent full class string
    [
      "ky-flex ky-px-2 hover:ky-bg-accent",
      "ky-flex ky-px-2 hover:ky-bg-accent",
    ],

    // Mixed: some prefixed, some not (should not double-prefix)
    ["flex ky-px-2 hover:bg-accent", "ky-flex ky-px-2 hover:ky-bg-accent"],

    // Preserves multi-whitespace / newlines
    ["flex\n  px-2", "ky-flex\n  ky-px-2"],

    // Sentinel preservation within a list
    ["kyma-root flex", "kyma-root ky-flex"],
    ["bg-accent kyma-dark text-sm", "ky-bg-accent kyma-dark ky-text-sm"],

    // Important + variant
    ["!px-2 hover:!font-bold", "!ky-px-2 hover:!ky-font-bold"],

    // Object key style strings (clsx map keys)
    ["px-2 flex", "ky-px-2 ky-flex"],

    // Arbitrary value with CSS variable
    ["shadow-[inset_0_1px_0_hsl(0_0%_100%/0.10)]", "ky-shadow-[inset_0_1px_0_hsl(0_0%_100%/0.10)]"],
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
      code.includes(`className="ky-flex ky-px-2 hover:ky-bg-accent"`),
      `Got: ${code}`
    );
  });

  it("rewrites className={\"...\"} JSX expression", () => {
    const src = `<div className={"flex items-center"} />`;
    const { code, count } = rewriteFile(src, "test.tsx");
    assert.equal(count, 1);
    assert.ok(code.includes(`className={"ky-flex ky-items-center"}`), `Got: ${code}`);
  });

  it("does not double-prefix already-prefixed classes", () => {
    const src = `<div className="ky-flex ky-px-2 hover:ky-bg-accent" />`;
    const { code, count } = rewriteFile(src, "test.tsx");
    assert.equal(count, 0);
    assert.equal(code, src);
  });

  it("preserves kyma-root and kyma-dark sentinels", () => {
    const src = `<div className="kyma-root flex" />`;
    const { code } = rewriteFile(src, "test.tsx");
    assert.ok(code.includes("kyma-root"), `Got: ${code}`);
    assert.ok(code.includes("ky-flex"), `Got: ${code}`);
    assert.ok(!code.includes("ky-kyma-root"), `Should not prefix sentinel. Got: ${code}`);
  });
});

describe("rewriteFile – cn() / cva() / clsx() / twMerge()", () => {
  it("rewrites string arguments in cn()", () => {
    const src = `const x = cn("flex px-2", "hover:bg-accent")`;
    const { code, count } = rewriteFile(src, "test.tsx");
    assert.ok(count >= 2, `Expected >= 2 rewrites, got ${count}`);
    assert.ok(code.includes(`"ky-flex ky-px-2"`), `Got: ${code}`);
    assert.ok(code.includes(`"hover:ky-bg-accent"`), `Got: ${code}`);
  });

  it("rewrites string arguments in cva()", () => {
    const src = `const v = cva("inline-flex items-center", { variants: { size: { sm: "h-9 px-3" } } })`;
    const { code, count } = rewriteFile(src, "test.tsx");
    assert.ok(count >= 2, `Expected >= 2 rewrites, got ${count}`);
    assert.ok(code.includes(`"ky-inline-flex ky-items-center"`), `Got: ${code}`);
    assert.ok(code.includes(`"ky-h-9 ky-px-3"`), `Got: ${code}`);
  });

  it("rewrites object KEYS in clsx() map", () => {
    // clsx({"px-2": cond}) — the key is a class string
    const src = `const cls = clsx({"px-2": isActive, "flex items-center": true})`;
    const { code, count } = rewriteFile(src, "test.tsx");
    assert.ok(count >= 2, `Expected >= 2 rewrites, got ${count}`);
    assert.ok(code.includes(`"ky-px-2"`), `Got: ${code}`);
    assert.ok(code.includes(`"ky-flex ky-items-center"`), `Got: ${code}`);
  });

  it("rewrites string in conditional expression inside cn()", () => {
    const src = `const cls = cn(cond && "px-2 flex", "bg-accent")`;
    const { code } = rewriteFile(src, "test.tsx");
    assert.ok(code.includes(`"ky-px-2 ky-flex"`), `Got: ${code}`);
    assert.ok(code.includes(`"ky-bg-accent"`), `Got: ${code}`);
  });

  it("rewrites twMerge() arguments", () => {
    const src = `const c = twMerge("flex px-4", "md:px-2")`;
    const { code } = rewriteFile(src, "test.tsx");
    assert.ok(code.includes(`"ky-flex ky-px-4"`), `Got: ${code}`);
    assert.ok(code.includes(`"md:ky-px-2"`), `Got: ${code}`);
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
    // "flex " static part should become "ky-flex "
    assert.ok(code.includes("ky-flex"), `Got: ${code}`);
  });

  it("rewrites static-only template literal className without warning", () => {
    const src = "const x = <div className={`flex px-2 hover:bg-accent`} />";
    const { code, warnings } = rewriteFile(src, "test.tsx");
    assert.equal(warnings.length, 0);
    assert.ok(code.includes("ky-flex"), `Got: ${code}`);
    assert.ok(code.includes("ky-px-2"), `Got: ${code}`);
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
    assert.ok(code.includes("ky-inline-flex"), `Got: ${code}`);
    assert.ok(code.includes("ky-items-center"), `Got: ${code}`);
    assert.ok(code.includes("[&_svg]:ky-pointer-events-none"), `Got: ${code}`);
    assert.ok(code.includes("[&_svg]:ky-size-4"), `Got: ${code}`);
    assert.ok(code.includes("hover:ky-bg-primary/90"), `Got: ${code}`);
    assert.ok(code.includes("focus-visible:ky-ring-2"), `Got: ${code}`);
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
    assert.ok(code.includes("-ky-mt-2"), `Got: ${code}`);
    assert.ok(code.includes("!ky-px-2"), `Got: ${code}`);
    assert.ok(code.includes("md:-ky-mx-4"), `Got: ${code}`);
  });
});

describe("rewriteFile – arbitrary variant tokens", () => {
  it("rewrites [&>svg]:px-2 correctly", () => {
    const src = `<div className="[&>svg]:px-2 data-[state=open]:flex" />`;
    const { code } = rewriteFile(src, "test.tsx");
    assert.ok(code.includes("[&>svg]:ky-px-2"), `Got: ${code}`);
    assert.ok(code.includes("data-[state=open]:ky-flex"), `Got: ${code}`);
  });
});
