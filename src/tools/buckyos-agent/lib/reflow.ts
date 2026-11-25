// The reflow library: in a user's bucket `user/control-panel/<user>/reflow`,
// per site, the fallback pages made for single pages of it
// (`<site>/fallback/<page>.html` and its `.json`). Rules:
// doc/opendan/protocol/Agent Browser Protocol.md §5.1. The Agent Service
// follows the same ones; both sides are checked against
// doc/opendan/protocol/fixtures/reflow_vectors.json. The same bucket holds the
// user's reading site (`_data/`, `_app/`, …): lib/site/.

export const REFLOW_APP = "control-panel";
export const REFLOW_BUCKET = "reflow";
export const FALLBACK_SCHEMA = "buckyos.reflow.fallback/1";
const FALLBACK_DIR = "fallback";

export class ReflowError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ReflowError";
  }
}

// ── §5.1: site, normalized URL, page ───────────────────────────────────────

export interface ReflowAddress {
  /** The normalized URL. */
  url: string;
  /** Its host name: lower case, no port, no trailing dot. */
  host: string;
  /** Its path, as the normalized URL spells it (percent-encoded). */
  path: string;
  site: string;
  page: string;
}

/** `www.example.com` → `www-example-com`; a name without a dash gets `-host`. */
export function siteOfHost(host: string): string {
  const name = host.toLowerCase().replace(/\.$/, "");
  if (!/^[a-z0-9._-]+$/.test(name)) {
    throw new ReflowError(`unsupported host name: ${host}`);
  }
  const site = name.replaceAll(".", "-");
  return site.includes("-") ? site : `${site}-host`;
}

export function isSite(value: string): boolean {
  return /^[a-z0-9_-]+$/.test(value) && value.includes("-");
}

/** WHATWG serialization without fragment and credentials; http(s) only. */
export function normalizeUrl(
  input: string,
): { url: string; host: string; path: string } {
  let url: URL;
  try {
    url = new URL(input.trim());
  } catch {
    throw new ReflowError(`not a URL: ${input}`);
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    throw new ReflowError(`only http(s) pages are reflowed: ${input}`);
  }
  url.hash = "";
  url.username = "";
  url.password = "";
  return {
    url: url.href,
    host: url.hostname.replace(/\.$/, ""),
    path: url.pathname,
  };
}

/** The first 16 hex characters of the SHA-256 of the normalized URL. */
export async function pageOf(normalizedUrl: string): Promise<string> {
  const digest = await crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(normalizedUrl),
  );
  return Array.from(
    new Uint8Array(digest).slice(0, 8),
    (b) => b.toString(16).padStart(2, "0"),
  ).join("");
}

export async function reflowAddress(input: string): Promise<ReflowAddress> {
  const { url, host, path } = normalizeUrl(input);
  return { url, host, path, site: siteOfHost(host), page: await pageOf(url) };
}

// ── match rules ────────────────────────────────────────────────────────────

/** `host` is a lower-case host name; `path` a pattern (none: every page of the host). */
export interface MatchRule {
  host: string;
  path?: string;
}

/** `*` matches any run of characters, `/` included; the rest is literal. */
export function pathMatches(pattern: string, path: string): boolean {
  const parts = pattern.split("*");
  if (parts.length === 1) return pattern === path;
  if (!path.startsWith(parts[0])) return false;
  let at = parts[0].length;
  for (const part of parts.slice(1, -1)) {
    const found = path.indexOf(part, at);
    if (found < 0) return false;
    at = found + part.length;
  }
  const last = parts[parts.length - 1];
  return path.length - at >= last.length && path.endsWith(last);
}

/** Whether one of the rules takes the address: a source's `match` of a reading site is a list of them. */
export function rulesMatch(
  rules: MatchRule[],
  address: { host: string; path: string },
): boolean {
  return rules.some((rule) =>
    rule.host === address.host && pathMatches(rule.path ?? "/*", address.path)
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object" && !Array.isArray(value);
}

/** The type a file is served with on `reflow.<zone>`. */
export function contentTypeOf(name: string): string {
  const ext = name.includes(".") ? name.slice(name.lastIndexOf(".") + 1).toLowerCase() : "";
  switch (ext) {
    case "html":
    case "htm":
      return "text/html; charset=utf-8";
    case "js":
    case "mjs":
      return "text/javascript; charset=utf-8";
    case "css":
      return "text/css; charset=utf-8";
    case "json":
    case "map":
      return "application/json";
    case "txt":
    case "md":
      return "text/plain; charset=utf-8";
    case "svg":
      return "image/svg+xml";
    case "png":
      return "image/png";
    case "jpg":
    case "jpeg":
      return "image/jpeg";
    case "gif":
      return "image/gif";
    case "webp":
      return "image/webp";
    case "avif":
      return "image/avif";
    case "ico":
      return "image/x-icon";
    case "woff":
      return "font/woff";
    case "woff2":
      return "font/woff2";
    case "ttf":
      return "font/ttf";
    case "wasm":
      return "application/wasm";
    default:
      return "application/octet-stream";
  }
}

// ── The library in the bucket ──────────────────────────────────────────────

/** The part of the websdk `BucketClient` the library uses. */
export interface LibraryStore {
  listObjects(
    bucketId: string,
    req: { prefix?: string; delimiter?: string; start_after?: string },
  ): Promise<{
    items: { key: string }[];
    common_prefixes: string[];
    next_start_after?: string;
  }>;
  fetchObject(bucketId: string, key: string): Promise<Response>;
  putBytes(
    bucketId: string,
    key: string,
    body: Uint8Array | string,
    options?: { contentType?: string },
  ): Promise<unknown>;
  delete(bucketId: string, key: string): Promise<unknown>;
}

export function libraryBucketId(ownerUserId: string): string {
  return `user/${REFLOW_APP}/${ownerUserId}/${REFLOW_BUCKET}`;
}

export function errorCode(err: unknown): string | undefined {
  const code = (err as { code?: unknown } | null)?.code;
  return typeof code === "string" ? code : undefined;
}

async function readJson(
  store: LibraryStore,
  bucketId: string,
  key: string,
): Promise<unknown> {
  let response: Response;
  try {
    response = await store.fetchObject(bucketId, key);
  } catch (err) {
    if (errorCode(err) === "no_such_key") return undefined;
    throw err;
  }
  try {
    return JSON.parse(await response.text());
  } catch {
    return undefined;
  }
}

async function listKeys(
  store: LibraryStore,
  bucketId: string,
  prefix: string,
): Promise<string[]> {
  const keys: string[] = [];
  let startAfter: string | undefined;
  do {
    const page = await store.listObjects(bucketId, {
      prefix,
      ...(startAfter ? { start_after: startAfter } : {}),
    });
    keys.push(...page.items.map((item) => item.key));
    startAfter = page.next_start_after;
  } while (startAfter);
  return keys;
}

export interface Made {
  made_at: number;
  made_by: string;
  source_url: string;
}

/** What the library has for one URL: what `browser.reflow_state` answers. */
export interface LibraryState {
  url: string;
  site: string;
  page: string;
  fallback: Made | null;
  /** The page to show, relative to `https://reflow.<zone>/`. */
  view: string | null;
}

function madeOf(value: unknown): Made | null {
  if (
    !isRecord(value) || typeof value.made_at !== "number" ||
    typeof value.made_by !== "string" || typeof value.source_url !== "string"
  ) {
    return null;
  }
  return { made_at: value.made_at, made_by: value.made_by, source_url: value.source_url };
}

/** The key of the fallback page of an address; also its address on `https://reflow.<zone>/`. */
export function fallbackKey(address: ReflowAddress): string {
  return `${address.site}/${FALLBACK_DIR}/${address.page}.html`;
}

export async function libraryState(
  store: LibraryStore,
  bucketId: string,
  address: ReflowAddress,
): Promise<LibraryState> {
  const { site, page } = address;
  const fallback = madeOf(await readJson(store, bucketId, `${site}/${FALLBACK_DIR}/${page}.json`));
  return { url: address.url, site, page, fallback, view: fallback ? fallbackKey(address) : null };
}

export interface SiteEntry {
  site: string;
  /** How many fallback pages the library keeps for it. */
  pages: number;
}

/** The sites that have fallback pages. */
export async function listSites(
  store: LibraryStore,
  bucketId: string,
): Promise<SiteEntry[]> {
  const sites: string[] = [];
  let startAfter: string | undefined;
  do {
    const page = await store.listObjects(bucketId, {
      delimiter: "/",
      ...(startAfter ? { start_after: startAfter } : {}),
    });
    sites.push(...page.common_prefixes.map((prefix) => prefix.slice(0, -1)).filter(isSite));
    startAfter = page.next_start_after;
  } while (startAfter);
  const entries: SiteEntry[] = [];
  for (const site of sites) {
    const pages = (await listKeys(store, bucketId, `${site}/${FALLBACK_DIR}/`))
      .filter((key) => key.endsWith(".html")).length;
    if (pages) entries.push({ site, pages });
  }
  return entries;
}

/** Removes everything the library keeps for `site`. */
export async function removeSite(
  store: LibraryStore,
  bucketId: string,
  site: string,
): Promise<number> {
  if (!isSite(site)) throw new ReflowError(`not a site name: ${site}`);
  const keys = await listKeys(store, bucketId, `${site}/`);
  for (const key of keys) await store.delete(bucketId, key);
  return keys.length;
}

export interface FallbackReport {
  key: string;
  state: LibraryState;
}

/** Keeps the `.html` file as the fallback page of one address. */
export async function putFallback(
  store: LibraryStore,
  bucketId: string,
  address: ReflowAddress,
  file: string,
  madeBy: string,
  now: number = Date.now(),
): Promise<FallbackReport> {
  const name = file.toLowerCase();
  if (!name.endsWith(".html") && !name.endsWith(".htm")) {
    throw new ReflowError(
      `${file}: a fallback page is a .html file. A website the user keeps reading is a source of the reading site instead: site --help`,
    );
  }
  const key = fallbackKey(address);
  await store.putBytes(bucketId, key, await Deno.readFile(file), {
    contentType: contentTypeOf(key),
  });
  await store.putBytes(
    bucketId,
    `${address.site}/${FALLBACK_DIR}/${address.page}.json`,
    JSON.stringify({ schema: FALLBACK_SCHEMA, source_url: address.url, made_at: now, made_by: madeBy }),
    { contentType: "application/json" },
  );
  return { key, state: await libraryState(store, bucketId, address) };
}
