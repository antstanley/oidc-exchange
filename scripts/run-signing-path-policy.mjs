#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";

const ROOT = resolve(import.meta.dirname, "..");
const policyPath = resolve(ROOT, "config/signing-path-policy.json");
const policy = JSON.parse(readFileSync(policyPath, "utf8"));
const today = process.env.SIGNING_POLICY_DATE ?? new Date().toISOString().slice(0, 10);
const directory = mkdtempSync(resolve(tmpdir(), "oidc-signing-policy-"));
try {
  for (const mode of policy.modes) {
    const metadata = spawnSync("cargo", ["metadata", "--manifest-path", resolve(ROOT, "Cargo.toml"), ...mode.metadata_args], { cwd: ROOT, encoding: "utf8", env: { ...process.env, CARGO_NET_OFFLINE: "true" }, maxBuffer: 8 * 1024 * 1024 });
    if (metadata.status !== 0 || metadata.error) throw new Error(`cargo metadata failed for ${mode.name}: ${metadata.stderr}`);
    const metadataPath = resolve(directory, `${mode.name}.json`);
    writeFileSync(metadataPath, metadata.stdout);
    const result = spawnSync(process.execPath, [resolve(ROOT, "scripts/signing-path-policy.mjs"), "evaluate", policyPath, metadataPath, mode.name, today], { cwd: ROOT, encoding: "utf8" });
    process.stdout.write(result.stdout ?? "");
    process.stderr.write(result.stderr ?? "");
    if (result.status !== 0) process.exitCode = result.status;
  }
} catch (error) {
  console.error(`signing path policy failed closed: ${error.message}`);
  process.exitCode = 2;
} finally {
  rmSync(directory, { recursive: true, force: true });
}
