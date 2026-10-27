import { serveDir } from "jsr:@std/http/file-server";
import { serveSystestStatic } from "./static_web.ts";

const assertModule = "node:assert/strict";
const { default: assert } = await import(assertModule);

Deno.test("static HTML upgrades ignore colliding package metadata validators", async (t) => {
  const root = await Deno.makeTempDir({ prefix: "systest-static-" });
  const index = `${root}/index.html`;
  const oldHtml = '<script src="/assets/old.js"></script>';
  const newHtml = '<script src="/assets/new.js"></script>';
  try {
    await Deno.writeTextFile(index, oldHtml);
    await Deno.utime(index, 0, 0);
    const previous = await serveDir(new Request("http://localhost/"), {
      fsRoot: root,
      quiet: true,
    });
    const etag = previous.headers.get("etag")!;
    const modified = previous.headers.get("last-modified")!;
    assert.ok(etag);
    assert.equal(modified, "Thu, 01 Jan 1970 00:00:00 GMT");
    assert.equal(await previous.text(), oldHtml);
    assert.equal(oldHtml.length, newHtml.length);
    await Deno.writeTextFile(index, newHtml);
    await Deno.utime(index, 0, 0);

    const collision = await serveDir(
      new Request("http://localhost/", {
        headers: { "if-none-match": etag },
      }),
      { fsRoot: root, quiet: true },
    );
    assert.equal(collision.status, 304);
    assert.equal(collision.headers.get("etag"), etag);
    await collision.body?.cancel();

    const validators: Record<string, string>[] = [
      {},
      { "if-none-match": etag },
      { "if-modified-since": modified },
      { "if-none-match": etag, "if-modified-since": modified },
    ];
    for (const path of ["/", "/index.html", "/nested/route?view=1"]) {
      for (const method of ["GET", "HEAD"]) {
        for (const headers of validators) {
          await t.step(
            `${method} ${path} ${Object.keys(headers).join(",") || "uncached"}`,
            async () => {
              const response = await serveSystestStatic(
                new Request(`http://localhost${path}`, {
                  method,
                  headers: { accept: "text/html", ...headers },
                }),
                root,
              );
              assert.equal(response.status, 200);
              assert.equal(response.headers.get("cache-control"), "no-store");
              assert.equal(response.headers.get("etag"), null);
              assert.equal(response.headers.get("last-modified"), null);
              assert.equal(
                response.headers.get("content-length"),
                String(newHtml.length),
              );
              assert.equal(
                await response.text(),
                method === "HEAD" ? "" : newHtml,
              );
            },
          );
        }
      }
    }
  } finally {
    await Deno.remove(root, { recursive: true });
  }
});

Deno.test("static assets retain validators and missing files and methods retain their status", async (t) => {
  const root = await Deno.makeTempDir({ prefix: "systest-static-" });
  try {
    await Deno.mkdir(`${root}/assets`);
    for (
      const [name, content] of [
        ["index-a1b2c3.js", "export const version = 1;"],
        ["index-d4e5f6.css", "body { color: red; }"],
      ]
    ) {
      await Deno.writeTextFile(`${root}/assets/${name}`, content);
      await Deno.utime(`${root}/assets/${name}`, 0, 0);
      await t.step(name, async () => {
        const url = `http://localhost/assets/${name}`;
        const response = await serveSystestStatic(new Request(url), root);
        assert.equal(response.status, 200);
        assert.equal(response.headers.get("cache-control"), null);
        const etag = response.headers.get("etag")!;
        assert.ok(etag);
        assert.equal(
          response.headers.get("last-modified"),
          "Thu, 01 Jan 1970 00:00:00 GMT",
        );
        assert.equal(await response.text(), content);
        const cached = await serveSystestStatic(
          new Request(url, {
            headers: { "if-none-match": etag },
          }),
          root,
        );
        assert.equal(cached.status, 304);
        assert.equal(await cached.text(), "");
        const head = await serveSystestStatic(
          new Request(url, { method: "HEAD" }),
          root,
        );
        assert.equal(head.status, 200);
        assert.equal(
          head.headers.get("content-length"),
          String(content.length),
        );
        assert.equal(await head.text(), "");
      });
    }
    for (const method of ["GET", "HEAD", "POST"]) {
      for (const accept of ["*/*", "text/html"]) {
        await t.step(`${method} missing ${accept}`, async () => {
          const response = await serveSystestStatic(
            new Request("http://localhost/missing", {
              method,
              headers: { accept },
            }),
            root,
          );
          assert.equal(response.status, method === "POST" ? 405 : 404);
          await response.body?.cancel();
        });
      }
    }
    await Deno.writeTextFile(`${root}/index.html`, "<main>index</main>");
    await t.step(
      "missing asset does not become HTML without a navigation Accept",
      async () => {
        const response = await serveSystestStatic(
          new Request("http://localhost/assets/missing.js"),
          root,
        );
        assert.equal(response.status, 404);
        await response.body?.cancel();
      },
    );
    await t.step("POST to the index remains unsupported", async () => {
      const response = await serveSystestStatic(
        new Request("http://localhost/", {
          method: "POST",
          headers: { accept: "text/html" },
        }),
        root,
      );
      assert.equal(response.status, 405);
      await response.body?.cancel();
    });
  } finally {
    await Deno.remove(root, { recursive: true });
  }
});
