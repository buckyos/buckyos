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
      throw new Error("username and password are required without session_token");
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
