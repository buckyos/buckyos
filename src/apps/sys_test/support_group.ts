/**
 * sys_test official example: a customer-service group with a customer link
 * (buckyos notepads/self-host-group-extension-todo.md §11, EXT-10).
 *
 * The page (the installing account's session on this App's origin) creates the
 * group itself; this module is the App side that needs the App service identity:
 *
 *   GET  /support/panel                                 the group's custom page (L2 panel)
 *   GET  /sdk/appservice/support/summary?group_did=&session_id=
 *                                                        what the panel shows: counts, no text
 *   POST /sdk/appservice/support/customer_link          a 48-hour customer link
 *                                                        (`user.app_login.issue {kind:"link"}`)
 *
 * `customer_link` is for the App instance owner only: the page sends its own session
 * as `Authorization: Bearer`, and the backend asks Control Panel who that is before
 * issuing anything with the App identity. Links, secrets and external ids never go
 * to the log.
 */

type RpcClient = { call(method: string, params: Record<string, unknown>): Promise<unknown> };

export interface SupportContext {
  ownerUserId: string;
  appInstanceId: string;
  zoneHost: string;
  /** Control Panel / msg-center kRPC clients holding the App service session. */
  serviceClient(service: "control-panel" | "msg-center"): RpcClient;
  /** A Control Panel kRPC client holding the caller's session (to learn who they are). */
  clientWithToken(service: "control-panel", token: string): RpcClient;
}

type JsonRecord = Record<string, unknown>;

function json(payload: unknown, status = 200): Response {
  return new Response(JSON.stringify(payload, null, 2), {
    status,
    headers: { "content-type": "application/json; charset=utf-8" },
  });
}

function isRecord(value: unknown): value is JsonRecord {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** A failed service call as the page sees it: the message plus the kRPC `code` / `data` (the denial), never a guess from text. */
function failure(error: unknown, status = 502): Response {
  const record = isRecord(error) ? error : {};
  return json({
    ok: false,
    error: error instanceof Error ? error.message : String(error),
    ...(typeof record.code === "string" ? { code: record.code } : {}),
    ...(record.data !== undefined ? { data: record.data } : {}),
  }, status);
}

const panelHtml = `<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<title>客服工单</title>
<style>
  :root { color-scheme: light dark; font: 14px/1.5 system-ui, sans-serif; }
  body { margin: 0; padding: 12px; }
  h1 { font-size: 15px; margin: 0 0 8px; }
  dl { display: grid; grid-template-columns: max-content 1fr; gap: 2px 10px; margin: 0 0 12px; }
  dd { margin: 0; word-break: break-all; }
  .muted { opacity: .7; }
  .warn { padding: 8px 10px; border-radius: 8px; background: rgba(255,170,0,.15); }
  ul { list-style: none; padding: 0; margin: 0; }
  li { padding: 6px 0; border-top: 1px solid rgba(127,127,127,.25); }
</style>
</head>
<body>
<h1>客服工单</h1>
<p class="muted">sys-test 官方例子的群自定义页面。它只拿到群和 Session 的地址（D-1），内容由 sys-test 后台以应用身份汇总，不含消息正文。</p>
<dl id="summary"><dt>状态</dt><dd id="status">加载中…</dd></dl>
<ul id="recent"></ul>
<script type="module">
const q = new URLSearchParams(location.search)
const groupDid = q.get('group_did') ?? ''
const sessionId = q.get('session_id') ?? ''
const status = document.getElementById('status')
const dl = document.getElementById('summary')
const row = (k, v) => { const dt = document.createElement('dt'); dt.textContent = k; const dd = document.createElement('dd'); dd.textContent = v; dl.append(dt, dd) }
try {
  const res = await fetch('/sdk/appservice/support/summary?' + new URLSearchParams({ group_did: groupDid, session_id: sessionId }))
  const data = await res.json()
  if (!res.ok || !data.ok) throw new Error(data.error ?? ('HTTP ' + res.status))
  status.textContent = '正常'
  row('群', groupDid)
  row('Session', sessionId || '（默认）')
  row('消息数', String(data.count))
  row('参与者', String(data.participants))
  row('最近一条', data.last_at ? new Date(data.last_at).toLocaleString() : '—')
  const ul = document.getElementById('recent')
  for (const item of data.recent) { const li = document.createElement('li'); li.textContent = '#' + item.seq + ' ' + item.from + ' · ' + new Date(item.at).toLocaleTimeString(); ul.append(li) }
  if (!data.recent.length) { const li = document.createElement('li'); li.className = 'muted'; li.textContent = '这个 Session 还没有消息。'; ul.append(li) }
} catch (error) {
  status.className = 'warn'
  status.textContent = '面板内容不可用：' + (error instanceof Error ? error.message : String(error))
}
</script>
</body>
</html>`;

function messageSummary(page: unknown) {
  const items = (isRecord(page) && Array.isArray(page.items) ? page.items : []) as JsonRecord[];
  const senders = new Set<string>();
  let lastAt = 0;
  const recent = items.map((item) => {
    const msg = isRecord(item.msg) ? item.msg : {};
    const from = String(msg.from ?? item.from ?? "");
    if (from) senders.add(from);
    const at = Number(item.accepted_at_ms ?? 0);
    lastAt = Math.max(lastAt, at);
    return { seq: Number(item.seq ?? 0), from, at };
  });
  recent.sort((a, b) => b.seq - a.seq);
  return { count: items.length, participants: senders.size, last_at: lastAt || null, recent: recent.slice(0, 10) };
}

export async function handleSupportRoute(
  req: Request,
  url: URL,
  prefix: string,
  context: SupportContext | null,
): Promise<Response | null> {
  if (req.method === "GET" && url.pathname === "/support/panel") {
    return new Response(panelHtml, { headers: { "content-type": "text/html; charset=utf-8" } });
  }
  if (!url.pathname.startsWith(`${prefix}/support/`)) return null;
  if (!context) return json({ ok: false, error: "AppService not initialized" }, 503);

  if (req.method === "GET" && url.pathname === `${prefix}/support/summary`) {
    const groupDid = url.searchParams.get("group_did")?.trim() ?? "";
    const sessionId = url.searchParams.get("session_id")?.trim() ?? "";
    if (!groupDid) return json({ ok: false, error: "group_did is required" }, 400);
    try {
      const page = await context.serviceClient("msg-center").call("group.list_messages", {
        group_did: groupDid,
        session_id: sessionId || null,
        limit: 200,
        with_object: true,
      });
      return json({ ok: true, ...messageSummary(page) });
    } catch (error) {
      return failure(error);
    }
  }

  if (req.method === "POST" && url.pathname === `${prefix}/support/customer_link`) {
    const token = req.headers.get("authorization")?.replace(/^Bearer\s+/i, "").trim();
    if (!token) return json({ ok: false, error: "the page session is required" }, 401);
    let body: JsonRecord;
    try {
      body = JSON.parse(await req.text()) as JsonRecord;
      if (!isRecord(body)) throw new Error("not an object");
    } catch {
      return json({ ok: false, error: "request body must be a JSON object" }, 400);
    }
    const groupDid = String(body.group_did ?? "").trim();
    const sessionId = String(body.session_id ?? "").trim();
    const externalAccountId = String(body.external_account_id ?? "").trim();
    const showName = String(body.show_name ?? "").trim();
    if (!groupDid || !sessionId || !externalAccountId) {
      return json({ ok: false, error: "group_did, session_id and external_account_id are required" }, 400);
    }
    let caller: unknown;
    try {
      caller = await context.clientWithToken("control-panel", token).call("auth.current_identity", {});
    } catch (error) {
      return failure(error, 401);
    }
    if (!isRecord(caller) || caller.user_id !== context.ownerUserId) {
      return json({ ok: false, error: "only the installing account of this App issues customer links" }, 403);
    }
    const returnPath = `/messagehub/guest?${new URLSearchParams({ group: groupDid, session: sessionId })}`;
    try {
      const issued = await context.serviceClient("control-panel").call("user.app_login.issue", {
        kind: "link",
        external_account_id: externalAccountId,
        ...(showName ? { show_name: showName } : {}),
        resource: { kind: "group_session", group_did: groupDid, session_id: sessionId, title: String(body.title ?? "客服工单") },
        return_path: returnPath,
      });
      if (!isRecord(issued) || typeof issued.link !== "string") throw new Error("invalid user.app_login.issue response");
      console.log(`[sys_test] customer link issued for account '${issued.user_id}' (new: ${issued.new_account === true})`);
      return json({
        ok: true,
        link: issued.link,
        user_id: issued.user_id,
        expires_at: issued.expires_at,
        new_account: issued.new_account === true,
        return_path: returnPath,
        invite: isRecord(issued.invite) ? { invite_id: issued.invite.invite_id, resource_state: issued.invite.resource_state } : null,
      });
    } catch (error) {
      return failure(error);
    }
  }
  return json({ ok: false, error: `no such route: ${url.pathname}` }, 404);
}
