type NodeSdkModule = typeof import("@sys-test/websdk-node-types");

export async function runXllmIdentitySelftest(
  sdk: NodeSdkModule,
  identity: { appId: string; ownerUserId: string },
): Promise<Record<string, unknown>> {
  const account = await sdk.buckyos.getAccountInfo();
  const token = account?.session_token;
  const claims = sdk.parseSessionTokenClaims(token);
  const appInstanceId = `${identity.appId}@${identity.ownerUserId}`;
  if (
    !token || claims?.iss !== "verify-hub" ||
    claims.token_use !== "session" || claims.target_kind !== "app" ||
    claims.appid !== identity.appId ||
    claims.app_instance_id !== appInstanceId ||
    claims.app_owner_user_id !== identity.ownerUserId ||
    claims.sub !== identity.ownerUserId || claims.principal_kind !== "app"
  ) {
    throw new Error(
      "xllm requires the logged-in AppService session for this app instance",
    );
  }
  const hostGateway = Deno.env.get("BUCKYOS_HOST_GATEWAY")?.trim();
  const device = JSON.parse(Deno.env.get("BUCKYOS_THIS_DEVICE") ?? "null");
  if (!hostGateway || device?.device_type !== "ood") {
    throw new Error(
      "issue #640 regression requires BUCKYOS_HOST_GATEWAY and an inherited OOD device document",
    );
  }

  const agentTool = Deno.env.get("BUCKYOS_SYSTEST_AGENT_TOOL")?.trim() ||
    `${Deno.env.get("BUCKYOS_ROOT") || "/opt/buckyos"}/bin/opendan/agent_tool`;
  const model = Deno.env.get("BUCKYOS_SYSTEST_XLLM_MODEL")?.trim() ||
    "llm.chat";
  const workdir = await Deno.makeTempDir({ prefix: "systest-xllm-" });
  const startedAt = Date.now();
  try {
    const child = new Deno.Command("/bin/bash", {
      args: [
        "--noprofile",
        "--norc",
        "-c",
        'exec "$@"',
        "systest-xllm",
        agentTool,
        "xllm",
        "--dir",
        workdir,
        "--runs-dir",
        `${workdir}/runs`,
        "--provider",
        "buckyos",
        "--model",
        model,
        "--no-tools",
        "--max-tokens",
        "256",
        "--timeout",
        "90",
        "--llm-timeout",
        "60",
        "--format",
        "json",
        "Reply with exactly: SYSTEST_XLLM_OK",
      ],
      cwd: workdir,
      env: { BUCKYOS_APPCLIENT_SESSION_TOKEN: token },
      stdin: "null",
      stdout: "piped",
      stderr: "piped",
    }).spawn();
    let timedOut = false;
    const timeout = setTimeout(() => {
      timedOut = true;
      try {
        child.kill("SIGKILL");
      } catch {
      }
    }, 120_000);
    let output: Deno.CommandOutput;
    try {
      output = await child.output();
    } finally {
      clearTimeout(timeout);
    }
    const decoder = new TextDecoder();
    const redact = (text: string) =>
      text.replaceAll(token, "[redacted]").slice(-2_000);
    if (timedOut || !output.success) {
      throw new Error(
        `bash → xllm ${timedOut ? "timed out" : `exited ${output.code}`}: ${
          redact(decoder.decode(output.stderr))
        }`,
      );
    }
    let result: Record<string, unknown>;
    try {
      result = JSON.parse(decoder.decode(output.stdout));
    } catch {
      throw new Error("xllm did not return a JSON result");
    }
    if (
      result.status !== "completed" || result.provider !== "buckyos" ||
      typeof result.run_id !== "string" || !result.run_id ||
      typeof result.answer !== "string" ||
      !result.answer.includes("SYSTEST_XLLM_OK")
    ) {
      throw new Error(
        `unexpected xllm result: ${redact(JSON.stringify(result))}`,
      );
    }

    const client = sdk.buckyos.getAiccClient();
    let events: Awaited<ReturnType<typeof client.queryUsage>>["events"] = [];
    for (let attempt = 0; attempt < 10; attempt++) {
      const usage = await client.queryUsage({
        time_range: {
          kind: "explicit",
          start_time_ms: startedAt - 1_000,
          end_time_ms: Date.now() + 1_000,
        },
        output_mode: "events",
        limit: 100,
      });
      events = usage.events?.filter((event) =>
        event.trace_id === result.run_id
      ) ?? [];
      if (events.length > 0) break;
      await new Promise((resolve) => setTimeout(resolve, 500));
    }
    if (!events.length) {
      throw new Error(`no AICC usage evidence for xllm run ${result.run_id}`);
    }
    const expectedCaller = `app:${appInstanceId}`;
    for (const event of events) {
      if (
        event.caller_app_id !== expectedCaller ||
        event.user_id !== identity.ownerUserId ||
        event.tenant_id !== identity.ownerUserId
      ) {
        throw new Error(
          `AICC recorded an unexpected caller for xllm run ${result.run_id}: ${event.caller_app_id}/${event.user_id}/${event.tenant_id}`,
        );
      }
    }
    return {
      runId: result.run_id,
      status: result.status,
      model: result.model,
      hostGateway,
      deviceType: device.device_type,
      callerAppId: expectedCaller,
      userId: identity.ownerUserId,
      usageEvents: events.length,
      taskIds: events.map((event) => event.task_id),
    };
  } finally {
    await Deno.remove(workdir, { recursive: true });
  }
}
