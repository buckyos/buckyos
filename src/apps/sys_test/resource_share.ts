import { DatabaseSync } from "node:sqlite";
import { join } from "node:path";
import { timingSafeEqual } from "node:crypto";
import type { SupportContext } from "./support_group.ts";

const databases = new Map<string, DatabaseSync>();
function database(instance: string): DatabaseSync {
  let db = databases.get(instance);
  if (db) return db;
  const root = Deno.env.get("SCRIPT_DATA_ROOT");
  if (!root) throw new Error("App data directory unavailable");
  db = new DatabaseSync(join(root, "resource-shares.sqlite"));
  db.exec(`PRAGMA journal_mode=WAL;
    CREATE TABLE IF NOT EXISTS documents(instance TEXT,id TEXT,title TEXT,body TEXT,PRIMARY KEY(instance,id));
    CREATE TABLE IF NOT EXISTS invitations(instance TEXT,id TEXT,document TEXT,operator TEXT,action TEXT,secret TEXT,closed INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(instance,id));
    CREATE TABLE IF NOT EXISTS grants(instance TEXT,invitation TEXT,did TEXT,active INTEGER NOT NULL,PRIMARY KEY(instance,invitation,did));`);
  databases.set(instance, db);
  return db;
}
const reply = (data: unknown, status = 200) =>
  Response.json(data, { status, headers: { "cache-control": "no-store" } });
const validId = (id: unknown): id is string =>
  typeof id === "string" && /^[a-zA-Z0-9_-]{1,96}$/.test(id);
function tokenOf(request: Request) {
  const bearer = request.headers.get("authorization")?.match(
    /^Bearer ([^ ]+)$/i,
  );
  const sessionCookie = "buckyos_session_token=";
  return bearer?.[1] ??
    request.headers.get("cookie")?.split(";").map((s) => s.trim()).find((s) =>
      s.startsWith(sessionCookie)
    )?.slice(sessionCookie.length);
}
async function actor(request: Request, context: SupportContext) {
  const token = tokenOf(request);
  if (!token) return null;
  const verified = await context.serviceClient("verify-hub").call(
    "verify_token",
    {
      session_token: token,
      expected_target: { kind: "app", app_instance_id: context.appInstanceId },
    },
  );
  if (verified !== true) return null;
  const claims = JSON.parse(
    atob(token.split(".")[1].replaceAll("-", "+").replaceAll("_", "/")),
  );
  if (claims.principal_kind !== "user" || typeof claims.sub !== "string") {
    return null;
  }
  return {
    owner: claims.sub === context.ownerUserId && claims.restricted !== true,
    did: typeof claims.resource_share_did === "string"
      ? claims.resource_share_did
      : claims.sub,
  };
}

const page =
  `<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Shared document</title>
<main><h1 id="title">Shared document</h1><textarea id="body" disabled rows="16" cols="70" aria-label="Document text"></textarea>
<button id="save" hidden disabled type="button">Save</button><form id="share" hidden><select id="audience" aria-label="Audience"><option value="directed">Specific recipient</option><option value="open">Open invitation</option></select>
<input id="recipient" aria-label="Recipient DID or Google email"><select id="action" aria-label="Permission"><option value="read">Read</option><option value="edit">Edit</option></select><button>Create invitation</button><output id="link"></output><button id="copy" hidden type="button">Copy link</button></form><p id="status" role="status"></p></main>
<script type="module">
const el=id=>document.getElementById(id)
const api=location.pathname.replace('/docs/','/sdk/appservice/documents/')
async function call(path,opts) {const r=await fetch(path,opts); if(!r.ok) throw new Error('Request failed ('+r.status+')'); return r.json()}
const post=(method,data)=>({method,headers:{'content-type':'application/json'},body:JSON.stringify(data)})
try {
  const doc=await call(api); el('title').textContent=doc.title; el('body').value=doc.body; el('body').disabled=false; el('save').disabled=!doc.can_edit; el('body').readOnly=!doc.can_edit; el('save').hidden=!doc.can_edit; el('share').hidden=!doc.owner;
  el('save').onclick=async()=>{try{await call(api,post('PUT',{body:el('body').value}));el('status').textContent='Saved'}catch(e){el('status').textContent=e.message}}
  let lastRequest = null;
  el('copy').onclick=async()=>{try {await navigator.clipboard.writeText(el('link').textContent);el('status').textContent='Link copied'}catch(e){el('status').textContent=e.message}};
  el('share').onsubmit=async(e)=>{e.preventDefault();try{const to=el('recipient').value.trim(),open=el('audience').value==='open',email=to.includes('@')&&!to.startsWith('did:');
    const request={action:el('action').value,audience:open?{kind:'open'}:email?{kind:'directed'}:{kind:'directed',did:to},...(!open&&email?{recipient_email:{source_id:'google',email:to}}:{}),return_path:location.pathname+location.search+location.hash};
    const key=JSON.stringify(request); if(lastRequest?.key!==key) lastRequest={key,id:crypto.randomUUID()};
    const r=await call(api+'/invitations',post('POST',{...request,request_id:lastRequest.id}));
    el('link').textContent=r.url; el('copy').hidden=false; lastRequest=null;
  }catch(e){el('status').textContent=e.message}}
}catch(e){el('status').textContent=e.message;el('save').hidden=true;el('body').hidden=true}
</script></html>`;

export async function handleResourceShare(
  req: Request,
  url: URL,
  context: SupportContext | null,
): Promise<Response | null> {
  const callback = url.pathname === "/sdk/appservice/resource-share/callback";
  const view = url.pathname.match(/^\/docs\/([a-zA-Z0-9_-]{1,96})$/);
  const api = url.pathname.match(
    /^\/sdk\/appservice\/documents\/([a-zA-Z0-9_-]{1,96})(\/invitations)?$/,
  );
  if (!callback && !view && !api) return null;
  if (!context) return reply({ ok: false }, 503);
  if (view && req.method === "GET") {
    return new Response(page, {
      headers: {
        "content-type": "text/html;charset=utf-8",
        "cache-control": "no-store",
      },
    });
  }
  const instance = context.appInstanceId;
  try {
    const db = database(instance);
    if (callback && req.method === "POST") {
      const q = await req.json();
      if (
        !validId(q.resource_id) || typeof q.callback_secret !== "string" ||
        q.callback_secret.length > 128
      ) return reply({ ok: false }, 403);
      const invitation = db.prepare(
        "SELECT i.*, d.title FROM invitations i JOIN documents d ON i.instance=d.instance AND d.id=i.document WHERE i.instance=? AND i.id=?",
      ).get(instance, q.resource_id);
      if (
        !invitation || invitation.operator !== context.ownerUserId ||
        typeof invitation.secret !== "string" ||
        invitation.secret.length !== q.callback_secret.length ||
        !timingSafeEqual(
          new TextEncoder().encode(invitation.secret),
          new TextEncoder().encode(q.callback_secret),
        )
      ) return reply({ ok: false }, 403);
      if (q.operation === "resume") {
        const row = db.prepare(
          "SELECT active FROM grants WHERE instance=? AND invitation=? AND did=?",
        ).get(instance, q.resource_id, q.did);
        return reply({ ok: row?.active === 1 });
      }
      if (invitation.closed !== 0) return reply({ ok: false }, 403);
      if (q.operation === "inspect") {
        return reply({
          ok: true,
          sharer: invitation.operator,
          title: invitation.title,
          operations: invitation.action === "edit"
            ? ["read", "edit"]
            : ["read"],
          expires_at: null,
          methods: { did_login: true, oauth: true, password: true },
        });
      }
      if (
        typeof q.did !== "string" || !q.did.startsWith("did:") ||
        q.did.length > 256
      ) return reply({ ok: false }, 400);
      if (q.operation === "prepare") {
        db.prepare("INSERT OR IGNORE INTO grants VALUES(?,?,?,0)").run(
          instance,
          q.resource_id,
          q.did,
        );
      } else if (q.operation === "commit") {
        if (
          !db.prepare(
            "UPDATE grants SET active=1 WHERE instance=? AND invitation=? AND did=? AND active>=0",
          ).run(instance, q.resource_id, q.did).changes
        ) return reply({ ok: false }, 403);
      } else return reply({ ok: false }, 400);
      return reply({ ok: true });
    }
    if (!api) return reply({ ok: false }, 404);
    const user = await actor(req, context);
    if (!user) return reply({ ok: false }, 401);
    const id = api[1];
    let doc = db.prepare(
      "SELECT title,body FROM documents WHERE instance=? AND id=?",
    ).get(instance, id);
    if (!doc && user.owner && req.method === "GET") {
      db.prepare("INSERT OR IGNORE INTO documents VALUES(?,?,?,?)").run(
        instance,
        id,
        `Document ${id}`,
        "",
      );
      doc = db.prepare(
        "SELECT title,body FROM documents WHERE instance=? AND id=?",
      ).get(instance, id);
    }
    if (!doc) return reply({ ok: false }, 404);
    if (api[2]) {
      if (!user.owner) return reply({ ok: false }, 403);
      const q = await req.json();
      if (!validId(q.request_id)) return reply({ ok: false }, 400);
      if (req.method === "DELETE") {
        if (typeof q.did === "string") {
          db.prepare(
            "UPDATE grants SET active=-1 WHERE instance=? AND invitation IN (SELECT id FROM invitations WHERE instance=? AND document=? AND id=?) AND did=?",
          ).run(instance, instance, id, q.request_id, q.did);
        } else {db.prepare(
            "UPDATE invitations SET closed=1 WHERE instance=? AND document=? AND id=?",
          ).run(instance, id, q.request_id);}
        return reply({ ok: true });
      }
      if (req.method !== "POST" || !["read", "edit"].includes(q.action)) {
        return reply({ ok: false }, 400);
      }
      const path = new URL(q.return_path || `/docs/${id}`, url.origin);
      if (path.origin !== url.origin || path.pathname !== `/docs/${id}`) {
        return reply({ ok: false }, 400);
      }
      db.prepare(
        "INSERT OR IGNORE INTO invitations(instance,id,document,operator,action,secret) VALUES(?,?,?,?,?,?)",
      ).run(
        instance,
        q.request_id,
        id,
        context.ownerUserId,
        q.action,
        crypto.randomUUID() + crypto.randomUUID(),
      );
      const row = db.prepare(
        "SELECT * FROM invitations WHERE instance=? AND id=? AND document=? AND action=? AND closed=0",
      ).get(instance, q.request_id, id, q.action);
      if (!row) return reply({ ok: false }, 409);
      const result = await context.serviceClient("control-panel").call(
        "resource.share.issue",
        {
          resource_id: q.request_id,
          request_id: q.request_id,
          callback_secret: row.secret,
          audience: q.audience,
          recipient_email: q.recipient_email,
          return_path: path.pathname + path.search + path.hash,
        },
      );
      return reply(result);
    }
    const grants = db.prepare(
      "SELECT i.action FROM grants g JOIN invitations i ON i.instance=g.instance AND i.id=g.invitation WHERE g.instance=? AND i.document=? AND g.did=? AND g.active=1",
    ).all(instance, id, user.did);
    const edit = user.owner || grants.some((r) => r.action === "edit");
    if (!user.owner && !grants.length) return reply({ ok: false }, 404);
    if (req.method === "GET") {
      return reply({ ...doc, owner: user.owner, can_edit: edit });
    }
    if (req.method !== "PUT" || !edit) return reply({ ok: false }, 403);
    const body = (await req.json()).body;
    if (typeof body !== "string" || body.length > 262144) {
      return reply({ ok: false }, 400);
    }
    db.prepare("UPDATE documents SET body=? WHERE instance=? AND id=?").run(
      body,
      instance,
      id,
    );
    return reply({ ok: true });
  } catch {
    return reply({ ok: false }, 403);
  }
}
