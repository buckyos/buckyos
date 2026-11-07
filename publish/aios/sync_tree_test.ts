import { syncTree } from "./sync_tree.ts";

function assertEquals(actual: unknown, expected: unknown) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) throw new Error(`${JSON.stringify(actual)} != ${JSON.stringify(expected)}`);
}

Deno.test("copies new files, follows upstream, keeps what changed locally, deletes nothing", () => {
  const root = Deno.makeTempDirSync();
  const [src, dst, meta, log] = [`${root}/src`, `${root}/dst`, `${root}/m/meta.json`, `${root}/m/log.json`];
  Deno.mkdirSync(`${src}/sub`, { recursive: true });
  Deno.writeTextFileSync(`${src}/a`, "1");
  Deno.writeTextFileSync(`${src}/sub/b`, "1");
  Deno.writeTextFileSync(`${src}/gone`, "1");
  assertEquals(syncTree(src, dst, meta, log), { copied: 3, updated: 0, kept_local: 0, removed_from_upstream: 0 });
  assertEquals(Deno.readTextFileSync(`${dst}/sub/b`), "1");

  assertEquals(syncTree(src, dst, meta, log), { copied: 0, updated: 0, kept_local: 0, removed_from_upstream: 0 });

  Deno.writeTextFileSync(`${src}/a`, "2");
  Deno.writeTextFileSync(`${src}/sub/b`, "2");
  Deno.writeTextFileSync(`${dst}/sub/b`, "mine");
  Deno.removeSync(`${src}/gone`);
  assertEquals(syncTree(src, dst, meta, log), { copied: 0, updated: 1, kept_local: 1, removed_from_upstream: 1 });
  assertEquals(Deno.readTextFileSync(`${dst}/a`), "2");
  assertEquals(Deno.readTextFileSync(`${dst}/sub/b`), "mine");
  assertEquals(Deno.readTextFileSync(`${dst}/gone`), "1");
  const events = JSON.parse(Deno.readTextFileSync(log)).events.map((e: { path: string; reason: string }) => [e.path, e.reason]);
  assertEquals(events, [["sub/b", "local_divergence_kept"], ["gone", "upstream_removed_local_kept"]]);

  // There before the first sync: locally owned.
  const fresh = `${root}/fresh`;
  Deno.mkdirSync(fresh);
  Deno.writeTextFileSync(`${fresh}/a`, "other");
  assertEquals(syncTree(src, fresh, `${root}/m2/meta.json`, `${root}/m2/log.json`), { copied: 1, updated: 0, kept_local: 1, removed_from_upstream: 0 });
  assertEquals(Deno.readTextFileSync(`${fresh}/a`), "other");
  Deno.removeSync(root, { recursive: true });
});
