// The reflow library: a user's bucket `user/control-panel/<user>/reflow`
// holding, per site, a package (manifest.json, refresh.mjs, index.html and
// what it refers to), the results made with it and fallback pages.
// Rules: doc/opendan/protocol/Agent Browser Protocol.md §5.1–5.2. The Agent
// Service follows the same ones; both sides are checked against
// doc/opendan/protocol/fixtures/reflow_vectors.json.

export const REFLOW_APP = "control-panel";
export const REFLOW_BUCKET = "reflow";
export const MANIFEST_SCHEMA = "buckyos.reflow/1";
export const RESULT_SCHEMA = "buckyos.reflow.result/1";
export const FALLBACK_SCHEMA = "buckyos.reflow.fallback/1";
export const MANIFEST_FILE = "manifest.json";
export const REFRESH_SCRIPT = "refresh.mjs";
export const INDEX_FILE = "index.html";
// Kept by the library beside a package; never part of one.
export const DATA_DIRS = ["results", "fallback"];
// `<site>/preference.json`: the user's preference for a site that has no
// package yet (the App writes it); a package states it in its manifest.
export const PREFERENCE_FILE = "preference.json";
export const DEFAULT_REFRESH_TIMEOUT_S = 60;
export const MAX_REFRESH_TIMEOUT_S = 600;
// `PUT /bucket/v1/...` takes one request body up to this size.
const MAX_FILE_BYTES = 32 * 1024 * 1024;
const MAX_PACKAGE_FILES = 500;

export class ReflowError extends Error {
  constructor(message: string, readonly problems: string[] = []) {
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

// ── §5.2: the package ──────────────────────────────────────────────────────

export interface MatchRule {
  host: string;
  path?: string;
}

export interface Author {
  kind: "agent" | "user";
  id: string;
}

export interface Manifest {
  schema: string;
  site: string;
  match: MatchRule[];
  preference?: string;
  author: Author;
  updated_at: number;
  refresh?: { script: string; timeout_s: number };
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

export function packageMatches(
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

/** What is wrong with a manifest kept at `<site>/manifest.json`; empty when it is valid. */
export function manifestProblems(value: unknown, site: string): string[] {
  if (!isRecord(value)) return ["manifest.json is not a JSON object"];
  const problems: string[] = [];
  if (value.schema !== MANIFEST_SCHEMA) {
    problems.push(`schema must be "${MANIFEST_SCHEMA}"`);
  }
  if (value.site !== site) problems.push(`site must be "${site}"`);
  if (!Array.isArray(value.match) || value.match.length === 0) {
    problems.push("match must list at least one { host, path }");
  } else {
    value.match.forEach((rule, index) => {
      const at = `match[${index}]`;
      if (!isRecord(rule) || typeof rule.host !== "string") {
        problems.push(`${at}.host must be a host name`);
        return;
      }
      let ruleSite: string | undefined;
      try {
        ruleSite = siteOfHost(rule.host);
      } catch {
        // reported below
      }
      if (
        ruleSite !== site || rule.host !== rule.host.toLowerCase() ||
        rule.host.endsWith(".")
      ) {
        problems.push(
          `${at}.host "${rule.host}" is not the lower-case host name of site ${site}`,
        );
      }
      if (
        rule.path !== undefined &&
        (typeof rule.path !== "string" || !rule.path.startsWith("/"))
      ) {
        problems.push(`${at}.path must be a path pattern starting with "/"`);
      }
    });
  }
  if (value.preference !== undefined && typeof value.preference !== "string") {
    problems.push("preference must be text");
  }
  const author = value.author;
  if (
    !isRecord(author) || (author.kind !== "agent" && author.kind !== "user") ||
    typeof author.id !== "string" || !author.id
  ) {
    problems.push('author must be { "kind": "agent" | "user", "id": "<who>" }');
  }
  if (!Number.isSafeInteger(value.updated_at) || (value.updated_at as number) <= 0) {
    problems.push("updated_at must be a millisecond timestamp");
  }
  if (value.refresh !== undefined) {
    const refresh = value.refresh;
    if (!isRecord(refresh) || refresh.script !== REFRESH_SCRIPT) {
      problems.push(`refresh.script must be "${REFRESH_SCRIPT}"`);
    }
    const timeout = isRecord(refresh) ? refresh.timeout_s : undefined;
    if (
      !Number.isSafeInteger(timeout) || (timeout as number) < 1 ||
      (timeout as number) > MAX_REFRESH_TIMEOUT_S
    ) {
      problems.push(`refresh.timeout_s must be 1 to ${MAX_REFRESH_TIMEOUT_S} seconds`);
    }
  }
  return problems;
}

/** What is wrong with a package (its files, relative to its directory, and its manifest). */
export function packageProblems(
  files: string[],
  manifest: unknown,
  site: string,
): string[] {
  const problems: string[] = [];
  if (!files.includes(MANIFEST_FILE)) {
    problems.push(`${MANIFEST_FILE} is missing`);
  } else {
    problems.push(...manifestProblems(manifest, site));
  }
  if (!files.includes(REFRESH_SCRIPT) && !files.includes(INDEX_FILE)) {
    problems.push(`a package has ${REFRESH_SCRIPT}, ${INDEX_FILE} or both`);
  }
  if (isRecord(manifest) && manifest.refresh !== undefined && !files.includes(REFRESH_SCRIPT)) {
    problems.push(`the manifest declares refresh but ${REFRESH_SCRIPT} is missing`);
  }
  for (const dir of DATA_DIRS) {
    if (files.some((file) => file.startsWith(`${dir}/`))) {
      problems.push(`${dir}/ holds the library's data and is not part of a package`);
    }
  }
  if (files.includes(PREFERENCE_FILE)) {
    problems.push(
      `${PREFERENCE_FILE} is where the library keeps the user's preference for a site without a package and is not part of one: state the preference in ${MANIFEST_FILE}`,
    );
  }
  if (files.length > MAX_PACKAGE_FILES) {
    problems.push(`a package has at most ${MAX_PACKAGE_FILES} files, this one has ${files.length}`);
  }
  return problems;
}

/** The type a package file is served with on `reflow.<zone>`. */
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

function isDataKey(site: string, key: string): boolean {
  return DATA_DIRS.some((dir) => key.startsWith(`${site}/${dir}/`));
}

async function validManifest(
  store: LibraryStore,
  bucketId: string,
  site: string,
): Promise<Manifest | null> {
  const manifest = await readJson(store, bucketId, `${site}/${MANIFEST_FILE}`);
  return manifestProblems(manifest, site).length === 0 ? manifest as Manifest : null;
}

async function keptPreference(
  store: LibraryStore,
  bucketId: string,
  site: string,
): Promise<string> {
  const kept = await readJson(store, bucketId, `${site}/${PREFERENCE_FILE}`);
  return isRecord(kept) && typeof kept.preference === "string" ? kept.preference : "";
}

/** The user's preference for a site: the package's when the site has a valid one, else the one kept beside it. */
export async function readPreference(
  store: LibraryStore,
  bucketId: string,
  site: string,
): Promise<string> {
  const manifest = await validManifest(store, bucketId, site);
  return manifest ? manifest.preference ?? "" : await keptPreference(store, bucketId, site);
}

export interface Made {
  made_at: number;
  made_by: string;
  source_url: string;
}

/** What the library has for one URL: `browser.reflow_state` without `refresh`. */
export interface LibraryState {
  url: string;
  site: string;
  page: string;
  package: { updated_at: number; author: Author; preference: string } | null;
  result: Made | null;
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

async function matchingManifest(
  store: LibraryStore,
  bucketId: string,
  address: ReflowAddress,
): Promise<Manifest | null> {
  const manifest = await validManifest(store, bucketId, address.site);
  return manifest && packageMatches(manifest.match, address) ? manifest : null;
}

export function viewOf(
  address: ReflowAddress,
  shown: { result: boolean; fallback: boolean },
): string | null {
  if (shown.result) {
    return `${address.site}/?url=${encodeURIComponent(address.url)}&page=${address.page}`;
  }
  return shown.fallback ? `${address.site}/fallback/${address.page}.html` : null;
}

export async function libraryState(
  store: LibraryStore,
  bucketId: string,
  address: ReflowAddress,
): Promise<LibraryState> {
  const { site, page } = address;
  const manifest = await matchingManifest(store, bucketId, address);
  const result = madeOf(await readJson(store, bucketId, `${site}/results/${page}.json`));
  const fallback = madeOf(await readJson(store, bucketId, `${site}/fallback/${page}.json`));
  return {
    url: address.url,
    site,
    page,
    package: manifest && {
      updated_at: manifest.updated_at,
      author: manifest.author,
      preference: manifest.preference ?? "",
    },
    result,
    fallback,
    view: viewOf(address, { result: !!manifest && !!result, fallback: !!fallback }),
  };
}

export interface SiteEntry {
  site: string;
  /** `null`: the site has results, fallback pages or a preference but no valid package. */
  package: { match: MatchRule[]; updated_at: number; author: Author } | null;
  /** The user's preference for the site (`readPreference`); empty when there is none. */
  preference: string;
}

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
    sites.push(...page.common_prefixes.map((prefix) => prefix.slice(0, -1)));
    startAfter = page.next_start_after;
  } while (startAfter);
  const entries: SiteEntry[] = [];
  for (const site of sites) {
    const manifest = await validManifest(store, bucketId, site);
    entries.push({
      site,
      package: manifest && {
        match: manifest.match,
        updated_at: manifest.updated_at,
        author: manifest.author,
      },
      preference: manifest
        ? manifest.preference ?? ""
        : await keptPreference(store, bucketId, site),
    });
  }
  return entries;
}

/** The files of a package directory, relative, `/`-separated, sorted; hidden entries are left out. */
export async function packageFiles(dir: string): Promise<string[]> {
  const files: string[] = [];
  const walk = async (path: string, prefix: string) => {
    for await (const entry of Deno.readDir(path)) {
      if (entry.name.startsWith(".")) continue;
      const relative = `${prefix}${entry.name}`;
      if (entry.isSymlink) {
        throw new ReflowError(`${relative} is a symbolic link; a package holds plain files`);
      }
      if (entry.isDirectory) {
        await walk(`${path}/${entry.name}`, `${relative}/`);
      } else {
        files.push(relative);
      }
    }
  };
  await walk(dir, "");
  return files.sort();
}

export interface PutReport {
  site: string;
  files: string[];
  /** What the new package replaced: files of the old one it no longer has, and `preference.json`. */
  removed: string[];
  manifest: Manifest;
}

/**
 * Checks the package in `dir` and makes it the package of `site`: every file
 * goes to `<site>/<path>`, the manifest last (stamped with `updated_at`, with
 * `author` when it names none and `agentDid` is known, and with the site's
 * preference when it states none); files of the replaced package that are
 * gone are deleted, and so is `preference.json`, whose place the manifest
 * takes. Results and fallback pages stay. The App's import follows the same
 * rules (Agent Browser Protocol §5.2).
 */
export async function putPackage(
  store: LibraryStore,
  bucketId: string,
  site: string,
  dir: string,
  options: { agentDid?: string; now?: number } = {},
): Promise<PutReport> {
  if (!isSite(site)) throw new ReflowError(`not a site name: ${site}`);
  const files = await packageFiles(dir);
  let manifest: unknown;
  if (files.includes(MANIFEST_FILE)) {
    try {
      manifest = JSON.parse(await Deno.readTextFile(`${dir}/${MANIFEST_FILE}`));
    } catch (err) {
      throw new ReflowError(`${MANIFEST_FILE} is not JSON: ${(err as Error).message}`);
    }
    if (isRecord(manifest)) {
      manifest.updated_at = options.now ?? Date.now();
      if (manifest.author === undefined && options.agentDid) {
        manifest.author = { kind: "agent", id: options.agentDid };
      }
    }
  }
  const problems = packageProblems(files, manifest, site);
  for (const file of files) {
    const { size } = await Deno.stat(`${dir}/${file}`);
    if (size > MAX_FILE_BYTES) {
      problems.push(`${file} is ${size} bytes; a package file is at most ${MAX_FILE_BYTES}`);
    }
  }
  if (problems.length > 0) {
    throw new ReflowError(`the package in ${dir} is not valid: ${problems.join("; ")}`, problems);
  }

  const next = manifest as Manifest;
  if (next.preference === undefined) {
    const kept = await readPreference(store, bucketId, site);
    if (kept) next.preference = kept;
  }
  const before = (await listKeys(store, bucketId, `${site}/`)).filter((key) =>
    !isDataKey(site, key)
  );
  for (const file of files.filter((file) => file !== MANIFEST_FILE)) {
    await store.putBytes(bucketId, `${site}/${file}`, await Deno.readFile(`${dir}/${file}`), {
      contentType: contentTypeOf(file),
    });
  }
  await store.putBytes(bucketId, `${site}/${MANIFEST_FILE}`, JSON.stringify(next, null, 2), {
    contentType: contentTypeOf(MANIFEST_FILE),
  });
  const removed: string[] = [];
  for (const key of before) {
    const file = key.slice(site.length + 1);
    if (!files.includes(file)) {
      await store.delete(bucketId, key);
      removed.push(file);
    }
  }
  return { site, files, removed, manifest: next };
}

async function isEmptyDir(dir: string): Promise<boolean> {
  try {
    for await (const _ of Deno.readDir(dir)) return false;
    return true;
  } catch (err) {
    if (err instanceof Deno.errors.NotFound) return true;
    throw err;
  }
}

/** Writes the package of `site` into `dir` (new or empty); results, fallback pages and `preference.json` are not part of it. */
export async function getPackage(
  store: LibraryStore,
  bucketId: string,
  site: string,
  dir: string,
): Promise<string[]> {
  if (!isSite(site)) throw new ReflowError(`not a site name: ${site}`);
  const keys = (await listKeys(store, bucketId, `${site}/`)).filter((key) =>
    !isDataKey(site, key) && key !== `${site}/${PREFERENCE_FILE}`
  );
  if (keys.length === 0) throw new ReflowError(`the library has no package for ${site}`);
  if (!await isEmptyDir(dir)) throw new ReflowError(`${dir} is not empty`);
  const files: string[] = [];
  for (const key of keys) {
    const file = key.slice(site.length + 1);
    if (file.split("/").some((segment) => !segment || segment === "." || segment === "..")) {
      throw new ReflowError(`the library key ${key} is not a file path`);
    }
    const response = await store.fetchObject(bucketId, key);
    const target = `${dir}/${file}`;
    await Deno.mkdir(target.slice(0, target.lastIndexOf("/")), { recursive: true });
    await Deno.writeFile(target, new Uint8Array(await response.arrayBuffer()));
    files.push(file);
  }
  return files;
}

/** Removes everything the library keeps for `site`: package, preference, results and fallback pages. */
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

export interface ResultReport {
  kind: "result" | "fallback";
  key: string;
  state: LibraryState;
}

/**
 * Keeps what was made for one page. A `.json` file is the data the site's
 * package draws (stored as `results/<page>.json`; the package must match the
 * URL); a `.html` file is a fallback page (`fallback/<page>.html`).
 */
export async function putResult(
  store: LibraryStore,
  bucketId: string,
  address: ReflowAddress,
  file: string,
  madeBy: string,
  now: number = Date.now(),
): Promise<ResultReport> {
  const { site, page } = address;
  const made: Made = { source_url: address.url, made_at: now, made_by: madeBy };
  const name = file.toLowerCase();
  let report: { kind: "result" | "fallback"; key: string };
  if (name.endsWith(".html") || name.endsWith(".htm")) {
    const key = `${site}/fallback/${page}.html`;
    await store.putBytes(bucketId, key, await Deno.readFile(file), {
      contentType: contentTypeOf(key),
    });
    await store.putBytes(
      bucketId,
      `${site}/fallback/${page}.json`,
      JSON.stringify({ schema: FALLBACK_SCHEMA, ...made }),
      { contentType: "application/json" },
    );
    report = { kind: "fallback", key };
  } else if (name.endsWith(".json")) {
    let data: unknown;
    try {
      data = JSON.parse(await Deno.readTextFile(file));
    } catch (err) {
      throw new ReflowError(`${file} is not JSON: ${(err as Error).message}`);
    }
    if (!await matchingManifest(store, bucketId, address)) {
      throw new ReflowError(
        `no package in the library matches ${address.url}: put the package first (reflow put ${site} <dir>), or give a .html file to keep a fallback page`,
      );
    }
    const key = `${site}/results/${page}.json`;
    await store.putBytes(bucketId, key, JSON.stringify({ schema: RESULT_SCHEMA, ...made, data }), {
      contentType: "application/json",
    });
    report = { kind: "result", key };
  } else {
    throw new ReflowError(
      `${file}: a result is a .json file (data for the site's package) or a .html file (a fallback page)`,
    );
  }
  return { ...report, state: await libraryState(store, bucketId, address) };
}
