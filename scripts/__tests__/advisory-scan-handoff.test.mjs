import assert from "node:assert/strict";
import { chmodSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { delimiter, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import test from "node:test";

const ROOT = resolve(import.meta.dirname, "../..");
const WRAPPER = resolve(ROOT, "scripts/run-advisory-scans.mjs");
const SCANNER_REQUIREMENTS = resolve(ROOT, "config/pip-audit-requirements.txt");
const GOOD_EXPORT = "maturin==1.9.4\ntomli==2.4.1 ; python_full_version < '3.11'\n";

function executable(path, content) {
  writeFileSync(path, `#!/bin/sh\nset -eu\n${content}`);
  chmodSync(path, 0o755);
}

function runPython(exported, auditStatus = 0) {
  const directory = mkdtempSync(join(tmpdir(), "oidc-advisory-handoff-"));
  const bin = join(directory, "bin");
  const output = join(directory, "output");
  const auditLog = join(directory, "pip-audit-input.txt");
  spawnSync("mkdir", ["-p", bin]);
  executable(join(bin, "uv"), `
output=""
previous=""
for argument in "$@"; do
  if [ "$previous" = "--output-file" ]; then output="$argument"; fi
  previous="$argument"
done
[ -n "$output" ]
printf '%b' ${JSON.stringify(exported)} > "$output"
`);
  executable(join(bin, "pip-audit"), `
if [ "$1" = "--version" ]; then printf '%s\\n' 'pip-audit 2.9.0'; exit 0; fi
requirement=""
previous=""
for argument in "$@"; do
  if [ "$previous" = "--requirement" ]; then requirement="$argument"; fi
  previous="$argument"
done
[ -n "$requirement" ]
printf '%s\\n' "$requirement" > ${JSON.stringify(auditLog)}
cat "$requirement" >> ${JSON.stringify(auditLog)}
printf '%s\\n' '{"dependencies":[]}'
exit ${auditStatus}
`);
  const result = spawnSync(process.execPath, [WRAPPER, "python"], {
    cwd: ROOT,
    encoding: "utf8",
    env: {
      ...process.env,
      PATH: `${bin}${delimiter}${process.env.PATH}`,
      ADVISORY_OUTPUT_DIR: output,
      ADVISORY_POLICY_DATE: "2026-08-23",
    },
  });
  return { directory, output, auditLog, result };
}

function cleanup(run) {
  return rm(run.directory, { recursive: true, force: true });
}

test("python advisory stage audits the exact nonempty frozen build export", async () => {
  const run = runPython(GOOD_EXPORT);
  try {
    assert.equal(run.result.status, 0, `${run.result.stdout}\n${run.result.stderr}`);
    const generated = resolve(run.output, "python-build-requirements.txt");
    assert.equal(readFileSync(generated, "utf8"), GOOD_EXPORT);
    const handoff = readFileSync(run.auditLog, "utf8");
    assert.equal(handoff, `${generated}\n${GOOD_EXPORT}`);
    assert.notEqual(generated, SCANNER_REQUIREMENTS);
  } finally {
    await cleanup(run);
  }
});

for (const [name, exported, message] of [
  ["missing maturin", "tomli==2.4.1 ; python_full_version < '3.11'\n", /missing maturin/],
  ["mismatched maturin", "maturin==1.9.3\ntomli==2.4.1 ; python_full_version < '3.11'\n", /maturin 1\.9\.3, expected 1\.9\.4/],
  ["empty export", "", /export is empty/],
]) {
  test(`python advisory stage fails closed for ${name}`, async () => {
    const run = runPython(exported);
    try {
      assert.equal(run.result.status, 2);
      assert.match(run.result.stderr, message);
      assert.throws(() => readFileSync(run.auditLog, "utf8"));
    } finally {
      await cleanup(run);
    }
  });
}

test("python advisory stage fails closed when pip-audit fails", async () => {
  const run = runPython(GOOD_EXPORT, 2);
  try {
    assert.equal(run.result.status, 2);
    assert.match(run.result.stderr, /pip-audit tool or vulnerability DB failure/);
    assert.match(readFileSync(run.auditLog, "utf8"), /maturin==1\.9\.4/);
  } finally {
    await cleanup(run);
  }
});
