// What a source package's `extract.ts` runs in: Personal Site Protocol §4.2.
// Polite HTTP (one User-Agent, one request a second per host, conditional
// requests), HTML / XML through linkedom, feeds, article DOM to blocks, and the
// Agent's browser for the sources that need it.

import { agentServiceAccess, callAgentService, serviceError } from "../agent_service.ts";
import { SiteError } from "./types.ts";
import type {
  Extract,
  ExtractCtx,
  Extracted,
  FeedEntry,
  FetchResult,
  RawArticle,
  RawBlock,
  RawLink,
  SourceDef,
  TextBlockType,
} from "./types.ts";

export const USER_AGENT = "Mozilla/5.0 (compatible; BuckyOS-Reader/1.0; personal reading site)";
export const EXTRACT_FILE = "extract.ts";
const FETCH_TIMEOUT_MS = 30_000;
const HOST_INTERVAL_MS = 1000;
const MAX_PAGE_BYTES = 8 * 1024 * 1024;
const MAX_STATE_BYTES = 64 * 1024;
const MAX_VALIDATORS = 200;
const MAX_LINKS = 100;
const TEXT_BLOCKS = ["p", "h2", "h3", "quote", "li", "code"];

export interface Validators {
  etag?: string;
  last_modified?: string;
}

export interface NetResponse {
  status: number;
  url: string;
  bytes: Uint8Array;
  headers: Headers;
  notModified: boolean;
}

export interface NetOptions {
  fetcher?: typeof fetch;
  now?: () => number;
  sleep?: (ms: number) => Promise<void>;
}

/** HTTP for a sync: every request of the run goes through one of these. */
export class Net {
  requests = 0;
  bytes = 0;
  notModified = 0;
  private readonly next = new Map<string, number>();
  private readonly fetcher: typeof fetch;
  private readonly now: () => number;
  private readonly sleep: (ms: number) => Promise<void>;

  constructor(options: NetOptions = {}) {
    this.fetcher = options.fetcher ?? fetch;
    this.now = options.now ?? Date.now;
    this.sleep = options.sleep ?? ((ms) => new Promise((resolve) => setTimeout(resolve, ms)));
  }

  /** Network failures, 429 and 5xx are `transient`; every other status is the caller's to judge. */
  async get(
    input: string,
    init: { headers?: Record<string, string>; validators?: Validators; maxBytes?: number } = {},
  ): Promise<NetResponse> {
    let target: URL;
    try {
      target = new URL(input);
    } catch {
      throw new SiteError("program", `not a URL: ${input}`);
    }
    if (target.protocol !== "http:" && target.protocol !== "https:") {
      throw new SiteError("program", `only http(s) is fetched: ${input}`);
    }
    const now = this.now();
    const at = Math.max(now, this.next.get(target.host) ?? 0);
    this.next.set(target.host, at + HOST_INTERVAL_MS);
    if (at > now) await this.sleep(at - now);

    const headers: Record<string, string> = {
      "user-agent": USER_AGENT,
      accept: "*/*",
      ...init.headers,
    };
    if (init.validators?.etag) headers["if-none-match"] = init.validators.etag;
    if (init.validators?.last_modified) headers["if-modified-since"] = init.validators.last_modified;
    this.requests += 1;
    let response: Response;
    let bytes: Uint8Array;
    try {
      response = await this.fetcher(target.href, {
        headers,
        redirect: "follow",
        signal: AbortSignal.timeout(FETCH_TIMEOUT_MS),
      });
      bytes = new Uint8Array(await response.arrayBuffer());
    } catch (err) {
      throw new SiteError("transient", `${target.href}: ${(err as Error).message}`);
    }
    this.bytes += bytes.byteLength;
    if (response.status === 429 || response.status >= 500) {
      throw new SiteError("transient", `${target.href}: HTTP ${response.status}`);
    }
    const max = init.maxBytes ?? MAX_PAGE_BYTES;
    if (bytes.byteLength > max) {
      throw new SiteError("program", `${target.href}: ${bytes.byteLength} bytes, more than the ${max} taken`);
    }
    const notModified = response.status === 304;
    if (notModified) this.notModified += 1;
    return { status: response.status, url: response.url || target.href, bytes, headers: response.headers, notModified };
  }
}

/** The text of a page: the charset its headers or its first bytes name, UTF-8 otherwise. */
export function decodeText(bytes: Uint8Array, contentType: string | null): string {
  const head = new TextDecoder("latin1").decode(bytes.subarray(0, 2048));
  const label = /charset\s*=\s*["']?([\w-]+)/i.exec(contentType ?? "")?.[1] ??
    /<\?xml[^>]*encoding\s*=\s*["']([\w-]+)/i.exec(head)?.[1] ??
    /<meta[^>]*charset\s*=\s*["']?([\w-]+)/i.exec(head)?.[1];
  if (label) {
    try {
      return new TextDecoder(label).decode(bytes);
    } catch {
      // an unknown label: read it as UTF-8
    }
  }
  return new TextDecoder().decode(bytes);
}

// ── HTML / XML ─────────────────────────────────────────────────────────────

export interface Dom {
  html(text: string): Document;
  xml(text: string): Document;
}

let loaded: Promise<Dom> | undefined;

/** linkedom, loaded when a site command first parses something. */
export function loadDom(): Promise<Dom> {
  // A distributed copy imports the bundle in vendor/, which has no types: the two entry points are typed here.
  loaded ??= import("linkedom").then((module) => {
    const { DOMParser, parseHTML } = module as unknown as {
      DOMParser: new () => { parseFromString(text: string, type: string): unknown };
      parseHTML(text: string): { document: unknown };
    };
    return {
      html: (text: string) => parseHTML(text).document as Document,
      xml: (text: string) => new DOMParser().parseFromString(text, "text/xml") as Document,
    };
  });
  return loaded;
}

const tag = (element: Element) => element.tagName.toLowerCase();
const tidy = (text: string) =>
  text.replace(/[^\S\n]+/g, " ").replace(/ ?\n ?/g, "\n").trim();

function absolute(href: string | null | undefined, base: string): string | null {
  if (!href) return null;
  try {
    const url = new URL(href.trim(), base);
    return url.protocol === "http:" || url.protocol === "https:" ? url.href : null;
  } catch {
    return null;
  }
}

function timeOf(text: string | undefined): number | undefined {
  const ms = text ? Date.parse(text.trim()) : NaN;
  return Number.isFinite(ms) ? ms : undefined;
}

/** The entries of an RSS 1.0 / 2.0 or Atom document, in its order. */
export function feedEntries(doc: Document, dom: Dom): FeedEntry[] {
  const child = (parent: Element, ...names: string[]) =>
    Array.from(parent.children).find((node) => names.includes(tag(node)));
  const text = (parent: Element, ...names: string[]) => child(parent, ...names)?.textContent?.trim() || undefined;
  const plain = (html: string | undefined) =>
    html === undefined
      ? undefined
      : tidy(dom.html(`<html><body>${html}</body></html>`).body?.textContent ?? "") || undefined;
  const firstImage = (html: string | undefined) => /<img[^>]+src\s*=\s*["']([^"']+)["']/i.exec(html ?? "")?.[1];

  const root = doc.documentElement;
  const atom = !!root && tag(root) === "feed";
  const nodes = atom
    ? Array.from(root.children).filter((node) => tag(node) === "entry")
    : Array.from(doc.querySelectorAll("item"));
  const entries: FeedEntry[] = [];
  for (const node of nodes) {
    let url: string | undefined;
    if (atom) {
      const links = Array.from(node.children).filter((n) => tag(n) === "link" && n.getAttribute("href"));
      url = (links.find((n) => (n.getAttribute("rel") ?? "alternate") === "alternate") ?? links[0])
        ?.getAttribute("href") ?? undefined;
    } else {
      url = text(node, "link") ?? text(node, "guid");
    }
    const title = tidy(text(node, "title") ?? "");
    if (!url || !/^https?:\/\//i.test(url)) continue;
    const description = text(node, atom ? "summary" : "description");
    const content = text(node, atom ? "content" : "content:encoded");
    const media = Array.from(node.children).find((n) =>
      (tag(n) === "media:thumbnail" && n.getAttribute("url")) ||
      (["media:content", "enclosure"].includes(tag(n)) && n.getAttribute("url") &&
        /^image/.test(n.getAttribute("type") ?? n.getAttribute("medium") ?? ""))
    );
    const image = media?.getAttribute("url") ?? firstImage(content) ?? firstImage(description);
    const entry: FeedEntry = { url, title };
    const published = timeOf(text(node, ...(atom ? ["published", "updated"] : ["pubdate", "dc:date"])));
    if (published !== undefined) entry.published_at = published;
    const summary = plain(description);
    if (summary) entry.summary = summary;
    if (image && /^https?:\/\//i.test(image)) entry.image = image;
    if (content) entry.content_html = content;
    const comments = atom ? undefined : text(node, "comments");
    if (comments && /^https?:\/\//i.test(comments)) entry.comments_url = comments;
    entries.push(entry);
  }
  return entries;
}

const SKIPPED = new Set([
  "script", "style", "noscript", "template", "iframe", "svg", "nav", "aside", "footer", "form", "button",
  "select", "input", "textarea", "video", "audio", "canvas", "object", "dialog", "menu", "head",
]);
const CONTAINERS = new Set([
  "p", "div", "section", "article", "main", "header", "address", "dl", "dt", "dd", "fieldset", "figcaption",
  "details", "summary", "center", "body", "html", "li", "tr", "td", "th", "tbody", "thead", "tfoot", "hgroup",
]);
const NESTED_IN_TABLE = "table, p, div, ul, ol, img, h1, h2, h3, h4, pre, blockquote";

function hidden(element: Element): boolean {
  return element.hasAttribute("hidden") || element.getAttribute("aria-hidden") === "true" ||
    /display\s*:\s*none/i.test(element.getAttribute("style") ?? "");
}

// The address of the picture an <img> shows; null for what is no picture to keep: a spacer, a `data:` URI, an SVG.
function imageUrl(image: Element, base: string, note?: (message: string) => void): string | null {
  const small = (name: string) => {
    const value = Number(image.getAttribute(name));
    return image.hasAttribute(name) && Number.isFinite(value) && value <= 16;
  };
  if (small("width") && small("height")) return null;
  let svg: string | undefined;
  const srcset = (image.getAttribute("srcset") ?? image.getAttribute("data-srcset") ?? "")
    .split(",").map((part) => part.trim().split(/\s+/)[0]).filter(Boolean);
  for (const candidate of [
    image.getAttribute("data-src"),
    image.getAttribute("data-original"),
    image.getAttribute("data-lazy-src"),
    image.getAttribute("src"),
    srcset.at(-1),
  ]) {
    if (candidate && !candidate.startsWith("data:")) {
      const url = absolute(candidate, base);
      if (!url) continue;
      if (!/\.svg$/i.test(new URL(url).pathname)) return url;
      svg ??= url;
    }
  }
  if (svg) note?.(`image ${svg}: an SVG is not a picture the site keeps — left out of the article`);
  return null;
}

/**
 * An article's DOM as blocks: paragraphs, headings (h1/h2 → h2, deeper → h3),
 * quotes, list items, code, images and tables in document order, and the
 * http(s) links found on the way. Navigation, forms, scripts and hidden
 * elements are left out.
 */
export function domBlocks(
  root: Element,
  base: string,
  note?: (message: string) => void,
): { blocks: RawBlock[]; links: RawLink[] } {
  const blocks: RawBlock[] = [];
  const links: RawLink[] = [];
  const linked = new Set<string>();
  let inline = "";
  let mode: TextBlockType = "p";

  const flush = () => {
    for (const part of inline.split(/\n{2,}/)) {
      const text = tidy(part);
      if (text) blocks.push({ type: mode, text });
    }
    inline = "";
  };
  const within = (type: TextBlockType, body: () => void) => {
    flush();
    const outer = mode;
    mode = type;
    body();
    flush();
    mode = outer;
  };
  const image = (node: Element, caption?: string) => {
    const url = imageUrl(node, base, note);
    if (!url) return;
    flush();
    const alt = tidy(node.getAttribute("alt") ?? "") || caption || "";
    blocks.push(alt ? { type: "img", url, alt } : { type: "img", url });
  };
  const children = (node: Node) => {
    for (const next of Array.from(node.childNodes)) walk(next);
  };
  const walk = (node: Node) => {
    if (node.nodeType === 3) {
      inline += (node.nodeValue ?? "").replace(/\s+/g, " ");
      return;
    }
    if (node.nodeType !== 1) return;
    const element = node as Element;
    const name = tag(element);
    if (SKIPPED.has(name) || hidden(element)) return;
    switch (name) {
      case "br":
        inline += "\n";
        return;
      case "hr":
        flush();
        return;
      case "a": {
        const href = element.getAttribute("href");
        const url = href?.trim().startsWith("#") ? null : absolute(href, base);
        const text = tidy(element.textContent ?? "");
        if (url && text && !linked.has(url) && links.length < MAX_LINKS) {
          linked.add(url);
          links.push({ url, text });
        }
        children(element);
        return;
      }
      case "img":
        image(element);
        return;
      case "picture": {
        const inner = element.querySelector("img");
        if (inner) image(inner);
        return;
      }
      case "figure": {
        const images = Array.from(element.querySelectorAll("img"));
        if (!images.length) {
          within(mode, () => children(element));
          return;
        }
        const caption = tidy(element.querySelector("figcaption")?.textContent ?? "");
        for (const inner of images) image(inner, caption);
        return;
      }
      case "h1":
      case "h2":
      case "h3":
      case "h4":
      case "h5":
      case "h6": {
        flush();
        const text = tidy(element.textContent ?? "");
        if (text) blocks.push({ type: name === "h1" || name === "h2" ? "h2" : "h3", text });
        return;
      }
      case "blockquote":
        within("quote", () => children(element));
        return;
      case "ul":
      case "ol":
        for (const item of Array.from(element.children)) {
          if (tag(item) === "li") within("li", () => children(item));
          else walk(item);
        }
        return;
      case "pre": {
        flush();
        const text = (element.textContent ?? "").replace(/\s+$/, "");
        if (text.trim()) blocks.push({ type: "code", text });
        return;
      }
      case "table": {
        if (element.querySelector(NESTED_IN_TABLE)) {
          within(mode, () => children(element));
          return;
        }
        flush();
        const rows = Array.from(element.querySelectorAll("tr"))
          .map((row) => Array.from(row.children).map((cell) => tidy(cell.textContent ?? "")))
          .filter((row) => row.some(Boolean));
        if (rows.length) blocks.push({ type: "table", rows });
        return;
      }
    }
    if (CONTAINERS.has(name)) within(mode, () => children(element));
    else children(element);
  };

  walk(root);
  flush();
  return { blocks, links };
}

// ── the context ────────────────────────────────────────────────────────────

/** Runs a script of the package in a background tab of the Agent's browser (Agent Browser Protocol §7.1). */
export type BrowserRun = (params: { script_path: string; url?: string; args?: unknown }) => Promise<unknown>;

const agentBrowser: BrowserRun = async (params) => {
  const access = agentServiceAccess();
  if (!access) {
    throw new SiteError(
      "transient",
      "the Agent's browser is reachable only from the Agent's own runtime ($OPENDAN_BROWSER_RPC and $BUCKYOS_APPCLIENT_SESSION_TOKEN are not both set here)",
    );
  }
  try {
    const answer = await callAgentService<{ data: unknown }>(
      access.endpoint,
      access.token,
      "browser.background_run",
      params,
      400_000,
    );
    return answer.data;
  } catch (err) {
    const { code, message } = serviceError(err);
    if (message.startsWith("auth: ")) throw new SiteError("auth", message.slice("auth: ".length));
    // No browser, a site that cannot be reached, a run that took all its time: try again later. Anything else
    // is the script's — also a wait for something the page no longer has, which ends well before the run does.
    const transient = code === "browser_unavailable" || !code ||
      /net::ERR_|timed out after \d+ seconds|page\.goto: Timeout/.test(message);
    throw new SiteError(transient ? "transient" : "program", `browser.background_run: ${message}`);
  }
};

export interface ExtractEnv {
  source: SourceDef;
  /** The source package: `ctx.browser` scripts are relative to it. */
  dir: string;
  state: unknown;
  /** ETag / Last-Modified of the URLs fetched with `conditional`; updated in place. */
  validators: Record<string, Validators>;
  limit: number;
  known: (url: string) => boolean;
  net: Net;
  log: (message: string) => void;
  browser?: BrowserRun;
}

export async function extractContext(env: ExtractEnv): Promise<ExtractCtx> {
  const dom = await loadDom();
  const browser = env.browser ?? agentBrowser;
  return {
    source: env.source,
    state: env.state ?? null,
    limit: env.limit,
    known: env.known,
    log: env.log,
    async fetch(url, init = {}): Promise<FetchResult> {
      const kept = init.conditional ? env.validators[url] : undefined;
      const response = await env.net.get(url, { headers: init.headers, validators: kept });
      if (init.conditional && response.status === 200) {
        const validators: Validators = {};
        const etag = response.headers.get("etag");
        const modified = response.headers.get("last-modified");
        if (etag) validators.etag = etag;
        if (modified) validators.last_modified = modified;
        delete env.validators[url];
        if (etag || modified) env.validators[url] = validators;
        for (const old of Object.keys(env.validators).slice(0, -MAX_VALIDATORS)) delete env.validators[old];
      }
      return {
        status: response.status,
        url: response.url,
        text: response.notModified ? "" : decodeText(response.bytes, response.headers.get("content-type")),
        notModified: response.notModified,
        headers: response.headers,
      };
    },
    html: dom.html,
    xml: dom.xml,
    feed: (text) => feedEntries(dom.xml(text), dom),
    blocks: (root, base) => domBlocks(root, base, env.log),
    async browser<T>(url: string | null, script: string, args?: unknown): Promise<T> {
      if (env.source.needs === "http") {
        throw new SiteError("program", 'ctx.browser is for sources whose source.json says needs "browser" or "login"');
      }
      if (!script.endsWith(".mjs") || script.startsWith("/") || script.split("/").includes("..")) {
        throw new SiteError("program", `ctx.browser runs a .mjs file of the package, not ${script}`);
      }
      const data = await browser({
        script_path: `${await Deno.realPath(env.dir)}/${script}`,
        ...(url ? { url } : {}),
        ...(args === undefined ? {} : { args }),
      });
      return data as T;
    },
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object" && !Array.isArray(value);
}

function isHttpUrl(value: unknown): value is string {
  return typeof value === "string" && /^https?:\/\/[^/\s]+/i.test(value);
}

function rawBlockProblem(block: unknown): string | null {
  if (!isRecord(block)) return "is not an object";
  if (TEXT_BLOCKS.includes(block.type as string)) return typeof block.text === "string" ? null : "has no text";
  if (block.type === "img") return isHttpUrl(block.url) ? null : "has no http(s) url";
  if (block.type === "table") {
    const ok = Array.isArray(block.rows) &&
      block.rows.every((row) => Array.isArray(row) && row.every((cell) => typeof cell === "string"));
    return ok ? null : "has no rows of text";
  }
  return `has the unknown type ${JSON.stringify(block.type)}`;
}

/** What is wrong with what an `extract` returned; empty when it follows §4.2. */
export function extractedProblems(value: unknown): string[] {
  if (!isRecord(value) || !Array.isArray(value.list) || !Array.isArray(value.items)) {
    return ["extract must return { list: [...], items: [...], state? }"];
  }
  const problems: string[] = [];
  value.list.forEach((entry, index) => {
    if (!isRecord(entry) || !isHttpUrl(entry.url) || typeof entry.title !== "string") {
      problems.push(`list[${index}] must be { url, title, … }`);
    }
  });
  value.items.forEach((item, index) => {
    const at = `items[${index}]`;
    if (!isRecord(item) || !isHttpUrl(item.url) || typeof item.title !== "string" || !Array.isArray(item.blocks)) {
      problems.push(`${at} must be { url, title, blocks, … }`);
      return;
    }
    item.blocks.forEach((block: unknown, at2: number) => {
      const problem = rawBlockProblem(block);
      if (problem) problems.push(`${at}.blocks[${at2}] ${problem}`);
    });
    const comments = item.comments ?? [];
    if (!Array.isArray(comments) || !comments.every((c) => isRecord(c) && typeof c.text === "string")) {
      problems.push(`${at}.comments must list { author?, at?, text }`);
    }
    const links = item.links ?? [];
    if (!Array.isArray(links) || !links.every((l) => isRecord(l) && isHttpUrl(l.url) && typeof l.text === "string")) {
      problems.push(`${at}.links must list { url, text }`);
    }
  });
  if (value.state !== undefined && new TextEncoder().encode(JSON.stringify(value.state)).length > MAX_STATE_BYTES) {
    problems.push(`state is larger than ${MAX_STATE_BYTES} bytes`);
  }
  return problems.slice(0, 10);
}

/** How a failure of an extractor counts: what it says itself, `program` otherwise. */
export function failureOf(err: unknown): SiteError {
  if (err instanceof SiteError) return err;
  const kind = (err as { kind?: unknown } | null)?.kind;
  const message = err instanceof Error ? err.message : String(err);
  return new SiteError(kind === "auth" || kind === "transient" ? kind : "program", message);
}

/** Loads the package's `extract.ts` and runs it once; the result is checked and cut to `ctx.limit`. */
export async function runExtract(dir: string, ctx: ExtractCtx): Promise<Extracted> {
  // `throw new SiteError(kind, message)` is how a package says how it failed, and a package imports nothing of ours.
  (globalThis as { SiteError?: unknown }).SiteError = SiteError;
  let extract: Extract;
  try {
    const module = await import(`file://${await Deno.realPath(`${dir}/${EXTRACT_FILE}`)}`);
    extract = module.default;
  } catch (err) {
    throw new SiteError("program", `${EXTRACT_FILE} does not load: ${(err as Error).message}`);
  }
  if (typeof extract !== "function") {
    throw new SiteError("program", `${EXTRACT_FILE} must export the extract function as its default`);
  }
  let out: Extracted;
  try {
    out = await extract(ctx);
  } catch (err) {
    throw failureOf(err);
  }
  const problems = extractedProblems(out);
  if (problems.length) throw new SiteError("program", `${EXTRACT_FILE} returned something else: ${problems.join("; ")}`);
  return { list: out.list, items: (out.items as RawArticle[]).slice(0, Math.max(0, ctx.limit)), state: out.state };
}
