import { describe, expect, it } from "vitest";
import { dayGroup, formatTokens, parsePathRef, splitMarkdownBlocks } from "./util";

describe("splitMarkdownBlocks", () => {
  it("splits paragraphs at blank lines", () => {
    expect(splitMarkdownBlocks("a\nb\n\nc\n\n\nd")).toEqual(["a\nb", "c", "d"]);
  });

  it("keeps fenced code with blank lines intact", () => {
    const md = "Intro\n\n```rust\nfn a() {}\n\nfn b() {}\n```\n\nAfter";
    expect(splitMarkdownBlocks(md)).toEqual(["Intro", "```rust\nfn a() {}\n\nfn b() {}\n```", "After"]);
  });

  it("handles an unterminated fence while streaming", () => {
    expect(splitMarkdownBlocks("x\n\n```ts\nconst a = 1;\n\nconst b")).toEqual(["x", "```ts\nconst a = 1;\n\nconst b"]);
  });

  it("only closes a fence with a matching marker", () => {
    const md = "````md\n```\ninner\n```\n\nstill code\n````\n\nend";
    expect(splitMarkdownBlocks(md)).toEqual(["````md\n```\ninner\n```\n\nstill code\n````", "end"]);
  });
});

describe("parsePathRef", () => {
  it("parses paths with lines", () => {
    expect(parsePathRef("src/main.rs:42")).toEqual({ path: "src/main.rs", line: 42 });
    expect(parsePathRef("./src/app.tsx:3:9")).toEqual({ path: "src/app.tsx", line: 3 });
    expect(parsePathRef("src/lib.rs")).toEqual({ path: "src/lib.rs", line: undefined });
    expect(parsePathRef("main.rs:10")).toEqual({ path: "main.rs", line: 10 });
  });

  it("rejects non-paths", () => {
    expect(parsePathRef("foo")).toBeNull();
    expect(parsePathRef("main.rs")).toBeNull();
    expect(parsePathRef("npm run build")).toBeNull();
    expect(parsePathRef("https://x.com/a.js")).toBeNull();
  });
});

describe("format helpers", () => {
  it("formats tokens", () => {
    expect(formatTokens(950)).toBe("950");
    expect(formatTokens(1500)).toBe("1.5k");
    expect(formatTokens(250_000)).toBe("250k");
    expect(formatTokens(1_200_000)).toBe("1.20M");
  });

  it("groups days", () => {
    const now = new Date(2026, 9, 2, 15).getTime();
    expect(dayGroup(new Date(2026, 9, 2, 1).getTime(), now)).toBe("Today");
    expect(dayGroup(new Date(2026, 9, 1, 23).getTime(), now)).toBe("Yesterday");
    expect(dayGroup(new Date(2026, 8, 28).getTime(), now)).toBe("Previous 7 days");
  });
});
