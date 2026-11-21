import { handleResourceShare } from "./resource_share.ts";
import type { SupportContext } from "./support_group.ts";

Deno.test("resource invitations keep document ACLs and installation identities independent", async () => {
  const root = await Deno.makeTempDir();
  const previous = Deno.env.get("SCRIPT_DATA_ROOT");
  Deno.env.set("SCRIPT_DATA_ROOT", root);
  try {
    for (const installer of ["zoe", "eve"]) {
      const instance = `notes@${installer}`;
      let issued: Record<string, unknown> = {};
      const token = (sub: string, external = false) =>
        "eyJhbGciOiJub25lIn0." +
        btoa(
          JSON.stringify({
            sub,
            principal_kind: "user",
            restricted: external,
            resource_share_did: sub,
            instance,
          }),
        ) + ".test";
      const owner = token(installer);
      const visitor = token("did:web:b.zone.example", true);
      const other = token("did:web:c.zone.example", true);
      const context: SupportContext = {
        ownerUserId: installer,
        appInstanceId: instance,
        zoneHost: "zone.example",
        clientWithToken: () => {
          throw new Error("unexpected broad credential");
        },
        serviceClient: (service) => ({
          call(method, p) {
            if (service === "verify-hub" && method === "verify_token") {
              return Promise.resolve(
                [owner, visitor, other].includes(String(p.session_token)) &&
                  (p.expected_target as { app_instance_id: string })
                      .app_instance_id === instance,
              );
            }
            if (
              service === "control-panel" && method === "resource.share.issue"
            ) {
              issued = p;
              return Promise.resolve({
                url: "https://zone.example/sso/share/test",
              });
            }
            throw new Error("unexpected method");
          },
        }),
      };
      const call = async (
        path: string,
        method = "GET",
        t?: string,
        data?: unknown,
      ) => {
        const url = new URL(path, "https://app.zone.example");
        const headers = new Headers();
        if (t && method === "GET") {
          headers.set("cookie", `buckyos_session_token=${t}`);
        } else if (t) headers.set("authorization", `Bearer ${t}`);
        const r = await handleResourceShare(
          new Request(url, {
            method,
            headers,
            ...(data ? { body: JSON.stringify(data) } : {}),
          }),
          url,
          context,
        );
        if (!r) throw new Error("route missing");
        return { status: r.status, body: await r.json() };
      };
      const expect = (received: unknown, wanted: unknown) => {
        if (received !== wanted) {
          throw new Error(`Expected ${wanted}, got ${received}`);
        }
      };
      const doc = "/sdk/appservice/documents/42";
      expect((await call(doc, "GET", owner)).body.body, "");
      expect((await call(doc, "GET", visitor)).status, 404);
      expect(
        (await call(doc + "/invitations", "POST", owner, {
          request_id: "edit-b",
          action: "edit",
          audience: { kind: "open" },
          return_path: "/docs/42?mode=edit#comments",
        })).body.url,
        "https://zone.example/sso/share/test",
      );
      expect(issued.return_path, "/docs/42?mode=edit#comments");
      const editInvitation = issued;
      const cb = (op: string, invitation = editInvitation) =>
        call("/sdk/appservice/resource-share/callback", "POST", undefined, {
          operation: op,
          resource_id: invitation.resource_id,
          callback_secret: invitation.callback_secret,
          did: "did:web:b.zone.example",
        });
      expect((await cb("inspect")).body.sharer, installer);
      expect((await cb("prepare")).body.ok, true);
      expect((await call(doc, "GET", visitor)).status, 404);
      expect((await cb("commit")).body.ok, true);
      expect(
        (await call(doc, "PUT", visitor, { body: "Edit from B" })).status,
        200,
      );
      expect((await call(doc, "GET", owner)).body.body, "Edit from B");
      expect((await call(doc, "GET", other)).status, 404);
      expect(
        (await call("/sdk/appservice/documents/43", "GET", visitor)).status,
        404,
      );
      expect(
        (await call(doc + "/invitations", "DELETE", owner, {
          request_id: "edit-b",
        })).status,
        200,
      );
      expect((await cb("resume")).body.ok, true);
      expect(
        (await call(doc + "/invitations", "POST", owner, {
          request_id: "read-b",
          action: "read",
          audience: { kind: "directed", did: "did:web:b.zone.example" },
        })).status,
        200,
      );
      const readInvitation = issued;
      expect(readInvitation.resource_id, "read-b");
      expect((await cb("prepare", readInvitation)).body.ok, true);
      expect((await cb("commit", readInvitation)).body.ok, true);
      expect(
        (await call(doc + "/invitations", "DELETE", owner, {
          request_id: "edit-b",
          did: "did:web:b.zone.example",
        })).status,
        200,
      );
      expect((await cb("resume")).body.ok, false);
      expect((await cb("resume", readInvitation)).body.ok, true);
      const view = await call(doc, "GET", visitor);
      expect(view.status, 200);
      expect(view.body.body, "Edit from B");
      expect(view.body.can_edit, false);
      expect(
        (await call(doc, "PUT", visitor, { body: "Another edit" })).status,
        403,
      );
      expect((await call(doc, "GET", owner)).body.body, "Edit from B");
      expect(
        (await call(doc + "/invitations", "DELETE", owner, {
          request_id: "read-b",
          did: "did:web:b.zone.example",
        })).status,
        200,
      );
      expect((await cb("resume", readInvitation)).body.ok, false);
      expect((await call(doc, "GET", visitor)).status, 404);
    }
  } finally {
    if (previous === undefined) Deno.env.delete("SCRIPT_DATA_ROOT");
    else Deno.env.set("SCRIPT_DATA_ROOT", previous);
    await Deno.remove(root, { recursive: true });
  }
});
