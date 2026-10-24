/**
 * L3: the business Hook service. Verifies every request with the binding's secret
 * (no BuckyOS runtime involved), then decides: `join` passes an allowlist, `post`
 * refuses messages containing `#deny`, everything else is allowed.
 *
 *   deno run --allow-net --allow-env --allow-read hook_server.ts
 *
 * Env: BUCKYOS_HOOK_SECRET (else `state.json`'s `hook_secret`), BUCKYOS_HOOK_PORT (8787),
 * BUCKYOS_HOOK_ALLOW (comma separated DIDs allowed to join; empty = everyone).
 */
import { verifyHookRequest } from "./common.ts";

const secret = Deno.env.get("BUCKYOS_HOOK_SECRET")?.trim() ||
  JSON.parse(await Deno.readTextFile(new URL("./state.json", import.meta.url))).hook_secret;
const port = Number(Deno.env.get("BUCKYOS_HOOK_PORT") ?? 8787);
const allow = new Set((Deno.env.get("BUCKYOS_HOOK_ALLOW") ?? "").split(",").map((s) => s.trim()).filter(Boolean));

export function decide(body: Record<string, unknown>): "allow" | "deny" {
  if (body.operation === "join" && allow.size > 0 && !allow.has(String(body.actor))) return "deny";
  if (body.operation === "post") {
    const text = String(((body.msg as Record<string, unknown>)?.content as Record<string, unknown>)?.content ?? "");
    if (text.includes("#deny")) return "deny";
  }
  return "allow";
}

Deno.serve({ port }, async (req) => {
  if (req.method !== "POST") return new Response("hook", { status: 405 });
  const { ok, body } = await verifyHookRequest(secret, req);
  if (!ok) return new Response("{}", { status: 401, headers: { "content-type": "application/json" } });
  const parsed = JSON.parse(body) as Record<string, unknown>;
  const decision = decide(parsed);
  console.log(`${parsed.operation} ${parsed.actor} @ ${parsed.group_did}/${parsed.session_id ?? ""} from ${parsed.group_host} → ${decision}`);
  return Response.json({ decision });
});
