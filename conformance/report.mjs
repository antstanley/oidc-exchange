import { readFile } from "node:fs/promises";

const corpusUrl = new URL("./corpus/requests.json", import.meta.url);
const corpus = JSON.parse(await readFile(corpusUrl, "utf8"));
const required = [
  "encoded-slash",
  "encoded-dot-dot",
  "encoded-question",
  "encoded-hash",
  "forwarded-for-first",
  "forwarded-for-reversed",
  "base-path-sibling",
  "base-path-child",
  "empty-path",
  "malformed-content-length",
  "huge-content-length",
  "body-one-over-cap",
];
const shapes = ["native", "ffi", "node", "lambda", "asgi", "wsgi"];
const ids = new Set(corpus.fixtures.map(({ id }) => id));
const missing = required.filter((id) => !ids.has(id));
if (missing.length > 0) throw new Error(`missing required fixtures: ${missing.join(", ")}`);

const baseline = {
  native: [],
  ffi: ["empty-path", "malformed-content-length", "huge-content-length", "body-one-over-cap"],
  node: ["empty-path", "malformed-content-length", "huge-content-length", "body-one-over-cap"],
  lambda: ["base-path-sibling", "empty-path", "malformed-content-length", "huge-content-length"],
  asgi: ["encoded-slash", "encoded-dot-dot", "encoded-question", "encoded-hash", "malformed-content-length", "huge-content-length"],
  wsgi: ["encoded-slash", "encoded-dot-dot", "encoded-question", "encoded-hash", "forwarded-for-first", "forwarded-for-reversed", "empty-path"],
};

console.log(`conformance reporting baseline: ${corpus.fixtures.length} fixtures, ${shapes.length} shapes`);
for (const shape of shapes) {
  const differences = baseline[shape];
  console.log(`${shape}: ${differences.length} known difference(s)`);
  for (const id of differences) {
    const fixture = corpus.fixtures.find((entry) => entry.id === id);
    const qualification = fixture.qualifications?.[shape];
    console.log(`  ${id}: fields=${corpus.fieldsCompared.join(",")}${qualification ? `; qualification=${qualification}` : ""}`);
  }
}
console.log("reporting mode: differences are recorded and intentionally non-gating until task 10");
