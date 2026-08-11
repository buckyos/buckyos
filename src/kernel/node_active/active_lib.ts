import {
  buckyos,
  namelib,
  sn,
  type BuckyOSOwnerDocument,
  type Ed25519Jwk,
} from "buckyos";
import {
  ActiveConfig,
  ActiveNameMapping,
  ActiveWizzardData,
  DomainBindingState,
  EnabledFeatures,
  GatewayTopology,
  GatewayType,
  JsonValue,
  PreparedActiveDocuments,
  RegionProbeStatus,
  SignedActiveDocuments,
  WebOwnerMaterial,
} from "./src/types";

export let SN_BASE_HOST = "buckyos.ai";
export let SN_HOST = `sn.${SN_BASE_HOST}`;
export let SN_API_URL = `https://${SN_HOST}/kapi/sn`;
export let SN_AUTH_API_URL = `${SN_API_URL}/auth`;
export let SN_DEVICEINFO_API_URL = `${SN_API_URL}/deviceinfo`;
export let SN_BNS_API_URL = `https://bns.${SN_BASE_HOST}/kapi/bns`;
export let WEB3_BASE_HOST = `web3.${SN_BASE_HOST}`;
export let AI_PROVIDER_TUTORIAL_URL = "https://buckyos.ai";
export let TELEGRAM_BOT_API_TOKEN_TUTORIAL_URL = "https://core.telegram.org/bots/tutorial";
export let TELEGRAM_ACCOUNT_ID_TUTORIAL_URL = "https://core.telegram.org/api/bots/ids";

export async function init_active_lib(config: ActiveConfig) {
  SN_BASE_HOST = config.sn_base_host;
  SN_HOST = `sn.${SN_BASE_HOST}`;
  SN_API_URL = `${config.http_schema}://${SN_HOST}/kapi/sn`;
  SN_AUTH_API_URL = `${SN_API_URL}/auth`;
  SN_DEVICEINFO_API_URL = `${SN_API_URL}/deviceinfo`;
  SN_BNS_API_URL = `${config.http_schema}://bns.${SN_BASE_HOST}/kapi/bns`;
  WEB3_BASE_HOST = `web3.${SN_BASE_HOST}`;
  AI_PROVIDER_TUTORIAL_URL = config.ai_provider_tutorial_url || AI_PROVIDER_TUTORIAL_URL;
  TELEGRAM_BOT_API_TOKEN_TUTORIAL_URL =
    config.telegram_bot_api_token_tutorial_url || TELEGRAM_BOT_API_TOKEN_TUTORIAL_URL;
  TELEGRAM_ACCOUNT_ID_TUTORIAL_URL =
    config.telegram_account_id_tutorial_url || TELEGRAM_ACCOUNT_ID_TUTORIAL_URL;
}

export function resolveEnabledFeatures(
  snActiveCode: string | null | undefined,
  explicit?: Partial<EnabledFeatures> | null,
): EnabledFeatures {
  return {
    llm_router:
      Boolean(explicit?.llm_router) ||
      (typeof snActiveCode === "string" && snActiveCode.trim().length > 0),
  };
}

function activeRpc() {
  return new buckyos.kRPCClient("/kapi/active");
}

export async function createInitialWizardData(
  initial?: Partial<ActiveWizzardData>,
): Promise<ActiveWizzardData> {
  const device = (await activeRpc().call("generate_device_key_pair", {})) as {
    public_key: Ed25519Jwk;
    private_key: string;
  };
  return {
    gatewy_type: GatewayType.BuckyForward,
    port_mapping_mode: "full",
    rtcp_port: 2980,
    use_self_domain: false,
    self_domain: "",
    domain_binding: { state: "unused" },
    sn_active_code: "",
    sn_user_name: "",
    sn_access_token: null,
    sn_refresh_token: null,
    enabled_features: resolveEnabledFeatures(initial?.sn_active_code, initial?.enabled_features),
    region_preference: "auto",
    region_probe_status: null,
    selected_region: null,
    admin_password_hash: "",
    friend_passcode: "",
    enable_guest_access: false,
    owner_document: null,
    evm_address: "",
    web_owner_material: null,
    device_public_key: device.public_key,
    device_private_key: device.private_key,
    prepared_documents: null,
    signed_documents: null,
    is_wallet_runtime: false,
    ai_provider_config: {
      openai_api_token: "",
      claude_api_token: "",
      google_api_token: "",
      openrouter_api_token: "",
      glm_api_token: "",
    },
    jarvis_msg_tunnel_config: {
      telegram_bot_api_token: "",
      telegram_account_id: "",
    },
    ...initial,
  };
}

function parseOwnerDocument(value: unknown): BuckyOSOwnerDocument {
  const parsed = typeof value === "string" ? JSON.parse(value) : value;
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error("owner_document must be a JSON object");
  }
  return JSON.parse(JSON.stringify(parsed)) as BuckyOSOwnerDocument;
}

function sameJwk(left: unknown, right: unknown): boolean {
  if (!left || !right || typeof left !== "object" || typeof right !== "object") return false;
  const a = left as Record<string, unknown>;
  const b = right as Record<string, unknown>;
  return a.kty === b.kty && a.crv === b.crv && a.x === b.x;
}

function containsSensitiveField(value: unknown): boolean {
  if (Array.isArray(value)) return value.some(containsSensitiveField);
  if (!value || typeof value !== "object") return false;
  return Object.entries(value as Record<string, unknown>).some(([key, child]) => {
    const normalized = key.toLowerCase();
    return (
      normalized.includes("mnemonic") ||
      normalized.includes("private_key") ||
      normalized.includes("password") ||
      normalized.includes("pwd_hash") ||
      normalized.includes("active_code") ||
      normalized.includes("token") ||
      normalized === "email" ||
      containsSensitiveField(child)
    );
  });
}

export function validateOwnerDocument(
  value: unknown,
  expectedSnUsername?: string,
  bridgePublicKey?: unknown,
): BuckyOSOwnerDocument {
  const owner = parseOwnerDocument(value);
  const name = owner.name?.trim().toLowerCase();
  if (!name || owner.name !== name || owner.id !== `did:bns:${name}`) {
    throw new Error("owner_document id/name mismatch");
  }
  if (expectedSnUsername && name !== expectedSnUsername.trim().toLowerCase()) {
    throw new Error("owner_document name does not match sn_username");
  }
  const key = owner.verificationMethod?.[0]?.publicKeyJwk as Record<string, unknown> | undefined;
  if (key?.kty !== "OKP" || key?.crv !== "Ed25519" || typeof key?.x !== "string" || !key.x) {
    throw new Error("owner_document default key is not Ed25519");
  }
  if (bridgePublicKey && !sameJwk(key, bridgePublicKey)) {
    throw new Error("wallet public_key does not match owner_document");
  }
  const wallet = owner.wallets?.main;
  if (
    wallet?.type !== "eth" ||
    typeof wallet.address !== "string" ||
    !/^0x[0-9a-fA-F]{40}$/.test(wallet.address)
  ) {
    throw new Error("owner_document wallets.main is not a valid EVM wallet");
  }
  if (containsSensitiveField(owner)) {
    throw new Error("owner_document contains sensitive fields");
  }
  const encoded = JSON.stringify(owner);
  if (new TextEncoder().encode(encoded).byteLength >= 4096) {
    throw new Error("owner_document exceeds 4KB");
  }
  if (JSON.stringify(JSON.parse(encoded)) !== encoded) {
    throw new Error("owner_document round-trip mismatch");
  }
  return owner;
}

export async function generateWebOwnerMaterial(): Promise<WebOwnerMaterial> {
  return (await activeRpc().call("generate_web_owner_material", {})) as WebOwnerMaterial;
}

export function buildWebOwnerDocument(
  normalizedName: string,
  material: WebOwnerMaterial,
): BuckyOSOwnerDocument {
  const owner = namelib.newOwnerDocument({
    did: `did:bns:${normalizedName}`,
    name: normalizedName,
    displayName: normalizedName,
    publicKeyJwk: material.owner_public_jwk,
  });
  owner.wallets = {
    main: {
      type: "eth",
      address: material.evm_address,
    },
  };
  return validateOwnerDocument(owner, normalizedName, material.owner_public_jwk);
}

export async function registerWebOwner(params: {
  name: string;
  email: string;
  pwdHash: string;
  activeCode: string;
  ownerDocument: BuckyOSOwnerDocument;
  evmAddress: string;
  region?: string | null;
}) {
  const client = new sn.SnClient(SN_API_URL);
  const request = {
    name: params.name,
    email: params.email.trim(),
    pwd_hash: params.pwdHash,
    active_code: params.activeCode,
    request_id: `sn:register:${params.name}`,
    asset_owner: params.evmAddress,
    owner_config: params.ownerDocument,
    ...(params.region ? { region: params.region } : {}),
  };
  const result = await client.register(request);
  if (
    result.code !== 0 ||
    result.need_bind_owner_key !== false
  ) {
    throw new Error("SN registration did not commit the BNS owner document");
  }
  return result;
}

export async function startRegionProbe(force = false): Promise<RegionProbeStatus> {
  return (await activeRpc().call("start_region_probe", { force })) as RegionProbeStatus;
}

export async function getRegionProbeStatus(): Promise<RegionProbeStatus> {
  return (await activeRpc().call("get_region_probe_status", {})) as RegionProbeStatus;
}

export async function waitForRegionProbe(force = false): Promise<RegionProbeStatus> {
  let status = await startRegionProbe(force);
  const deadline = Date.now() + 10_000;
  while (status.phase === "running" && Date.now() < deadline) {
    await new Promise((resolve) => window.setTimeout(resolve, 300));
    status = await getRegionProbeStatus();
  }
  return status;
}

export async function check_sn_active_code(activeCode: string): Promise<boolean> {
  return (await new sn.SnClient(SN_API_URL).checkActiveCode(activeCode)).valid;
}

export async function check_bucky_username(name: string) {
  return new sn.SnClient(SN_API_URL).checkUsername(name);
}

export function isValidDomain(domain: string): boolean {
  const value = domain.trim().toLowerCase();
  return (
    value.length <= 253 &&
    value.includes(".") &&
    value.split(".").every(
      (label) =>
        label.length > 0 &&
        label.length <= 63 &&
        !label.startsWith("-") &&
        !label.endsWith("-") &&
        /^[a-z0-9-]+$/.test(label),
    )
  );
}

export function get_net_id_by_gateway_type(
  gatewayType: GatewayType,
  portMappingMode: string,
): GatewayTopology["net_id"] {
  if (gatewayType === GatewayType.WAN) return "wan";
  if (gatewayType === GatewayType.PortForward) {
    return portMappingMode === "rtcp_only" ? "portmap" : "wan_dyn";
  }
  return "nat";
}

export function deriveActiveNames(data: ActiveWizzardData): ActiveNameMapping {
  const owner = data.owner_document;
  if (!owner) throw new Error("OwnerDocument is missing");
  const accessHostname = data.use_self_domain
    ? data.self_domain.trim().toLowerCase()
    : `${owner.name}.${WEB3_BASE_HOST}`;
  return {
    owner_name: owner.name,
    owner_did: owner.id,
    zone_did: data.use_self_domain ? `did:web:${accessHostname}` : owner.id,
    access_hostname: accessHostname,
    bns_publish_name: owner.name,
    use_self_domain: data.use_self_domain,
  };
}

export function deriveGatewayTopology(data: ActiveWizzardData): GatewayTopology {
  return {
    net_id: get_net_id_by_gateway_type(data.gatewy_type, data.port_mapping_mode),
    rtcp_port: data.rtcp_port,
    support_container: true,
    uses_sn_relay: data.gatewy_type !== GatewayType.WAN,
    sn_url: SN_API_URL,
  };
}

async function refreshAccessToken(refreshToken: string | null): Promise<string | null> {
  if (!refreshToken) return null;
  try {
    const result = await new sn.SnClient(SN_API_URL).refresh(refreshToken);
    return result.code === 0 && result.access_token ? result.access_token : null;
  } catch {
    return null;
  }
}

async function loginForSession(name: string, pwdHash: string) {
  const result = await new sn.SnClient(SN_API_URL).login({
    name,
    pwd_hash: pwdHash,
  });
  if (result.code !== 0 || !result.access_token) {
    throw new Error("SN login failed");
  }
  return result;
}

export async function acquireSnAccessToken(
  data: ActiveWizzardData,
  pwdHash?: string | null,
): Promise<string> {
  const refreshed = await refreshAccessToken(data.sn_refresh_token);
  if (refreshed) return refreshed;
  if (data.sn_user_name && pwdHash) {
    return (await loginForSession(data.sn_user_name, pwdHash)).access_token;
  }
  if (data.sn_access_token) return data.sn_access_token;
  throw new Error("No usable SN session");
}

type WalletSignResult = { signatures: string[]; pwdHash: string | null };

async function walletSign(payloads: Record<string, unknown>[]): Promise<WalletSignResult> {
  const raw = await buckyos.walletSignWithActiveDid(payloads);
  if (raw == null) throw new Error("Wallet signing was cancelled");
  const result = Array.isArray(raw)
    ? { signatures: raw, pwd_hash: null }
    : (raw as { signatures?: unknown; pwd_hash?: unknown });
  const signatures = Array.isArray(result.signatures)
    ? result.signatures.filter(
        (signature): signature is string => typeof signature === "string" && signature.length > 0,
      )
    : [];
  return {
    signatures,
    pwdHash:
      typeof result.pwd_hash === "string" && result.pwd_hash.trim()
        ? result.pwd_hash.trim()
        : null,
  };
}

function randomNonce(): string {
  const bytes = new Uint8Array(16);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (value) => value.toString(16).padStart(2, "0")).join("");
}

export async function authorizeWalletDomainSession(
  data: ActiveWizzardData,
  domain: string,
) {
  if (!data.owner_document || !data.sn_user_name) throw new Error("Wallet owner is incomplete");
  const authorization = await walletSign([
    {
      type: "buckyos.node_active.domain_bind",
      domain,
      owner_did: data.owner_document.id,
      iat: Math.floor(Date.now() / 1000),
      nonce: randomNonce(),
    },
  ]);
  if (authorization.signatures.length !== 1 || !authorization.pwdHash) {
    throw new Error("Wallet did not authorize the SN session");
  }
  return loginForSession(data.sn_user_name, authorization.pwdHash);
}

export async function bindUserDomain(
  data: ActiveWizzardData,
  domain: string,
): Promise<{
  binding: DomainBindingState;
  accessToken: string;
  refreshToken: string | null;
}> {
  let accessToken = await refreshAccessToken(data.sn_refresh_token);
  let refreshToken = data.sn_refresh_token;
  if (!accessToken && data.is_wallet_runtime) {
    const session = await authorizeWalletDomainSession(data, domain);
    accessToken = session.access_token;
    refreshToken = session.refresh_token;
  }
  if (!accessToken) {
    accessToken = await acquireSnAccessToken(data, data.admin_password_hash);
  }
  const client = new sn.SnClient(SN_API_URL, accessToken);
  try {
    const result = await client.bindDomain(domain);
    if (result.domain.trim().toLowerCase() !== domain) {
      throw new Error("SN verified a different domain");
    }
    return {
      binding: {
        state: "verified",
        domain,
        verified_at: result.verified_at,
      },
      accessToken,
      refreshToken,
    };
  } catch (error) {
    if (error instanceof sn.SnClientError && error.isSnError("domain_proof_failed")) {
      const proof = error.domainProofInfo();
      if (proof) {
        return {
          binding: {
            state: "challenge",
            domain,
            record_name: proof.pkx_record_name,
            value: proof.pkx,
            reason: proof.reason,
          },
          accessToken,
          refreshToken,
        };
      }
    }
    throw error;
  }
}

async function prepareDocuments(data: ActiveWizzardData): Promise<PreparedActiveDocuments> {
  if (!data.owner_document) throw new Error("OwnerDocument is missing");
  return (await activeRpc().call("prepare_active_documents", {
    owner_document: data.owner_document,
    names: deriveActiveNames(data),
    topology: deriveGatewayTopology(data),
    device_public_key: data.device_public_key,
  })) as PreparedActiveDocuments;
}

async function signWalletDocuments(prepared: PreparedActiveDocuments): Promise<{
  signed: SignedActiveDocuments;
  pwdHash: string;
}> {
  const first = await walletSign([
    prepared.boot_document as Record<string, unknown>,
    prepared.device_mini_document as Record<string, unknown>,
    prepared.device_document as Record<string, unknown>,
  ]);
  if (first.signatures.length !== 3 || !first.pwdHash) {
    throw new Error("Wallet did not return three document JWTs and pwd_hash");
  }
  const zoneDocument = (await activeRpc().call("assemble_zone_document", {
    prepared,
    boot_document_jwt: first.signatures[0],
    device_document_jwt: first.signatures[2],
    device_mini_document_jwt: first.signatures[1],
  })) as SignedActiveDocuments["zone_document"];
  const second = await walletSign([zoneDocument as Record<string, unknown>]);
  if (second.signatures.length !== 1) {
    throw new Error("Wallet did not return the ZoneDocument JWT");
  }
  if (second.pwdHash && second.pwdHash !== first.pwdHash) {
    throw new Error("Wallet returned inconsistent pwd_hash values");
  }
  return {
    pwdHash: first.pwdHash,
    signed: {
      boot_document: prepared.boot_document,
      boot_document_jwt: first.signatures[0],
      device_document: prepared.device_document,
      device_document_jwt: first.signatures[2],
      device_mini_document: prepared.device_mini_document,
      device_mini_document_jwt: first.signatures[1],
      zone_document: zoneDocument,
      zone_document_jwt: second.signatures[0],
    },
  };
}

export async function activateNode(data: ActiveWizzardData): Promise<{
  accessHostname: string;
  prepared: PreparedActiveDocuments;
  signed: SignedActiveDocuments;
}> {
  if (!data.owner_document) throw new Error("OwnerDocument is missing");
  if (data.use_self_domain && data.domain_binding.state !== "verified") {
    throw new Error("Custom domain has not been verified");
  }
  const prepared = await prepareDocuments(data);
  let signed: SignedActiveDocuments;
  let walletPwdHash: string | null = null;
  if (data.is_wallet_runtime) {
    const walletResult = await signWalletDocuments(prepared);
    signed = walletResult.signed;
    walletPwdHash = walletResult.pwdHash;
  } else {
    if (!data.web_owner_material) throw new Error("Web owner mnemonic is unavailable");
    signed = (await activeRpc().call("sign_web_active_documents", {
      mnemonic_words: data.web_owner_material.mnemonic_words,
      prepared,
    })) as SignedActiveDocuments;
  }
  const adminPasswordHash = data.is_wallet_runtime
    ? walletPwdHash || ""
    : data.admin_password_hash;
  const accessToken = await acquireSnAccessToken(data, adminPasswordHash);
  const result = (await activeRpc().call("commit_active", {
    owner_document: data.owner_document,
    prepared,
    signed_documents: signed,
    device_private_key: data.device_private_key,
    system_settings: {
      admin_password_hash: adminPasswordHash,
      guest_access: data.enable_guest_access,
      friend_passcode: data.friend_passcode,
      enabled_features: resolveEnabledFeatures(data.sn_active_code, data.enabled_features),
      ai_provider_config: data.ai_provider_config,
      jarvis_msg_tunnel_config: data.jarvis_msg_tunnel_config,
    },
    sn: {
      sn_url: SN_API_URL,
      bns_url: SN_BNS_API_URL,
      access_token: accessToken,
    },
  })) as { status: string; access_hostname: string };
  if (result.status !== "completed") throw new Error("Activation did not complete");
  return {
    accessHostname: result.access_hostname,
    prepared,
    signed,
  };
}
