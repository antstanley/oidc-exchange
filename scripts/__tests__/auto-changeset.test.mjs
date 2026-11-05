import assert from "node:assert/strict";
import test from "node:test";
import { bumpFor, cleanBody, parseTitle, renderChangeset } from "../auto-changeset.mjs";

test("conventional titles map to bumps", () => {
  const bump = (title, body) => bumpFor(parseTitle(title), body);
  assert.equal(bump("feat(release): add thing"), "minor");
  assert.equal(bump("fix: repair"), "patch");
  assert.equal(bump("perf(core): faster"), "patch");
  assert.equal(bump("feat!: drop api"), "major");
  assert.equal(bump("fix: x", "text\n\nBREAKING CHANGE: gone"), "major");
  for (const title of ["chore: deps", "docs: x", "ci: y", "refactor: z", "not conventional"])
    assert.equal(bump(title), null, title);
});

test("renders a changeset with summary and trimmed body", () => {
  const out = renderChangeset({
    title: "feat(release): self-hosted signed binaries (PR 2 of 3)",
    body: "## What changes\n- one\n\n🤖 Generated with [Claude Code](https://claude.com/claude-code)\n",
  });
  assert.equal(
    out,
    '---\n"@oidc-exchange/node": minor\n---\n\nSelf-hosted signed binaries (PR 2 of 3)\n\n**What changes**\n- one\n',
  );
});

test("non-release types produce no changeset", () => {
  assert.equal(renderChangeset({ title: "ci: tweak", body: "" }), null);
});

test("headings inside code fences are kept and truncation closes open fences", () => {
  assert.equal(cleanBody("# Top\n```\n# comment\n```"), "**Top**\n```\n# comment\n```");
  const cleaned = cleanBody(`\`\`\`\n${"line\n".repeat(600)}`);
  assert.ok(cleaned.includes("\n```\n\n…"));
});

test("long bodies are truncated and comments removed", () => {
  const cleaned = cleanBody(`<!-- hidden -->\n${"para\n\n".repeat(600)}`);
  assert.ok(cleaned.length <= 1510);
  assert.ok(cleaned.endsWith("…"));
  assert.ok(!cleaned.includes("hidden"));
});
