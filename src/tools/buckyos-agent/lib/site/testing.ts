// What the site tests share: a bucket in memory, canned HTTP, source packages
// in temporary directories. Not used by the commands.

import type { SiteStore } from "./store.ts";

export function assertEquals(actual: unknown, expected: unknown, what = ""): void {
  const actualJson = JSON.stringify(actual);
  const expectedJson = JSON.stringify(expected);
  if (actualJson !== expectedJson) throw new Error(`${what} expected ${expectedJson}, got ${actualJson}`);
}

export function assert(condition: unknown, what: string): asserts condition {
  if (!condition) throw new Error(what);
}

export async function assertFails(fn: () => unknown, includes: string): Promise<Error> {
  try {
    await fn();
  } catch (err) {
    if (!(err instanceof Error) || !err.message.includes(includes)) {
      throw new Error(`expected a failure with ${JSON.stringify(includes)}, got ${err}`);
    }
    return err;
  }
  throw new Error(`expected a failure with ${JSON.stringify(includes)}`);
}

export const FIXTURES = new URL("../../../../../doc/opendan/protocol/fixtures/", import.meta.url);

export async function fixture(name: string): Promise<unknown> {
  return JSON.parse(await Deno.readTextFile(new URL(name, FIXTURES)));
}

const coded = (code: string, message: string) => Object.assign(new Error(message), { code });

async function sha256Hex(bytes: Uint8Array): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", bytes as BufferSource);
  return Array.from(new Uint8Array(digest), (b) => b.toString(16).padStart(2, "0")).join("");
}

interface Stored {
  body: Uint8Array;
  objId: string;
  contentType?: string;
  headers?: Record<string, string>;
  expiresAt?: number;
}

/** A bucket service in memory: conditions, paging (3 entries a page), ETags, and a log of what was written. */
export function memoryStore(initial: Record<string, string> = {}) {
  const objects = new Map<string, Stored>();
  const encode = (text: string) => new TextEncoder().encode(text);
  const log: string[] = [];
  const hooks: {
    readOnly: boolean;
    /** Called before a PUT is applied: a test writes "somebody else's" change here. */
    beforePut?: (key: string) => void | Promise<void>;
  } = { readOnly: false };
  const denied = () => coded("permission_denied", "write on user/control-panel/alice/reflow");
  const set = async (key: string, body: Uint8Array, contentType?: string, expiresAt?: number) => {
    const objId = `cyfile:${await sha256Hex(body)}`;
    objects.set(key, { body, objId, contentType, expiresAt });
    return objId;
  };
  const matches = (key: string, etag: string) =>
    objects.get(key)?.objId === etag.replace(/^W\//, "").replaceAll('"', "") || (etag === "*" && objects.has(key));

  const store: SiteStore = {
    listObjects(_bucketId, req) {
      const prefix = req.prefix ?? "";
      const after = req.start_after ?? "";
      const out = {
        items: [] as { key: string }[],
        common_prefixes: [] as string[],
        next_start_after: undefined as string | undefined,
      };
      let last = "";
      for (const key of [...objects.keys()].sort()) {
        if (!key.startsWith(prefix) || key <= after) continue;
        if (req.delimiter && after.endsWith(req.delimiter) && key.startsWith(after)) continue;
        const cut = req.delimiter ? key.indexOf(req.delimiter, prefix.length) : -1;
        const common = cut >= 0 ? key.slice(0, cut + 1) : undefined;
        if (common && out.common_prefixes.includes(common)) continue;
        if (out.items.length + out.common_prefixes.length === 3) {
          out.next_start_after = last;
          break;
        }
        last = common ?? key;
        if (common) out.common_prefixes.push(common);
        else out.items.push({ key });
      }
      return Promise.resolve(out);
    },
    fetchObject(_bucketId, key) {
      const object = objects.get(key);
      if (!object) throw coded("no_such_key", `no such key: ${key}`);
      return Promise.resolve(new Response(object.body as BodyInit, { headers: { etag: `"${object.objId}"` } }));
    },
    async putBytes(_bucketId, key, body, options) {
      if (hooks.readOnly) throw denied();
      await hooks.beforePut?.(key);
      if (options?.if_none_match && matches(key, options.if_none_match)) {
        throw coded("precondition_failed", "the key already exists");
      }
      if (options?.if_match && !matches(key, options.if_match)) {
        throw coded("precondition_failed", "the current etag differs");
      }
      log.push(`put ${key}`);
      return {
        obj_id: await set(key, typeof body === "string" ? encode(body) : body, options?.contentType, options?.expiresAt),
      };
    },
    put(_bucketId, req) {
      if (hooks.readOnly) throw denied();
      const object = objects.get(req.key);
      if (!object || object.objId !== req.obj_id) throw coded("invalid_argument", `${req.key} does not hold ${req.obj_id}`);
      log.push(`bind ${req.key}`);
      object.contentType = req.content_type;
      object.headers = req.http_headers;
      object.expiresAt = req.expires_at;
      return Promise.resolve({});
    },
    get(_bucketId, key) {
      const object = objects.get(key);
      if (!object) throw coded("no_such_key", `no such key: ${key}`);
      return Promise.resolve({ obj_id: object.objId, expires_at: object.expiresAt });
    },
    delete(_bucketId, key) {
      if (hooks.readOnly) throw denied();
      log.push(`delete ${key}`);
      objects.delete(key);
      return Promise.resolve({});
    },
  };
  const seed = async () => {
    for (const [key, text] of Object.entries(initial)) await set(key, encode(text));
  };
  return {
    store,
    objects,
    log,
    hooks,
    seed,
    /** Writes a key as somebody else would. */
    write: (key: string, text: string) => set(key, encode(text)),
    text: (key: string) => new TextDecoder().decode(objects.get(key)!.body),
    json: (key: string) => JSON.parse(new TextDecoder().decode(objects.get(key)!.body)),
    keys: (prefix = "") => [...objects.keys()].filter((key) => key.startsWith(prefix)).sort(),
  };
}

export type Page = string | Uint8Array | { status?: number; body?: string | Uint8Array; headers?: Record<string, string> };

/** A `fetch` that answers from a table by URL (404 otherwise) and records the requests it got. */
export function cannedFetch(pages: Record<string, Page | (() => Page)>) {
  const requests: { url: string; headers: Record<string, string> }[] = [];
  const fetcher = ((input: string | URL | Request, init?: RequestInit) => {
    const url = String(input);
    requests.push({ url, headers: { ...(init?.headers as Record<string, string>) } });
    const found = pages[url];
    const page = typeof found === "function" ? found() : found;
    if (page === undefined) return Promise.resolve(new Response("not found", { status: 404 }));
    if (typeof page === "string" || page instanceof Uint8Array) {
      return Promise.resolve(new Response(page as BodyInit, { headers: { "content-type": "text/html; charset=utf-8" } }));
    }
    const status = page.status ?? 200;
    return Promise.resolve(
      new Response(status === 304 || status === 204 ? null : (page.body ?? "") as BodyInit, {
        status,
        headers: page.headers,
      }),
    );
  }) as typeof fetch;
  return { fetcher, requests };
}

/** A directory with the given files; the caller removes it. */
export async function tempDir(files: Record<string, string | Uint8Array> = {}): Promise<string> {
  const dir = await Deno.makeTempDir({ prefix: "site-test-" });
  for (const [name, content] of Object.entries(files)) {
    const path = `${dir}/${name}`;
    await Deno.mkdir(path.slice(0, path.lastIndexOf("/")), { recursive: true });
    if (typeof content === "string") await Deno.writeTextFile(path, content);
    else await Deno.writeFile(path, content);
  }
  return dir;
}

/** A 1×1 PNG with the given colour byte, so that two calls can give different images. */
export function tinyPng(seed = 0): Uint8Array {
  const base = Uint8Array.from(atob(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==",
  ), (ch) => ch.charCodeAt(0));
  return seed ? Uint8Array.from([...base, seed]) : base;
}
