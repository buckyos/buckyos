import { serveDir } from "jsr:@std/http/file-server";

async function serveWebFile(
  req: Request,
  staticRoot: string,
): Promise<Response> {
  const options = { fsRoot: staticRoot, quiet: true, showIndex: true };
  let response = await serveDir(req, options);
  if (response.headers.get("content-type")?.startsWith("text/html")) {
    if (response.status === 304) {
      const uncachedRequest = new Request(req);
      uncachedRequest.headers.delete("if-none-match");
      uncachedRequest.headers.delete("if-modified-since");
      response = await serveDir(uncachedRequest, options);
    }
    response.headers.set("cache-control", "no-store");
    response.headers.delete("etag");
    response.headers.delete("last-modified");
  }
  return response;
}

export async function serveSystestStatic(
  req: Request,
  staticRoot: string,
): Promise<Response> {
  const response = await serveWebFile(req, staticRoot);
  if (
    response.status === 404 &&
    req.headers.get("accept")?.includes("text/html")
  ) {
    const fallback = await serveWebFile(
      new Request(new URL("/index.html", req.url), req),
      staticRoot,
    );
    if (fallback.status === 200) {
      await response.body?.cancel();
      return fallback;
    }
    await fallback.body?.cancel();
  }
  return response;
}
