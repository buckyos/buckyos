import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { pathToFileURL } from "node:url";

function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), "systest-package-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const write = (path, content) => {
    const target = join(root, path);
    mkdirSync(dirname(target), { recursive: true });
    writeFileSync(target, content);
  };
  copyFileSync(
    new URL("./prepare.mjs", import.meta.url),
    join(root, "prepare.mjs"),
  );
  for (
    const name of ["static_web.ts", "xllm_selftest.ts", "support_group.ts","resource_share.ts"]
  ) {
    write(name, "export {};\n");
  }
  write(
    "main.ts",
    'type SDK = typeof import("@sys-test/websdk-node-types");\nconst sdk: SDK = await import("./node_modules/buckyos/dist/node.mjs");\nif (sdk.value !== "runtime" || (await sdk.load()).value !== "runtime") throw new Error("invalid runtime");\n',
  );
  write(
    "deno.json",
    JSON.stringify({
      imports: {
        "@sys-test/websdk-node-types": "./node_modules/buckyos/dist/node.d.ts",
      },
    }),
  );
  write("web/dist/index.html", "<html></html>");
  write("web/dist/assets/app.js", "export {};\n");
  write("web/dist/assets/app.js.map", "{}");
  write(
    "node_modules/buckyos/package.json",
    '{"name":"buckyos","type":"module"}',
  );
  write(
    "node_modules/buckyos/dist/node.mjs",
    'export { value } from "./chunks/shared.mjs";\nexport const load = () => import("./chunks/lazy.mjs");\n',
  );
  write(
    "node_modules/buckyos/dist/chunks/shared.mjs",
    'import { basename } from "node:path";\nexport const value = basename("/test/runtime");\n',
  );
  write(
    "node_modules/buckyos/dist/chunks/lazy.mjs",
    'export { value } from "./shared.mjs";\n',
  );
  for (
    const name of [
      "node.mjs.map",
      "node.d.ts",
      "browser.mjs",
      "node.cjs",
      "unused.mjs",
    ]
  ) {
    write(`node_modules/buckyos/dist/${name}`, "");
  }
  return {
    root,
    write,
    prepare: () =>
      execFileSync(process.execPath, [join(root, "prepare.mjs")], {
        encoding: "utf8",
        stdio: "pipe",
      }),
  };
}

test("package keeps static and lazy SDK dependencies and excludes development artifacts", async (t) => {
  const { root, prepare } = fixture(t);
  prepare();
  const dist = join(root, "dist");
  assert.equal(existsSync(join(dist, "web/assets/app.js")), true);
  assert.equal(existsSync(join(dist, "web/assets/app.js.map")), false);
  assert.deepEqual(
    readdirSync(join(dist, "node_modules/buckyos/dist")).sort(),
    ["chunks", "node.mjs"],
  );
  const entry =
    pathToFileURL(join(dist, "node_modules/buckyos/dist/node.mjs")).href;
  const sdk = await import(entry);
  assert.equal(sdk.value, "runtime");
  assert.equal((await sdk.load()).value, "runtime");
  const config = JSON.parse(readFileSync(join(dist, "deno.json"), "utf8"));
  assert.equal(
    existsSync(join(dist, config.imports["@sys-test/websdk-node-types"])),
    true,
  );
  execFileSync("deno", [
    "run",
    "--no-lock",
    "--no-npm",
    "--no-remote",
    "main.ts",
  ], { cwd: dist, stdio: "pipe" });
});

test("missing transitive SDK dependencies fail before replacing the package", (t) => {
  const { root, write, prepare } = fixture(t);
  write("dist/previous-build", "keep");
  rmSync(join(root, "node_modules/buckyos/dist/chunks/lazy.mjs"));
  assert.throws(prepare, /invalid SDK runtime/);
  assert.equal(readFileSync(join(root, "dist/previous-build"), "utf8"), "keep");
});
