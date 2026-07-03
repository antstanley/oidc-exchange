import { createRequire } from "node:module";
import { platform, arch } from "node:process";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
const __dirname = dirname(fileURLToPath(import.meta.url));

const PLATFORM_MAP = {
  "linux-x64": "@oidc-exchange/linux-x64-gnu",
  "linux-arm64": "@oidc-exchange/linux-arm64-gnu",
  "win32-x64": "@oidc-exchange/win32-x64-msvc",
  "darwin-arm64": "@oidc-exchange/darwin-arm64",
};

let nativeBinding = null;
let loadError = null;

const platformKey = `${platform}-${arch}`;
const packageName = PLATFORM_MAP[platformKey];

if (packageName) {
  try {
    nativeBinding = require(packageName);
  } catch (primaryError) {
    // The platform package resolved but failed to load — most often an
    // ABI/glibc mismatch (e.g. "version `GLIBC_2.39' not found" when the
    // published addon was linked against a newer glibc than this host has).
    // Fall back to a co-located dev build, but keep `primaryError`: it is the
    // informative one, whereas the fallback's "Cannot find module" masks it.
    try {
      nativeBinding = require(join(__dirname, "oidc-exchange.node"));
    } catch (fallbackError) {
      loadError = new Error(
        `Failed to load the native binding for ${platformKey}: the platform ` +
          `package "${packageName}" did not load and no local dev build ` +
          `(oidc-exchange.node) was found. See \`error.cause\` for the ` +
          `underlying failure — a "GLIBC_x.yz not found" there means the ` +
          `published addon needs a newer glibc than this host provides.`,
        { cause: primaryError },
      );
      loadError.fallbackError = fallbackError;
    }
  }
} else {
  loadError = new Error(`Unsupported platform: ${platformKey}`);
}

if (!nativeBinding) {
  if (loadError) throw loadError;
  throw new Error(`Failed to load native binding for platform: ${platformKey}`);
}

export const { OidcExchange } = nativeBinding;
