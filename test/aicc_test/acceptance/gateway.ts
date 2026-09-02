export type RpcClient = {
  call: (method: string, params: Record<string, unknown>) => Promise<unknown>;
};

export type GatewayCredentials = {
  gatewayUrl: string;
  sessionToken?: string;
  username?: string;
  password?: string;
  appId: string;
};

export type GatewaySession = {
  sessionToken: string;
  userId: string;
  aicc: RpcClient;
  taskManager: RpcClient;
  systemConfig: RpcClient;
};

export async function callChatCompletions(
  client: RpcClient,
  request: Record<string, unknown>,
): Promise<Record<string, unknown>> {
  const model = request.model as Record<string, unknown> | undefined;
  const payload = request.payload as Record<string, unknown> | undefined;
  const input = payload?.input_json as Record<string, unknown> | undefined;
  const options = payload?.options as Record<string, unknown> | undefined;
  const raw = await client.call("chat.completions.create", {
    exact_model: model?.alias,
    messages: input?.messages ?? payload?.messages ?? [],
    tools: input?.tool_specs ?? payload?.tool_specs ?? [],
    response_format: input?.response_format,
    temperature: input?.temperature ?? options?.temperature,
    max_output_tokens: input?.max_output_tokens ?? options?.max_output_tokens,
    idempotency_key: request.idempotency_key,
  }) as Record<string, unknown>;
  if (raw.status !== "succeeded") return raw;
  return {
    ...raw,
    result: raw.message
      ? {
        message: raw.message,
        usage: raw.usage,
        cost: raw.cost,
        finish_reason: raw.finish_reason,
        provider_task_ref: raw.provider_task_ref,
        extra: raw.route_trace ? { route_trace: raw.route_trace } : undefined,
      }
      : null,
  };
}

export async function callImagesGenerate(
  client: RpcClient,
  request: Record<string, unknown>,
): Promise<Record<string, unknown>> {
  const model = request.model as Record<string, unknown> | undefined;
  const payload = request.payload as Record<string, unknown> | undefined;
  const input = payload?.input_json as Record<string, unknown> | undefined;
  const raw = await client.call("images.generate", {
    exact_model: model?.alias,
    prompt: input?.prompt,
    negative_prompt: input?.negative_prompt,
    n: input?.n,
    aspect_ratio: input?.aspect_ratio,
    size: input?.size,
    quality: input?.quality,
    style: input?.style,
    seed: input?.seed,
    output: input?.output,
    idempotency_key: request.idempotency_key,
  }) as Record<string, unknown>;
  if (raw.status !== "succeeded") return raw;
  const artifacts = Array.isArray(raw.artifacts) ? raw.artifacts : [];
  return {
    ...raw,
    result: artifacts.length > 0
      ? {
        message: {
          role: "assistant",
          content: artifacts.map((artifact) => ({
            type: "image",
            source: (artifact as Record<string, unknown>).resource,
          })),
        },
        usage: raw.usage,
        cost: raw.cost,
        provider_task_ref: raw.provider_task_ref,
        extra: raw.route_trace ? { route_trace: raw.route_trace } : undefined,
      }
      : null,
  };
}

export async function loginGateway(
  credentials: GatewayCredentials,
): Promise<GatewaySession> {
  const { buckyos } = await import("buckyos");
  const gatewayUrl = credentials.gatewayUrl.replace(/\/+$/, "");
  if (!gatewayUrl) throw new Error("gateway URL is required");
  let sessionToken = credentials.sessionToken?.trim() ?? "";
  let userId = credentials.username?.trim() ?? "";
  if (!sessionToken) {
    if (!credentials.username || !credentials.password) {
      throw new Error(
        "username and password are required without session_token",
      );
    }
    const nonce = Date.now();
    const loginRpc = new buckyos.kRPCClient(
      `${gatewayUrl}/kapi/control-panel`,
      null,
      nonce,
    ) as RpcClient;
    const raw = await loginRpc.call("auth.login", {
      username: credentials.username,
      password: buckyos.hashPassword(
        credentials.username,
        credentials.password,
        nonce,
      ),
      appid: "control-panel",
      target: { kind: "system", service_id: "control-panel" },
      login_nonce: nonce,
    });
    const result = raw as {
      session_token?: unknown;
      user_info?: { user_id?: unknown };
    };
    sessionToken = typeof result.session_token === "string"
      ? result.session_token.trim()
      : "";
    userId = typeof result.user_info?.user_id === "string"
      ? result.user_info.user_id.trim()
      : userId;
    if (!sessionToken) throw new Error("auth.login returned no session_token");
  }
  return {
    sessionToken,
    userId,
    aicc: new buckyos.kRPCClient(
      `${gatewayUrl}/kapi/aicc`,
      sessionToken,
    ) as RpcClient,
    taskManager: new buckyos.kRPCClient(
      `${gatewayUrl}/kapi/task-manager`,
      sessionToken,
    ) as RpcClient,
    systemConfig: new buckyos.kRPCClient(
      `${gatewayUrl}/kapi/system_config`,
      sessionToken,
    ) as RpcClient,
  };
}
