/**
 * L4A: the App's own backend and UI for external customers (a mock CRM).
 *
 *   GET /login/<external_account_id>   issue a one-time code with the App service identity
 *                                      (CP `user.app_login.issue`) and render the exchange page
 *   GET /tickets/<session_id>          the customer's ticket page: lists the Session with the
 *                                      customer's own restricted session (cookie on this origin)
 *   GET /panel/*                       the L2 panel static files
 *
 * Install this as the App `<BUCKYOS_TEST_APP_ID>` of the group owner so the gateway serves
 * it on `<app>.<zone>`; the code exchange and the customer session only work on that origin.
 *
 * Env: BUCKYOS_APP_TOKEN (App service session token), BUCKYOS_APP_INSTANCE (app_id@owner),
 * BUCKYOS_APP_PORT (8788), BUCKYOS_TEST_ZONE_HOST, and `state.json` from setup_group.ts.
 */
import { asRecord, call, zoneHost } from "./common.ts";

const appToken = Deno.env.get("BUCKYOS_APP_TOKEN")?.trim() ?? "";
const port = Number(Deno.env.get("BUCKYOS_APP_PORT") ?? 8788);
const state = JSON.parse(await Deno.readTextFile(new URL("./state.json", import.meta.url))) as { group_did: string };
const panelDir = new URL("./panel/", import.meta.url);

function html(body: string, status = 200): Response {
  return new Response(`<!doctype html><meta charset="utf-8"><body style="font:14px system-ui;padding:16px">${body}</body>`, {
    status,
    headers: { "content-type": "text/html; charset=utf-8" },
  });
}

async function issueCode(externalAccountId: string, sessionId: string) {
  if (!appToken) throw new Error("BUCKYOS_APP_TOKEN is required to issue login codes");
  return asRecord(
    await call("control-panel", "user.app_login.issue", {
      external_account_id: externalAccountId,
      show_name: `客户 ${externalAccountId}`,
      resource: { kind: "group_session", group_did: state.group_did, session_id: sessionId, title: `工单 ${sessionId}` },
      return_path: `/tickets/${encodeURIComponent(sessionId)}`,
    }, appToken),
    "user.app_login.issue",
  );
}

// The exchange page: the code arrives in the fragment (never in a request line), a nonce
// is kept per tab so a reload retries the same exchange, then the browser follows the
// callback which sets this origin's cookies and returns to `return_path`.
const exchangePage = `
<h1>正在登录…</h1><p id="out"></p>
<script type="module">
const code = new URLSearchParams(location.hash.slice(1)).get('code')
history.replaceState(null, '', location.pathname)
const nonce = sessionStorage.appLoginNonce ??= crypto.randomUUID()
const res = await fetch('/kapi/control-panel', { method: 'POST', headers: { 'content-type': 'application/json' },
  body: JSON.stringify({ method: 'auth.app_login.exchange', params: { code, nonce }, sys: [Date.now()] }) })
const json = await res.json()
const result = json.result ?? {}
if (result.status === 'ready') location.assign(result.callback_url)
else document.getElementById('out').textContent = '登录失败：' + (result.denial?.reason ?? json.error?.message ?? 'unknown') + (result.denial?.detail ? ' / ' + result.denial.detail : '')
</script>`;

const ticketPage = (sessionId: string) => `
<h1>工单 ${sessionId}</h1><ul id="list"></ul>
<form id="f"><input name="text" placeholder="回复…" style="width:60%"><button>发送</button></form>
<script type="module">
const groupDid = ${JSON.stringify(state.group_did)}, sid = ${JSON.stringify(sessionId)}
const rpc = async (m, p) => { const r = await fetch('/kapi/msg-center', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ method: m, params: p, sys: [Date.now()] }) }); const j = await r.json(); if (j.error) throw new Error(j.error.message); return j.result }
const me = await (await fetch('/kapi/control-panel', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ method: 'auth.current_identity', params: {}, sys: [Date.now()] }) })).json()
async function refresh() {
  const page = await rpc('group.list_messages', { group_did: groupDid, session_id: sid, limit: 50, with_object: true })
  document.getElementById('list').innerHTML = (page.items ?? []).map(i => '<li>' + (i.msg?.from ?? '') + ': ' + (i.msg?.content?.content ?? '') + '</li>').join('')
}
document.getElementById('f').onsubmit = async (e) => {
  e.preventDefault(); const text = new FormData(e.target).get('text')
  await rpc('msg.post_send', { idempotency_key: crypto.randomUUID(), msg: { from: me.result.did, to: [groupDid], kind: 'group_msg', to_session: sid, created_at_ms: Date.now(), nonce: Date.now() % 1e9, content: { format: 'text/plain', content: text } } })
  e.target.reset(); await refresh()
}
await refresh()
</script>`;

Deno.serve({ port }, async (req) => {
  const url = new URL(req.url);
  if (url.pathname.startsWith("/login/")) {
    const external = decodeURIComponent(url.pathname.slice("/login/".length));
    const sessionId = url.searchParams.get("session") ?? "tickets";
    try {
      const issued = await issueCode(external, sessionId);
      // In a real CRM the link would be sent to the customer; here we redirect straight to it.
      return Response.redirect(`${url.origin}/exchange#code=${encodeURIComponent(String(issued.code))}`, 302);
    } catch (error) {
      return html(`<p>签发失败：${error instanceof Error ? error.message : String(error)}</p>`, 500);
    }
  }
  if (url.pathname === "/exchange") return html(exchangePage);
  if (url.pathname.startsWith("/tickets/")) return html(ticketPage(decodeURIComponent(url.pathname.slice("/tickets/".length))));
  if (url.pathname.startsWith("/panel")) {
    const file = url.pathname === "/panel" || url.pathname === "/panel/" ? "index.html" : url.pathname.slice("/panel/".length);
    try {
      const body = await Deno.readFile(new URL(file, panelDir));
      return new Response(body, { headers: { "content-type": file.endsWith(".html") ? "text/html; charset=utf-8" : "application/octet-stream" } });
    } catch {
      return new Response("not found", { status: 404 });
    }
  }
  return html(`<p>客服群样例 App（zone ${zoneHost}）。入口：<code>/login/&lt;external_id&gt;</code>、<code>/tickets/&lt;sid&gt;</code>、<code>/panel/</code></p>`);
});
