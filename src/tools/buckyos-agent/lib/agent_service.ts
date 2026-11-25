// kRPC to this Agent's own Agent Service ($OPENDAN_BROWSER_RPC, token
// $BUCKYOS_APPCLIENT_SESSION_TOKEN): a source package's `ctx.browser` uses it.

import http from "node:http";
import https from "node:https";

/** The endpoint and token a session on the Agent's own runtime has; `null` elsewhere. */
export function agentServiceAccess(): { endpoint: string; token: string } | null {
  const endpoint = Deno.env.get("OPENDAN_BROWSER_RPC");
  const token = Deno.env.get("BUCKYOS_APPCLIENT_SESSION_TOKEN");
  return endpoint && token ? { endpoint, token } : null;
}

// The Agent Service's errors travel as JSON inside the kRPC error text: the
// browser module's carry a stable `code`, the others a `kind` (Agent Browser
// Protocol §2.3, §2.5).
export function serviceError(err: unknown): { code?: string; message: string } {
  const text = err instanceof Error ? err.message : String(err);
  const own = (err as { code?: unknown } | null)?.code;
  const start = text.indexOf("{");
  if (start >= 0) {
    try {
      const wire = JSON.parse(text.slice(start));
      if (wire && typeof wire === "object") {
        return {
          code: [wire.code, wire.kind, own].find((code) => typeof code === "string"),
          message: typeof wire.message === "string" ? wire.message : text,
        };
      }
    } catch {
      // not JSON: the text is the message
    }
  }
  return { code: typeof own === "string" ? own : undefined, message: text };
}

// One kRPC call. Not `fetch` (the SDK's client uses it): it refuses the ports of its
// "bad port" list, and an Agent Service can be given one (10080).
export function callAgentService<T>(
  endpoint: string,
  token: string,
  method: string,
  params: unknown,
  timeoutMs = 60_000,
): Promise<T> {
  return new Promise((resolve, reject) => {
    const target = new URL(endpoint);
    const text = JSON.stringify({ method, params, sys: [Date.now(), token] });
    const request = (target.protocol === "https:" ? https : http).request(
      target,
      {
        method: "POST",
        headers: { "content-type": "application/json", "content-length": new TextEncoder().encode(text).length },
      },
      (response) => {
        let answer = "";
        response.setEncoding("utf8");
        response.on("data", (chunk: string) => answer += chunk);
        response.on("error", reject);
        response.on("end", () => {
          if ((response.statusCode ?? 0) < 200 || (response.statusCode ?? 0) >= 300) {
            reject(new Error(`the Agent Service answered HTTP ${response.statusCode}`));
            return;
          }
          let body: { result?: T; error?: unknown };
          try {
            body = JSON.parse(answer);
          } catch {
            reject(new Error("the Agent Service sent an unreadable answer"));
            return;
          }
          if (body.error !== undefined && body.error !== null) reject(new Error(String(body.error)));
          else resolve(body.result as T);
        });
      },
    );
    request.setTimeout(
      timeoutMs,
      () => request.destroy(new Error(`the Agent Service did not answer in ${Math.round(timeoutMs / 1000)}s`)),
    );
    request.on("error", (err: Error) => reject(new Error(`cannot reach the Agent Service at ${endpoint}: ${err.message}`)));
    request.end(text);
  });
}
