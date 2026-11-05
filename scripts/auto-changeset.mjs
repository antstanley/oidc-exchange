#!/usr/bin/env node
// Generates `.changeset/auto-pr-<n>.md` for a pull request from its title and body,
// committed to the PR branch through the GitHub contents API. Runs from the base
// branch under pull_request_target and never executes or checks out PR code; the
// title/body are read from the event payload, not interpolated into a shell.
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";

const PACKAGE = "@oidc-exchange/node"; // fixed group: bumps every published package
const TITLE = /^(\w+)(?:\([^)]*\))?(!)?:\s*(.+)$/;
const MINOR = new Set(["feat"]);
const PATCH = new Set(["fix", "perf", "revert"]);
const BODY_LIMIT = 1500;

export function parseTitle(title) {
  const match = TITLE.exec(title.trim());
  if (!match) return { type: null, breaking: false, description: title.trim() };
  return { type: match[1].toLowerCase(), breaking: match[2] === "!", description: match[3].trim() };
}

export function bumpFor({ type, breaking }, body = "") {
  if (breaking || /^BREAKING[ -]CHANGE:/m.test(body)) return "major";
  if (MINOR.has(type)) return "minor";
  if (PATCH.has(type)) return "patch";
  return null; // chore, docs, ci, test, refactor, build, style, unparseable
}

export function cleanBody(body = "") {
  let text = body.replace(/\r\n/g, "\n").replace(/<!--[\s\S]*?-->/g, "");
  const cut = text.search(/^.*Generated with \[Claude Code\]/m);
  if (cut >= 0) text = text.slice(0, cut);
  let fenced = false;
  text = text
    .split("\n")
    .map((line) => {
      if (/^\s*(```|~~~)/.test(line)) fenced = !fenced;
      return fenced ? line : line.replace(/^#{1,6}\s+(.*)$/, "**$1**");
    })
    .join("\n")
    .trim();
  if (text.length > BODY_LIMIT) {
    const head = text.slice(0, BODY_LIMIT);
    let kept = head.slice(0, Math.max(head.lastIndexOf("\n\n"), 200)).trim();
    // Close a code fence the cut left open so the changelog stays well formed.
    if ((kept.match(/^\s*(```|~~~)/gm) ?? []).length % 2 === 1) kept += "\n```";
    text = `${kept}\n\n…`;
  }
  return text;
}

// Returns null for types that do not affect a release (chore, docs, ci, ...): an empty
// changeset would only make the Version Packages workflow open a PR that bumps nothing.
export function renderChangeset({ title, body }) {
  const parsed = parseTitle(title);
  const bump = bumpFor(parsed, body);
  if (!bump) return null;
  const summary = parsed.description.charAt(0).toUpperCase() + parsed.description.slice(1);
  const detail = cleanBody(body);
  return `---\n"${PACKAGE}": ${bump}\n---\n\n${summary}${detail ? `\n\n${detail}` : ""}\n`;
}

const gh = (args, input) =>
  execFileSync("gh", ["api", ...args], { encoding: "utf8", input, stdio: ["pipe", "pipe", "pipe"] });

function existing(repo, path, ref) {
  try {
    return JSON.parse(gh([`repos/${repo}/contents/${path}?ref=${encodeURIComponent(ref)}`]));
  } catch (error) {
    if (String(error.stderr).includes("404")) return null;
    throw error;
  }
}

function main() {
  const event = JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH, "utf8"));
  const pr = event.pull_request;
  const base = event.repository.full_name;
  if (pr.head.repo?.full_name !== base) return console.log("fork PR: skipping");
  if (pr.head.ref.startsWith("changeset-release/")) return console.log("version PR: skipping");

  const path = `.changeset/auto-pr-${pr.number}.md`;
  const content = renderChangeset({ title: pr.title, body: pr.body ?? "" });
  const optOut = pr.labels.some((label) => label.name === "no-changeset");
  const skip = optOut || content === null;
  const files = JSON.parse(gh(["--paginate", "--slurp", `repos/${base}/pulls/${pr.number}/files`])).flat();
  const human = files.some(
    (file) =>
      /^\.changeset\/[^/]+\.md$/.test(file.filename) &&
      file.filename !== ".changeset/README.md" &&
      file.filename !== path &&
      file.status !== "removed",
  );
  const current = existing(base, path, pr.head.ref);

  if (skip || human) {
    if (current) {
      gh(
        ["--method", "DELETE", `repos/${base}/contents/${path}`, "--input", "-"],
        JSON.stringify({ message: `chore(changeset): remove generated changeset for #${pr.number}`, sha: current.sha, branch: pr.head.ref }),
      );
      console.log(`removed ${path} (${optOut ? "no-changeset label" : content === null ? "title needs no release" : "author-written changeset present"})`);
    } else console.log("nothing to do");
    return;
  }

  if (current && Buffer.from(current.content, "base64").toString("utf8") === content)
    return console.log("changeset up to date");
  gh(
    ["--method", "PUT", `repos/${base}/contents/${path}`, "--input", "-"],
    JSON.stringify({
      message: `chore(changeset): ${current ? "update" : "add"} generated changeset for #${pr.number}`,
      content: Buffer.from(content).toString("base64"),
      branch: pr.head.ref,
      ...(current ? { sha: current.sha } : {}),
    }),
  );
  console.log(`${current ? "updated" : "added"} ${path}`);
}

if (process.argv[1] === import.meta.filename) main();
