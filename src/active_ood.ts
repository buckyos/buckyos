#!/usr/bin/env -S deno run --allow-all
// active_ood.ts - dev-side RPC activation path (SN-Auth two-phase).
//
// make_config.ts pre-seeds identity files OFFLINE and never touches the
// activation RPC, so without this script the real activation flow
// (node_active web UI -> active_server do_active) has no dev equivalent.
// Against a node_daemon started with --enable_active this performs the same
// two-phase flow the web UI does (see node_active/active_lib.ts):
//
//   1. obtain an SN account access token (aud="sn", 1h) from
//      <schema>://sn.<sn_base_host>/kapi/sn/auth via auth.login — or
//      auth.register when --active-code is given (new account flow);
//   2. call do_active on <target>/kapi/active passing that token as
//      sn_access_token. active_server requires it whenever sn_url is set;
//      the old self-signed sn_device_proof path is gone.
//
// Seed facts come from devenv_config.ts groups plus the deterministic
// DEV_TEST_KEYS (websdk provision) — the same material make_sn_config.ts
// (cyfs-gateway repo) seeds into the SN DB and the BNS chain, so auth.login
// works out of the box for seed users (alice/bob/charlie, password
// DEV_TEST_PASSWORD below). BNS publish is skipped by default because the
// seed zone documents are already on-chain; pass --bns-evm-key (plus the
// --bns-* chain params from the SN VM's dv-env.json) to publish explicitly.
//
// Credential discipline: access token, pwd_hash and private keys are never
// logged here (SENSITIVE_LOG_KEYS mirrors active_lib.ts), and active_server
// strips sn_access_token & co from start_config.json on its side.
//
// Usage:
//   deno run --allow-all src/active_ood.ts <group> [--target <url>]
//     [--password <pwd>] [--active-code <code>] [--sn-ip <ip>]
//     [--sn-auth-url <url>]
//     [--bns-evm-key <hex> [--bns-url <url>] --bns-rpc <url>
//      --bns-contract <addr> --bns-chain-id <id>]
//   groups: OOD groups with a non-empty sn_base_host and an SN account
//     (alice.ood1 | bob.ood1 | charlie.ood1). --sn-ip keeps the
//     Host: sn.<base> header while connecting to the given IP, for hosts
//     that have not pointed DNS/hosts at the test SN. --sn-auth-url
//     overrides the full auth address this script dials (local SN on a
//     non-standard port); the sn_url handed to do_active stays canonical.

import { Buffer } from "node:buffer";
import { createHash } from "node:crypto";
import http from "node:http";
import https from "node:https";
import { parseArgs } from "node:util";
import {
  assertProvisionRuntime,
  getDevTestKeyPairById,
} from "buckyos/provision";
import {
  getParamsFromGroupName,
  OOD_GROUPS,
  type OODGroupParams,
} from "./devenv_config.ts";

type JsonValue =
  | string
  | number
  | boolean
  | null
  | JsonValue[]
  | { [key: string]: JsonValue };
type JsonObject = { [key: string]: JsonValue };

// 与 sn_seed.yaml 对齐的确定性 dev 测试密码（同步锚点：cyfs-gateway
// src/make_sn_config.ts 的 DEV_TEST_PASSWORD / SEED_ACTIVATION_CODES）。
const DEV_TEST_PASSWORD = "devtest-pwd";

const DEFAULT_TARGET = "http://127.0.0.1:3182";

// 凭证类字段绝不进 console，与 node_active/active_lib.ts 的 SENSITIVE_LOG_KEYS
// 和服务端 active_server.rs is_sensitive_param_key 保持同一纪律。
const SENSITIVE_LOG_KEYS = new Set([
  "sn_access_token",
  "sn_refresh_token",
  "access_token",
  "refresh_token",
  "private_key",
  "device_private_key",
  "owner_private_key",
  "bns_evm_private_key",
  "admin_password_hash",
  "friend_passcode",
  "pwd_hash",
]);

function redactForLog(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map(redactForLog);
  }
  if (value != null && typeof value === "object") {
    const redacted: Record<string, unknown> = {};
    for (const [key, child] of Object.entries(value)) {
      if (SENSITIVE_LOG_KEYS.has(key)) {
        const len = typeof child === "string" ? child.length : 0;
        redacted[key] = `[redacted:${len} chars]`;
      } else {
        redacted[key] = redactForLog(child);
      }
    }
    return redacted;
  }
  return value;
}

// SN 账号密码预哈希，与 websdk hashPassword(username, password)（无 nonce 档）
// 相同：base64(sha256(password + username + ".buckyos"))。激活参数里的
// admin_password_hash 与 SN 账号密码同源（前端 SecurityStep 同构）。
export function hashSnPassword(username: string, password: string): string {
  return createHash("sha256")
    .update(password + username + ".buckyos", "utf8")
    .digest("base64");
}

// ============================================================================
// minimal kRPC client over node:http
//
// 不复用 websdk kRPCClient：dev 宿主机常常没把 DNS/hosts 指到测试 SN，需要
// IP 直连 + 自定义 Host 头做 vhost 匹配，而 fetch 按规范丢弃 Host 头（实测
// Deno 如此），node:http 允许。协议形状与 websdk krpc_client.ts 一致：
// 请求 {method, params, sys:[seq]}，响应 {result, sys:[seq]} 或 {error}。
// ============================================================================

interface KrpcEndpoint {
  url: URL;
  // 连接目标 IP 覆盖（Host 头仍用 url.host 做 vhost 匹配）
  connectIp?: string;
}

let krpcSeq = Date.now();

function krpcCall(
  endpoint: KrpcEndpoint,
  method: string,
  params: JsonObject,
): Promise<JsonObject> {
  const seq = krpcSeq;
  krpcSeq += 1;
  const body = JSON.stringify({ method, params, sys: [seq] });
  const isHttps = endpoint.url.protocol === "https:";
  const requester = isHttps ? https : http;
  const port = endpoint.url.port
    ? Number(endpoint.url.port)
    : (isHttps ? 443 : 80);

  return new Promise((resolve, reject) => {
    const req = requester.request(
      {
        host: endpoint.connectIp ?? endpoint.url.hostname,
        port,
        path: endpoint.url.pathname + endpoint.url.search,
        method: "POST",
        headers: {
          Host: endpoint.url.host,
          "Content-Type": "application/json",
          "Content-Length": Buffer.byteLength(body),
        },
      },
      (res) => {
        let data = "";
        res.setEncoding("utf8");
        res.on("data", (chunk: string) => {
          data += chunk;
        });
        res.on("end", () => {
          if (res.statusCode == null || res.statusCode >= 400) {
            reject(
              new Error(`${method} failed: HTTP ${res.statusCode} ${data}`),
            );
            return;
          }
          let parsed: JsonObject;
          try {
            parsed = JSON.parse(data);
          } catch {
            reject(new Error(`${method} failed: invalid JSON response`));
            return;
          }
          if (parsed.error != null) {
            reject(
              new Error(`${method} failed: ${JSON.stringify(parsed.error)}`),
            );
            return;
          }
          const sys = parsed.sys;
          if (!Array.isArray(sys) || sys[0] !== seq) {
            reject(new Error(`${method} failed: seq not match`));
            return;
          }
          if (parsed.result === undefined) {
            reject(new Error(`${method} failed: response missing result`));
            return;
          }
          resolve(parsed.result as JsonObject);
        });
      },
    );
    req.on("error", (err: Error) =>
      reject(new Error(`${method} failed: ${err.message}`)));
    req.end(body);
  });
}

// ============================================================================
// phase 1: SN account access token (auth.login / auth.register)
// ============================================================================

async function acquireSnAccessToken(
  authEndpoint: KrpcEndpoint,
  username: string,
  pwdHash: string,
  activeCode: string | null,
): Promise<string> {
  const method = activeCode != null ? "auth.register" : "auth.login";
  console.log(`SN ${method} as ${username} via ${authEndpoint.url}`);
  const result = await krpcCall(authEndpoint, method, {
    name: username,
    pwd_hash: pwdHash,
    active_code: activeCode ?? "",
  });
  if (result.code !== undefined && result.code !== 0) {
    const hint = activeCode == null
      ? " (seed users are pre-registered with the devtest password; a new user needs --active-code)"
      : "";
    throw new Error(
      `SN ${method} rejected: ${JSON.stringify(redactForLog(result))}${hint}`,
    );
  }
  const accessToken = result.access_token;
  if (typeof accessToken !== "string" || accessToken.length === 0) {
    throw new Error(`SN ${method} response has no access_token`);
  }
  return accessToken;
}

// ============================================================================
// phase 2: do_active against the activation service
// ============================================================================

interface BnsPublishArgs {
  evmPrivateKey: string;
  bnsUrl: string;
  rpcEndpoint: string;
  contractAddress: string;
  chainId: number;
}

function edPublicKeyJwk(publicKeyX: string): JsonObject {
  return { crv: "Ed25519", kty: "OKP", x: publicKeyX };
}

// devenv 的 netid="lan" 在离线路径里表示"LAN 内 OOD、ood 描述串不带后缀"，
// 与 active RPC 的 "nat" 语义一致（web UI 的 BuckyForward 档）；其余
// wan / wan_dyn / portmap 与服务端判定一一对应，原样传递。
function rpcNetId(netid: string): string {
  return netid === "lan" ? "nat" : netid;
}

function buildDoActiveParams(
  params: OODGroupParams,
  snApiUrl: string,
  snAccessToken: string,
  pwdHash: string,
  activeCode: string | null,
  bns: BnsPublishArgs | null,
): JsonObject {
  // 确定性 dev 密钥（websdk DEV_TEST_KEYS），未知 id 会直接抛错
  const ownerKey = getDevTestKeyPairById(params.username);
  const deviceKey = getDevTestKeyPairById(
    `${params.username}.${params.node_name}`,
  );

  const activeParams: JsonObject = {
    user_name: params.username,
    // zone_id 的 host 形式由服务端 DID::from_str 解析：alice.bns.did ->
    // did:bns:alice，charlie.me -> did:web:charlie.me（自有域名档）。
    zone_name: params.zone_id,
    net_id: rpcNetId(params.netid),
    public_key: edPublicKeyJwk(ownerKey.publicKeyX),
    private_key: ownerKey.privateKeyPem,
    device_public_key: edPublicKeyJwk(deviceKey.publicKeyX),
    device_private_key: deviceKey.privateKeyPem,
    admin_password_hash: pwdHash,
    guest_access: false,
    friend_passcode: "",
    device_rtcp_port: params.rtcp_port,
    sn_active_code: activeCode ?? "",
    sn_username: params.username,
    sn_url: snApiUrl,
    sn_access_token: snAccessToken,
  };
  if (bns != null) {
    activeParams.bns_url = bns.bnsUrl;
    activeParams.bns_evm = {
      rpc_endpoint: bns.rpcEndpoint,
      contract_address: bns.contractAddress,
      chain_id: bns.chainId,
    };
    activeParams.bns_evm_private_key = bns.evmPrivateKey;
  }
  return activeParams;
}

// ============================================================================
// main
// ============================================================================

function printUsage(log: (message?: unknown) => void = console.error): void {
  log(
    "usage: active_ood.ts <group> [--target <url>] [--password <pwd>] [--active-code <code>]",
  );
  log(
    "       [--sn-ip <ip>] [--sn-auth-url <url>]",
  );
  log(
    "       [--bns-evm-key <hex> [--bns-url <url>] --bns-rpc <url> --bns-contract <addr> --bns-chain-id <id>]",
  );
  const snGroups = Object.entries(OOD_GROUPS)
    .filter(([, p]) => p.sn_base_host.trim().length > 0 && p.sn_account !== false)
    .map(([name]) => name);
  log(`groups with an SN account: ${snGroups.join(" | ")}`);
  log(`default target: ${DEFAULT_TARGET} (node_daemon --enable_active)`);
  log(
    "offline config pre-seeding (no activation RPC) stays in make_config.ts",
  );
}

export async function activateOodByGroupName(
  groupName: string,
  options: {
    target?: string;
    password?: string;
    activeCode?: string;
    snIp?: string;
    // 覆盖脚本自己连 auth 的完整地址（本地 SN 实例/非标准端口）；
    // 传给 do_active 的 sn_url 始终保持 OOD 视角可达的规范域名形式。
    snAuthUrl?: string;
    bns?: BnsPublishArgs | null;
  },
): Promise<void> {
  const params = getParamsFromGroupName(groupName);
  if (params.sn_base_host.trim().length === 0) {
    throw new Error(
      `group ${groupName} has no SN (sn_base_host is empty); RPC activation without SN is not supported here — use make_config.ts`,
    );
  }
  if (params.sn_account === false) {
    throw new Error(
      `group ${groupName} is a pure-web3 user (sn_account=false): it has no SN account to login`,
    );
  }

  const schema = params.force_https ? "https" : "http";
  const snHost = `sn.${params.sn_base_host}`;
  const snApiUrl = `${schema}://${snHost}/kapi/sn`;
  const authEndpoint: KrpcEndpoint = {
    url: new URL(options.snAuthUrl ?? `${snApiUrl}/auth`),
    connectIp: options.snIp,
  };
  const target = options.target ?? DEFAULT_TARGET;
  const activeEndpoint: KrpcEndpoint = {
    url: new URL("/kapi/active", target),
  };
  const password = options.password ?? DEV_TEST_PASSWORD;
  const activeCode = options.activeCode ?? null;

  console.log(
    `############ RPC activation for group: ${groupName} #########################`,
  );
  console.log(`target     : ${activeEndpoint.url}`);
  console.log(`user       : ${params.username}`);
  console.log(`zone       : ${params.zone_id}`);
  console.log(`net_id     : ${rpcNetId(params.netid)}`);
  console.log(
    `sn         : ${snApiUrl}${options.snIp ? ` (via ${options.snIp})` : ""}`,
  );

  const pwdHash = hashSnPassword(params.username, password);
  const snAccessToken = await acquireSnAccessToken(
    authEndpoint,
    params.username,
    pwdHash,
    activeCode,
  );
  console.log("SN access token acquired");

  const activeParams = buildDoActiveParams(
    params,
    snApiUrl,
    snAccessToken,
    pwdHash,
    activeCode,
    options.bns ?? null,
  );
  if (options.bns == null) {
    console.log(
      "BNS publish skipped (seed zone documents are already on-chain; pass --bns-evm-key to publish)",
    );
  }
  console.log(
    "call do_active:",
    JSON.stringify(redactForLog(activeParams), null, 2),
  );
  const result = await krpcCall(activeEndpoint, "do_active", activeParams);
  if (result.code !== 0) {
    throw new Error(
      `do_active rejected: ${JSON.stringify(redactForLog(result))}`,
    );
  }
  console.log("do_active result:", JSON.stringify(redactForLog(result)));
  console.log(
    `activation done. the activation service exits in ~2s and node_daemon restarts into normal boot as ${params.zone_id}.`,
  );
}

async function main(): Promise<void> {
  let values: {
    target?: string;
    password?: string;
    "active-code"?: string;
    "sn-ip"?: string;
    "sn-auth-url"?: string;
    "bns-evm-key"?: string;
    "bns-url"?: string;
    "bns-rpc"?: string;
    "bns-contract"?: string;
    "bns-chain-id"?: string;
    help?: boolean;
  };
  let positionals: string[];
  try {
    const parsed = parseArgs({
      args: Deno.args,
      options: {
        target: { type: "string" },
        password: { type: "string" },
        "active-code": { type: "string" },
        "sn-ip": { type: "string" },
        "sn-auth-url": { type: "string" },
        "bns-evm-key": { type: "string" },
        "bns-url": { type: "string" },
        "bns-rpc": { type: "string" },
        "bns-contract": { type: "string" },
        "bns-chain-id": { type: "string" },
        help: { type: "boolean", short: "h" },
      },
      allowPositionals: true,
    });
    values = parsed.values;
    positionals = parsed.positionals;
  } catch (e) {
    console.error(`argument error: ${e instanceof Error ? e.message : e}`);
    printUsage();
    Deno.exit(1);
  }

  if (values.help) {
    printUsage(console.log);
    return;
  }
  if (positionals.length !== 1) {
    console.error(
      `argument error: expected exactly one group, got ${positionals.length}`,
    );
    printUsage();
    Deno.exit(1);
  }

  let bns: BnsPublishArgs | null = null;
  if (values["bns-evm-key"]) {
    const missing = ["bns-rpc", "bns-contract", "bns-chain-id"].filter(
      (key) => !values[key as keyof typeof values],
    );
    if (missing.length > 0) {
      console.error(
        `argument error: --bns-evm-key needs ${missing.map((k) => `--${k}`).join(", ")} (values from the SN VM's dv-env.json)`,
      );
      printUsage();
      Deno.exit(1);
    }
    const chainId = Number(values["bns-chain-id"]);
    if (!Number.isFinite(chainId) || chainId <= 0) {
      console.error("argument error: --bns-chain-id must be a positive number");
      Deno.exit(1);
    }
    bns = {
      evmPrivateKey: values["bns-evm-key"]!,
      bnsUrl: values["bns-url"] ?? "",
      rpcEndpoint: values["bns-rpc"]!,
      contractAddress: values["bns-contract"]!,
      chainId,
    };
  }

  try {
    assertProvisionRuntime();
  } catch (e) {
    console.error(
      `runtime check failed: ${e instanceof Error ? e.message : e}`,
    );
    console.error("active_ood.ts requires Deno >= 2.2");
    Deno.exit(1);
  }

  try {
    const groupName = positionals[0];
    if (bns != null && bns.bnsUrl === "") {
      const params = getParamsFromGroupName(groupName);
      const schema = params.force_https ? "https" : "http";
      bns.bnsUrl = `${schema}://sn.${params.sn_base_host}/kapi/bns`;
    }
    await activateOodByGroupName(groupName, {
      target: values.target,
      password: values.password,
      activeCode: values["active-code"],
      snIp: values["sn-ip"],
      snAuthUrl: values["sn-auth-url"],
      bns,
    });
  } catch (e) {
    console.error(`activation failed: ${e instanceof Error ? e.message : e}`);
    printUsage();
    Deno.exit(1);
  }
}

if (import.meta.main) {
  await main();
}
