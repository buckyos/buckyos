// A site kept in a local directory, laid out like the bucket: what `site try`
// writes and `site preview --data` shows. Object ids are stand-ins made of the
// content's SHA-256; the real ones come from the bucket.

import type { SiteStore } from "./store.ts";

async function sha256Hex(bytes: Uint8Array): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", bytes as BufferSource);
  return Array.from(new Uint8Array(digest), (b) => b.toString(16).padStart(2, "0")).join("");
}

const coded = (code: string, message: string) => Object.assign(new Error(message), { code });

export function dirStore(root: string): SiteStore {
  const read = async (key: string): Promise<Uint8Array> => {
    try {
      return await Deno.readFile(`${root}/${key}`);
    } catch (err) {
      if (err instanceof Deno.errors.NotFound) throw coded("no_such_key", `no such key: ${key}`);
      throw err;
    }
  };
  const walk = async (dir: string, prefix: string, keys: string[]) => {
    try {
      for await (const entry of Deno.readDir(dir)) {
        if (entry.isDirectory) await walk(`${dir}/${entry.name}`, `${prefix}${entry.name}/`, keys);
        else keys.push(`${prefix}${entry.name}`);
      }
    } catch (err) {
      if (!(err instanceof Deno.errors.NotFound)) throw err;
    }
  };
  return {
    async listObjects(_bucketId, req) {
      const keys: string[] = [];
      await walk(root, "", keys);
      const prefix = req.prefix ?? "";
      const items: { key: string }[] = [];
      const folders = new Set<string>();
      for (const key of keys.sort()) {
        if (!key.startsWith(prefix)) continue;
        const cut = req.delimiter ? key.indexOf(req.delimiter, prefix.length) : -1;
        if (cut >= 0) folders.add(key.slice(0, cut + 1));
        else items.push({ key });
      }
      return { items, common_prefixes: [...folders] };
    },
    async fetchObject(_bucketId, key) {
      const bytes = await read(key);
      return new Response(bytes as BodyInit, { headers: { etag: `"${await sha256Hex(bytes)}"` } });
    },
    async putBytes(_bucketId, key, body, options) {
      const path = `${root}/${key}`;
      const bytes = typeof body === "string" ? new TextEncoder().encode(body) : body;
      if (options?.if_none_match === "*") {
        const exists = await Deno.stat(path).then(() => true, () => false);
        if (exists) throw coded("precondition_failed", "the key already exists");
      }
      await Deno.mkdir(path.slice(0, path.lastIndexOf("/")), { recursive: true });
      await Deno.writeFile(path, bytes);
      return { obj_id: `cyfile:${await sha256Hex(bytes)}` };
    },
    put: () => Promise.resolve({}),
    async get(_bucketId, key) {
      return { obj_id: `cyfile:${await sha256Hex(await read(key))}` };
    },
    async delete(_bucketId, key) {
      await Deno.remove(`${root}/${key}`).catch(() => {});
      return {};
    },
  };
}
