import { serveDir } from "jsr:@std/http/file-server";

const port = parseInt(Deno.env.get("PORT") ?? "3000");
const staticRoot = new URL("./dist", import.meta.url).pathname;

console.log(`[sys_test] serving ${staticRoot} on http://0.0.0.0:${port}`);

Deno.serve({ port, hostname: "0.0.0.0" }, (req: Request) => {
  return serveDir(req, {
    fsRoot: staticRoot,
    quiet: true,
  });
});
