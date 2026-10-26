import { runXllmIdentitySelftest } from "./xllm_selftest.ts";

const assertModule = "node:assert/strict";
const { default: assert } = await import(assertModule);

Deno.test("xllm preserves AppService identity in AICC usage attribution", async (t) => {
  const identity = { appId: "systest.buckyos.bns.did", ownerUserId: "alice" };
  const appInstanceId = `${identity.appId}@${identity.ownerUserId}`;
  const runId = "systest-identity-regression";
  const token = "systest-fake-app-session";
  const validClaims = {
    iss: "verify-hub",
    token_use: "session",
    target_kind: "app",
    appid: identity.appId,
    app_instance_id: appInstanceId,
    app_owner_user_id: identity.ownerUserId,
    sub: identity.ownerUserId,
    principal_kind: "app",
  };
  const validEvent = {
    trace_id: runId,
    task_id: "systest-task",
    caller_app_id: identity.appId,
    user_id: identity.ownerUserId,
    tenant_id: identity.ownerUserId,
    attribution: {
      app_id: identity.appId,
      user_id: identity.ownerUserId,
      operator_id: identity.ownerUserId,
      caller: `app:${appInstanceId}`,
      mode: "direct",
    },
  };
  let claims = { ...validClaims };
  let event: Record<string, unknown> = structuredClone(validEvent);
  let queryCount = 0;
  const sdk = {
    parseSessionTokenClaims: (value: string) => {
      assert.equal(value, token);
      return claims;
    },
    buckyos: {
      getAccountInfo: async () => ({ session_token: token }),
      getAiccClient: () => ({
        queryUsage: async () => {
          queryCount++;
          return {
            events: [{ trace_id: "another-run", caller_app_id: "another-app" }, event],
          };
        },
      }),
    },
  } as unknown as Parameters<typeof runXllmIdentitySelftest>[0];

  const fixtureDir = await Deno.makeTempDir({ prefix: "systest-xllm-fixture-" });
  const tool = `${fixtureDir}/agent_tool`;
  const environment = {
    BUCKYOS_HOST_GATEWAY: "127.0.0.1:3180",
    BUCKYOS_THIS_DEVICE: JSON.stringify({ device_type: "ood" }),
    BUCKYOS_SYSTEST_AGENT_TOOL: tool,
    BUCKYOS_SYSTEST_XLLM_MODEL: "llm.chat",
  };
  const previousEnvironment = Object.keys(environment).map((key) => [key, Deno.env.get(key)] as const);
  try {
    await Deno.writeTextFile(tool, `#!/bin/bash
[[ "$BUCKYOS_APPCLIENT_SESSION_TOKEN" == "${token}" ]] || exit 1
printf '%s\\n' '${JSON.stringify({
      status: "completed",
      provider: "buckyos",
      model: "llm.chat",
      run_id: runId,
      answer: "SYSTEST_XLLM_OK",
    })}'
`);
    await Deno.chmod(tool, 0o700);
    for (const [key, value] of Object.entries(environment)) Deno.env.set(key, value);

    await t.step("accepts the business AppId and exact raw AppInstance attribution", async () => {
      const result = await runXllmIdentitySelftest(sdk, identity);
      assert.equal(result.callerAppId, identity.appId);
      assert.equal(result.appInstanceId, appInstanceId);
      assert.equal(result.userId, identity.ownerUserId);
      assert.equal(result.runId, runId);
      assert.equal(result.usageEvents, 1);
      assert.deepEqual(result.taskIds, [validEvent.task_id]);
    });

    for (const field of ["caller_app_id", "user_id", "tenant_id"]) {
      await t.step(`rejects a different usage ${field}`, async () => {
        event = { ...structuredClone(validEvent), [field]: "wrong-identity" };
        await assert.rejects(runXllmIdentitySelftest(sdk, identity), /unexpected caller/);
      });
    }

    await t.step("rejects the old instance-shaped caller_app_id", async () => {
      event = { ...structuredClone(validEvent), caller_app_id: `app:${appInstanceId}` };
      await assert.rejects(runXllmIdentitySelftest(sdk, identity), /unexpected caller/);
    });

    for (const attribution of [undefined, null, [], "invalid"]) {
      await t.step(`rejects invalid attribution ${JSON.stringify(attribution)}`, async () => {
        event = { ...structuredClone(validEvent), attribution };
        await assert.rejects(runXllmIdentitySelftest(sdk, identity), /unexpected caller/);
      });
    }

    for (const field of ["caller", "app_id", "user_id", "mode", "operator_id"]) {
      for (const missing of [false, true]) {
        await t.step(`rejects ${missing ? "missing" : "incorrect"} attribution ${field}`, async () => {
          const attribution: Record<string, unknown> = { ...validEvent.attribution };
          if (missing) delete attribution[field];
          else attribution[field] = "wrong-identity";
          event = { ...structuredClone(validEvent), attribution };
          await assert.rejects(runXllmIdentitySelftest(sdk, identity), /unexpected caller/);
        });
      }
    }

    await t.step("rejects a token bound to a different AppInstance before querying usage", async () => {
      event = structuredClone(validEvent);
      claims = { ...validClaims, app_instance_id: `${identity.appId}@bob` };
      const before = queryCount;
      await assert.rejects(runXllmIdentitySelftest(sdk, identity), /requires the logged-in AppService session/);
      assert.equal(queryCount, before);
    });
  } finally {
    for (const [key, value] of previousEnvironment) {
      if (value === undefined) Deno.env.delete(key);
      else Deno.env.set(key, value);
    }
    await Deno.remove(fixtureDir, { recursive: true });
  }
});
