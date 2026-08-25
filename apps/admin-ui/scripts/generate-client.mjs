// Generates the admin console's internal-API client from the service's
// published contract (schemas/internal-api.schema.json).
//
// Why generated rather than handwritten: percent-encoded path segments and
// the service's own wire-format enum spellings become properties of this
// generator instead of obligations on every call site, and a service-side
// rename that is not reflected in the schema breaks generation here instead
// of silently changing an operator control.
//
// Determinism: output is a pure function of the schema — no timestamps, no
// environment reads, sorted operation order — so `--check` can demand byte
// equality between the committed artifacts and a fresh generation.
//
// Usage:
//   node scripts/generate-client.mjs          regenerate api.ts + types.ts
//   node scripts/generate-client.mjs --check  exit 1 if either file differs

import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const SCHEMA_PATH = join(SCRIPT_DIR, "../../../schemas/internal-api.schema.json");
const TYPES_PATH = join(SCRIPT_DIR, "../src/lib/types.ts");
const API_PATH = join(SCRIPT_DIR, "../src/lib/api.ts");

/** Upper bound on pages one completing `listUsers` call follows. */
const PAGER_MAX_PAGES = 1000;

export function loadSchema(schemaPath = SCHEMA_PATH) {
  return JSON.parse(readFileSync(schemaPath, "utf8"));
}

// ---------------------------------------------------------------------------
// $defs → src/lib/types.ts
// ---------------------------------------------------------------------------

/**
 * Map one JSON-schema node onto its TypeScript rendering. `$ref`s translate
 * by name; every reference was already validated against the document's
 * `$defs` before generation started, so an unknown name cannot reach here.
 */
function tsType(node) {
  if (node.$ref !== undefined) {
    return node.$ref.replace("#/$defs/", "");
  }
  if (Array.isArray(node.type)) {
    return node.type.map((part) => tsType({ ...node, type: part })).join(" | ");
  }
  switch (node.type) {
    case "string":
      return "string";
    case "integer":
    case "number":
      return "number";
    case "boolean":
      return "boolean";
    case "null":
      return "null";
    case "array":
      return `Array<${tsType(node.items ?? {})}>`;
    case "object": {
      // Property-bearing objects render inline as an object literal type;
      // only free-form maps land as Record<string, unknown>.
      if (node.properties === undefined) {
        return "Record<string, unknown>";
      }
      const required = new Set(node.required ?? []);
      const parts = Object.entries(node.properties).map(([key, child]) => {
        const optional = required.has(key) ? "" : "?";
        return `${key}${optional}: ${tsType(child)}`;
      });
      return `{ ${parts.join("; ")} }`;
    }
    default:
      throw new Error(`unsupported schema type: ${JSON.stringify(node.type ?? node)}`);
  }
}

/** Render every `$def` as its exported TypeScript declaration. */
export function generateTypes(schema) {
  const defs = schema["$defs"] ?? {};
  const blocks = [];
  for (const [name, def] of Object.entries(defs)) {
    if (def.type === "string" && Array.isArray(def.enum)) {
      // Literal unions of the SERVICE's wire values: a title-cased status
      // cannot even typecheck, let alone ship.
      const literals = def.enum.map((value) => JSON.stringify(value)).join(" | ");
      blocks.push(`export type ${name} = ${literals};`);
      continue;
    }
    if (def.type === "object" && def.properties === undefined) {
      blocks.push(`export type ${name} = Record<string, unknown>;`);
      continue;
    }

    const required = new Set(def.required ?? []);
    const lines = Object.entries(def.properties ?? {}).map(([key, child]) => {
      const optional = required.has(key) ? "" : "?";
      const doc = child.description === undefined ? "" : `  /** ${child.description} */\n`;
      return `${doc}  ${key}${optional}: ${tsType(child)};`;
    });
    blocks.push(`export interface ${name} {\n${lines.join("\n")}\n}`);
  }

  return [
    "// GENERATED FILE — do not edit.",
    "// Source: schemas/internal-api.schema.json (run `pnpm generate`).",
    "",
    ...blocks,
    "",
  ].join("\n");
}

/**
 * Reject any `$ref` that does not name a `$def` of the document — including
 * the refs inside `paths`, so a service contract rename breaks generation
 * instead of silently re-addressing an operator control.
 */
function validateRefs(node, known, seen = new Set()) {
  if (node === null || typeof node !== "object") return;
  if (typeof node.$ref === "string") {
    const name = node.$ref.replace("#/$defs/", "");
    if (!known.has(name)) {
      throw new Error(`schema references unknown definition: ${name}`);
    }
  }
  if (seen.has(node)) return;
  seen.add(node);
  for (const child of Object.values(node)) {
    validateRefs(child, known, seen);
  }
}

// ---------------------------------------------------------------------------
// paths → src/lib/api.ts
// ---------------------------------------------------------------------------

/**
 * Split an API path into literal segments and `{param}` placeholders so every
 * placeholder is wrapped in exactly one `encodeURIComponent` at generation
 * time.
 */
function pathParts(path) {
  return path.split("/").map((segment) => {
    if (segment.startsWith("{") && segment.endsWith("}")) {
      return { kind: "param", name: segment.slice(1, -1) };
    }
    return { kind: "literal", name: segment };
  });
}

function pathParamNames(path) {
  return pathParts(path)
    .filter((part) => part.kind === "param")
    .map((part) => part.name);
}

/** The target-path statements for one operation, query composition included. */
function targetStatements(path, queryParams) {
  const parts = pathParts(path);
  const hasParams = parts.some((part) => part.kind === "param");
  const base = parts
    .map((part) =>
      part.kind === "literal"
        ? part.name === ""
          ? ""
          : `/${part.name}`
        : `/\${encodeURIComponent(${part.name})}`,
    )
    .join("");

  // A path carrying parameters must be a template literal, or the encoding
  // expression would be dead text.
  const baseLiteral = hasParams ? `\`${base}\`` : `"${base}"`;

  if (queryParams.length === 0) {
    return [`const target = ${baseLiteral};`];
  }
  const lines = ["const search = new URLSearchParams();"];
  for (const param of queryParams) {
    if (param.name === "cursor") {
      lines.push('if (cursor !== null && cursor !== undefined) search.set("cursor", cursor);');
    } else if (param.name === "limit") {
      lines.push('if (limit !== undefined) search.set("limit", String(limit));');
    } else {
      throw new Error(`unsupported query parameter: ${param.name}`);
    }
  }
  const queryTarget = hasParams
    ? `\`${base}?\${search.toString()}\``
    : `"${base}?" + search.toString()`;
  lines.push(`const target = search.size === 0 ? ${baseLiteral} : ${queryTarget};`);
  return lines;
}

/**
 * One exported async function per operation. Response handling follows the
 * schema: the `2xx` `$ref` names the resolved result type, and
 * `x-not-found-is-null` on a declared 404 maps that outcome to `null`.
 */
export function generateOperation(path, method, op) {
  const verb = method.toUpperCase();
  const responses = op.responses ?? {};
  const okCode = Object.keys(responses).find((code) => code.startsWith("2")) ?? "200";
  const ok = responses[okCode] ?? {};
  const notFoundIsNull = responses["404"]?.["x-not-found-is-null"] === true;

  const resultType =
    ok.$ref !== undefined
      ? ok.$ref.replace("#/$defs/", "")
      : ok.type === "null"
        ? "void"
        : "unknown";
  const returnType = notFoundIsNull ? `${resultType} | null` : resultType;

  const args = pathParamNames(path).map((name) => `${name}: string`);
  const queryParams = (op.parameters ?? []).filter((p) => p.in === "query");
  const takesPageArgs = queryParams.length > 0;
  const hasBody = op.body !== undefined;
  if (takesPageArgs) {
    args.push("{ cursor, limit }: { cursor?: string | null; limit?: number }");
  }
  if (hasBody) {
    args.push(`body: ${op.body.$ref.replace("#/$defs/", "")}`);
  }

  const header =
    takesPageArgs && args.length === 1
      ? [
          `export async function ${op.operationId}({`,
          "  cursor,",
          "  limit,",
          "}: {",
          "  cursor?: string | null;",
          "  limit?: number;",
          // Defaulted so the bare call — start the listing from scratch —
          // destructures an object instead of `undefined`.
          `} = {}): Promise<${returnType}> {`,
        ].join("\n")
      : `export async function ${op.operationId}(${args.join(", ")}): Promise<${returnType}> {`;

  const bodyLines = [];
  if (hasBody) {
    bodyLines.push(
      "const init: RequestInit = {",
      '  method: "' + verb + '",',
      '  headers: { "Content-Type": "application/json" },',
      "  body: JSON.stringify(body),",
      "};",
    );
  } else if (verb !== "GET") {
    bodyLines.push(`const init: RequestInit = { method: "${verb}" };`);
  }

  const call = bodyLines.length > 0 ? "request(target, init)" : "request(target)";
  const successLine =
    returnType === "void" ? `await ${call};` : `return (await ${call}) as ${resultType};`;

  const tail = notFoundIsNull
    ? [
        "try {",
        `  ${successLine}`,
        "} catch (error) {",
        "  if (error instanceof InternalApiError && error.status === 404) {",
        "    return null;",
        "  }",
        "  throw error;",
        "}",
      ]
    : [successLine];

  // Every emitted body statement sits at one indent level; the try/catch
  // carries its own internal indentation.
  const body = [...targetStatements(path, queryParams), ...bodyLines, ...tail].map(
    (line) => `  ${line}`,
  );

  return [header, ...body, "}"].join("\n");
}

/** The name of the `$def` carrying an x-cursor-flagged property. */
export function findPageDefName(schema) {
  for (const [name, def] of Object.entries(schema["$defs"] ?? {})) {
    const flagged = Object.values(def.properties ?? {}).some((child) => child["x-cursor"] === true);
    if (flagged) return name;
  }
  throw new Error("schema declares no x-cursor-flagged page definition");
}

/** Emit the completing pager over the listing operation. */
export function buildPager(schema) {
  const pageDefName = findPageDefName(schema);
  const rowsField = Object.entries(schema["$defs"][pageDefName].properties).find(
    ([, child]) => child.type === "array",
  )?.[0];
  if (rowsField === undefined) {
    throw new Error(`${pageDefName} carries no array field to paginate`);
  }
  return [
    "/** Upper bound on pages one completing call follows. */",
    `const PAGER_MAX_PAGES = ${PAGER_MAX_PAGES};`,
    "",
    "/**",
    " * Follow next_cursor until the listing is exhausted. A short page does NOT",
    " * end the traversal — only a null next_cursor does — because adapters may",
    " * legitimately return fewer rows than the limit with more pages remaining.",
    " */",
    `export async function listUsers(options: { limit?: number } = {}): Promise<${pageDefName}> {`,
    "  let cursor: string | null = null;",
    "  const rows: Array<User> = [];",
    "  for (let page = 0; page < PAGER_MAX_PAGES; page += 1) {",
    "    const result = await listUsersPage({ cursor, limit: options.limit });",
    `    rows.push(...result.${rowsField});`,
    "    cursor = result.next_cursor;",
    "    if (cursor === null) {",
    `      return { ${rowsField}: rows, next_cursor: null };`,
    "    }",
    "  }",
    "  throw new InternalApiError(",
    "    500,",
    '    "pager_exhausted",',
    "    `listing did not terminate within ${PAGER_MAX_PAGES} pages`,",
    "  );",
    "}",
  ].join("\n");
}

/**
 * The `$def` names the generated api.ts actually references: operation
 * result and body refs, plus the pager's page type and its row element.
 * Sorted, so the emitted import is a deterministic function of the schema.
 */
function collectUsedDefNames(schema, operations) {
  const used = new Set();
  const addRef = (node) => {
    if (node !== null && typeof node === "object" && typeof node.$ref === "string") {
      used.add(node.$ref.replace("#/$defs/", ""));
    }
  };

  for (const { op } of operations) {
    const responses = op.responses ?? {};
    const okCode = Object.keys(responses).find((code) => code.startsWith("2")) ?? "200";
    addRef(responses[okCode] ?? {});
    addRef(op.body);
  }

  const pageDefName = findPageDefName(schema);
  used.add(pageDefName);
  const pageDef = schema["$defs"][pageDefName] ?? {};
  for (const child of Object.values(pageDef.properties ?? {})) {
    if (child.type === "array") {
      addRef(child.items);
    }
  }

  return [...used].sort();
}

export function generateApi(schema) {
  const preference = schema["x-credentials"].preference;

  const operations = [];
  for (const [path, methods] of Object.entries(schema.paths)) {
    for (const [method, op] of Object.entries(methods)) {
      operations.push({ path, method, op });
    }
  }
  operations.sort((a, b) => a.op.operationId.localeCompare(b.op.operationId));
  const usedDefNames = collectUsedDefNames(schema, operations);

  const credentialBranches = preference.map((entry) => {
    if (entry.transport === "authorization-header") {
      const envName = entry.env[0];
      return [
        "if (present(env." + envName + ")) {",
        '  return { kind: "' + entry.kind + '", authorization: `Bearer ${env.' + envName + "}` };",
        "}",
      ];
    }
    const [certEnv, keyEnv] = entry.env;
    return [
      "if (present(env." + certEnv + ") && present(env." + keyEnv + ")) {",
      "  return {",
      '    kind: "' + entry.kind + '",',
      "    certificate: env." + certEnv + ",",
      "    privateKey: env." + keyEnv + ",",
      "  };",
      "}",
    ];
  });

  const credentialEnvList = preference.flatMap((entry) => entry.env).join(", ");

  // A certificate/key pair must be configured completely or not at all;
  // anything else is a broken deployment, not a reason to fall back.
  const certEntry = preference.find((entry) => entry.transport === "mtls");
  const pairGuard =
    certEntry === undefined
      ? []
      : [
          `const [certSet, keySet] = [present(env.${certEntry.env[0]}), present(env.${certEntry.env[1]})];`,
          "if (certSet !== keySet) {",
          "  throw new InternalApiConfigurationError(",
          `    "${certEntry.env[0]} and ${certEntry.env[1]} must be configured together",`,
          "  );",
          "}",
        ];

  const head = [
    "// GENERATED FILE — do not edit.",
    "// Source: schemas/internal-api.schema.json (run `pnpm generate`).",
    "//",
    "// Server-side only: this module imports SvelteKit's `$env/dynamic/private`,",
    "// so reaching it from browser code fails the build. Operator credentials",
    "// live only in the server runtime and never reach the browser bundle.",
    'import { env } from "$env/dynamic/private";',
    "",
    'import { requestViaTls } from "./tls-transport";',
    // The contract's own names, resolved from types.ts: a rename that is not
    // reflected there fails typecheck here rather than degrading to `any`.
    ...(usedDefNames.length > 0
      ? [`import type { ${usedDefNames.join(", ")} } from "./types";`]
      : []),
    "",
    "/** The documented default is the admin listener, not the public one. */",
    "const INTERNAL_API_URL = env." +
      schema["x-base-url-env"] +
      ' || "' +
      schema["x-default-base-url"] +
      '";',
    "",
    "export class InternalApiError extends Error {",
    "  readonly status: number;",
    "  readonly code: string;",
    "",
    "  constructor(status: number, code: string, description: string | null) {",
    "    super(description === null ? `${code} (${status})` : `${code} (${status}): ${description}`);",
    '    this.name = "InternalApiError";',
    "    this.status = status;",
    "    this.code = code;",
    "  }",
    "}",
    "",
    "export class InternalApiConfigurationError extends Error {",
    "  constructor(message: string) {",
    "    super(message);",
    '    this.name = "InternalApiConfigurationError";',
    "  }",
    "}",
    "",
    "interface AuthorizationCredential {",
    '  kind: "operator_token" | "shared_secret";',
    "  authorization: string;",
    "}",
    "",
    "interface ClientCertificateCredential {",
    '  kind: "client_certificate";',
    "  certificate: string;",
    "  privateKey: string;",
    "}",
    "",
    "type OperatorCredential = AuthorizationCredential | ClientCertificateCredential;",
    "",
    'export type OperatorCredentialKind = OperatorCredential["kind"];',
    "",
    "function present(value: string | undefined): value is string {",
    '  return value !== undefined && value !== "";',
    "}",
    "",
    "/**",
    " * Resolve the operator credential in the contract's documented preference",
    " * order. Values come from the server-side environment only and are never",
    " * logged; a half-configured client certificate is a configuration error,",
    " * never a silent downgrade to a weaker credential.",
    " */",
    "export function resolveOperatorCredential(): OperatorCredential {",
    // A certificate/key pair must be configured completely or not at all;
    // anything else is a broken deployment, not a reason to fall back to a
    // weaker credential, so the guard runs before any preference branch.
    ...pairGuard.map((line) => "  " + line),
    ...credentialBranches.flat().map((line) => "  " + line),
    "  throw new InternalApiConfigurationError(",
    `    "no operator credential configured: set one of ${credentialEnvList}",`,
    "  );",
    "}",
  ];

  const transport = [
    "async function request(path: string, init: RequestInit = {}): Promise<unknown> {",
    "  const url = `${INTERNAL_API_URL}${path}`;",
    "  const credential = resolveOperatorCredential();",
    "  let response: Response;",
    '  if (credential.kind === "client_certificate") {',
    "    // Mutual TLS is presented by the TLS layer itself, not a header.",
    "    response = await requestViaTls(url, { ...init, credential });",
    "  } else {",
    "    const headers = new Headers(init.headers);",
    '    headers.set("Authorization", credential.authorization);',
    "    response = await fetch(url, { ...init, headers });",
    "  }",
    "  await assertOk(response);",
    "  const text = await response.text();",
    "  // Null-typed successes (the bare mutation verbs) ship an empty body.",
    '  return text === "" ? null : JSON.parse(text);',
    "}",
    "",
    "async function assertOk(response: Response): Promise<void> {",
    "  if (response.ok) {",
    "    return;",
    "  }",
    '  let code = "unknown_error";',
    "  let description: string | null = null;",
    "  try {",
    "    const body = (await response.json()) as { error?: unknown; error_description?: unknown };",
    '    if (typeof body.error === "string") code = body.error;',
    '    if (typeof body.error_description === "string") description = body.error_description;',
    "  } catch {",
    "    // A non-JSON error body still surfaces its status via the envelope.",
    "  }",
    "  throw new InternalApiError(response.status, code, description);",
    "}",
  ];

  const operationBlocks = operations
    .map(({ path, method, op }) => generateOperation(path, method, op))
    .join("\n\n");

  return [head.join("\n"), transport.join("\n"), operationBlocks, buildPager(schema)]
    .join("\n\n")
    .concat("\n");
}

function main(argv) {
  const checkOnly = argv.includes("--check");
  const schema = loadSchema();

  // Every `$ref` in the published document must name a `$def` — including
  // the refs under `paths`. A service contract rename that is not reflected
  // here breaks generation instead of silently changing an operator control.
  validateRefs(schema, new Set(Object.keys(schema["$defs"] ?? {})));

  const types = generateTypes(schema);
  const api = generateApi(schema);

  if (checkOnly) {
    const currentTypes = readFileSync(TYPES_PATH, "utf8");
    const currentApi = readFileSync(API_PATH, "utf8");
    if (currentTypes !== types || currentApi !== api) {
      console.error("generated client is stale: run `pnpm generate` and commit the result");
      process.exitCode = 1;
      return;
    }
    console.log("generated client is fresh");
    return;
  }

  writeFileSync(TYPES_PATH, types);
  writeFileSync(API_PATH, api);
  console.log(`wrote ${TYPES_PATH}`);
  console.log(`wrote ${API_PATH}`);
}

const invokedDirectly = import.meta.url === pathToFileURL(process.argv[1] ?? "").href;
if (invokedDirectly) {
  main(process.argv.slice(2));
}
