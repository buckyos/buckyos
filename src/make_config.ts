#!/usr/bin/env -S deno run --allow-all
// make_config.ts - TypeScript rewrite of make_config.py.
//
// Design intent:
// These make_*_config scripts pre-seed on-disk user data before a service or
// software package starts. They intentionally use the public Web SDK provision
// surface, not buckycli internals, so the dev environment continuously simulates
// what a third-party tool or agent can do when it writes a user's software
// configuration directly.
//
// The generated data is treated as real local state. At boot time BuckyOS should
// not care whether these files came from a previous runtime, a backup restore,
// or this script; this is how the scripts exercise "local data is truth" and
// expose data-plane incompatibilities before release.
//
// Iteration direction:
// Keep this script short. It should write only the minimal seed/core user data
// needed to identify the zone, device, trust roots, and startup environment.
// Derived configs, indexes, caches, module-private databases, and dependent
// module state should be lazily constructed by the owning module from its proper
// truth source. If this script has to write both module A's data and module B's
// data when B can be rebuilt from A, that is a signal to improve B's lazy/seed
// initialization boundary instead of expanding this script.
//
// Runtime: Deno >= 2.2 (node:sqlite). The websdk import is mapped in
// src/deno.json (buckyos/provision -> buckyos-websdk dist). For local websdk
// development point that import at your local dist/provision.mjs.
//
// Scope: build target-rootfs config files with buckyos-websdk provision. There
// is no buckycli binary or python/buckyos_devkit dependency here.
//
// Usage: deno run --allow-all src/make_config.ts <group> [--rootfs <dir>] [--ca <dir>]
//   groups: dev | vmtest | devtest_ood1 | alice.ood1 | bob.ood1 | charlie.ood1 |
//           devtests_ood1 | sn_web | nightly | release
//
// Removed relative to make_config.py (by design, 2026-06 review):
// - meta_index.db.fileobj fake cache (make_repo_cache_file / global env part)
// - did_docs cache preheat (make_cache_did_docs)
// - bin pkg meta seeding (seed_bin_pkg_meta_db)
// SN config generation moved to the cyfs-gateway repo: src/make_sn_config.ts.

import * as fs from "node:fs";
import * as path from "node:path";
import * as os from "node:os";
import { Buffer } from "node:buffer";
import { createPrivateKey, sign } from "node:crypto";
import { parseArgs } from "node:util";
import {
  assertProvisionRuntime,
  createIdentityCertFromCa,
  createNodeConfigs,
  createUserEnv,
  deviceIdentityPathsForRoots,
  ensureCa,
  IdentityRoots,
  loadLocalNodeIdentityConfig,
} from "buckyos/provision";
import {
  DEFAULT_TRUST_DID,
  ENV_ROOT_DIR,
  getParamsFromGroupName,
  OOD_GROUPS,
  type OODGroupParams,
} from "./devenv_config.ts";

export interface ProvisionKeyPair {
  privateKeyPem: string;
  publicKeyX: string;
}

export interface BuildUserEnvKeyPairs {
  ownerKeyPair?: ProvisionKeyPair;
  deviceKeyPair?: ProvisionKeyPair;
}

// ============================================================================
// shared paths & helpers (mirror buckyos_devkit get_buckyos_root / BUCKYCLI_DIR)
// ============================================================================

const IS_WINDOWS = os.platform() === "win32";

export function getBuckyosRoot(): string {
  const fromEnv = Deno.env.get("BUCKYOS_ROOT");
  if (fromEnv) {
    return fromEnv;
  }
  if (IS_WINDOWS) {
    const appData = Deno.env.get("APPDATA") ?? Deno.env.get("USERPROFILE") ??
      ".";
    return path.join(appData, "buckyos");
  }
  return "/opt/buckyos/";
}

export function ensureDir(dirPath: string): string {
  fs.mkdirSync(dirPath, { recursive: true });
  return dirPath;
}

function copyIfExists(src: string, dst: string): void {
  if (!fs.existsSync(src)) {
    console.log(`skip missing file: ${src}`);
    return;
  }
  ensureDir(path.dirname(dst));
  fs.copyFileSync(src, dst);
  console.log(`copy ${src} -> ${dst}`);
}

function writeJson(filePath: string, data: unknown): void {
  ensureDir(path.dirname(filePath));
  fs.writeFileSync(filePath, JSON.stringify(data, null, 2) + "\n");
  console.log(`write json ${filePath}`);
}

function writeText(filePath: string, content: string): void {
  ensureDir(path.dirname(filePath));
  fs.writeFileSync(filePath, content);
  console.log(`write content ${filePath}`);
}

function removeIfExists(filePath: string): void {
  if (fs.existsSync(filePath)) {
    fs.rmSync(filePath);
    console.log(`remove ${filePath}`);
  }
}

function removeTreeIfExists(dirPath: string): void {
  if (fs.existsSync(dirPath)) {
    fs.rmSync(dirPath, { recursive: true, force: true });
    console.log(`remove ${dirPath}`);
  }
}

function requireFiles(filePaths: string[]): void {
  const missing = filePaths.filter((filePath) => !fs.existsSync(filePath));
  if (missing.length > 0) {
    throw new Error(`missing generated files: ${missing.join(", ")}`);
  }
}

function readJsonObject(filePath: string): Record<string, unknown> {
  const value = JSON.parse(fs.readFileSync(filePath, "utf8"));
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`invalid json object: ${filePath}`);
  }
  return value as Record<string, unknown>;
}

function requireString(
  value: Record<string, unknown>,
  key: string,
  source: string,
): string {
  const field = value[key];
  if (typeof field !== "string" || field.trim().length === 0) {
    throw new Error(`${source} missing string field: ${key}`);
  }
  return field;
}

function requireObject(
  value: Record<string, unknown>,
  key: string,
  source: string,
): Record<string, unknown> {
  const field = value[key];
  if (!field || typeof field !== "object" || Array.isArray(field)) {
    throw new Error(`${source} missing object field: ${key}`);
  }
  return field as Record<string, unknown>;
}

function parseDid(did: string): { method: string; id: string } {
  const parts = did.split(":");
  if (parts.length < 3 || parts[0] !== "did" || !parts[1]) {
    throw new Error(`invalid DID: ${did}`);
  }
  return { method: parts[1], id: parts.slice(2).join(":") };
}

function didRawHostName(did: string): string {
  const { method, id } = parseDid(did);
  const realId = id.split(":")[0];
  return method === "web" ? realId : `${realId}.${method}.did`;
}

function base64UrlEncodeBytes(value: Uint8Array): string {
  return Buffer.from(value).toString("base64").replaceAll("+", "-")
    .replaceAll("/", "_").replace(/=+$/g, "");
}

function base64UrlEncodeString(value: string): string {
  return base64UrlEncodeBytes(Buffer.from(value, "utf8"));
}

function signJwtEdDSA(
  payload: Record<string, unknown>,
  privateKeyPem: string,
): string {
  const header = base64UrlEncodeString(JSON.stringify({ alg: "EdDSA" }));
  const body = base64UrlEncodeString(JSON.stringify(payload));
  const signingInput = `${header}.${body}`;
  const signature = sign(
    null,
    Buffer.from(signingInput, "utf8"),
    createPrivateKey(privateKeyPem),
  );
  return `${signingInput}.${base64UrlEncodeBytes(signature)}`;
}

// ============================================================================
// global env config (machine.json + active_config.json)
// ============================================================================

// Extract the base domain from web3_bns, e.g. web3.devtests.org -> devtests.org
function extractBaseHost(web3Bns: string): string {
  if (web3Bns.startsWith("web3.")) {
    return web3Bns.slice(5);
  }
  const dot = web3Bns.indexOf(".");
  if (dot >= 0) {
    return web3Bns.slice(dot + 1);
  }
  return web3Bns;
}

export function didHostToRealHost(didHost: string, web3Bridge: string): string {
  if (didHost.endsWith(".bns.did")) {
    const result = didHost.split(".bns.did")[0] + "." + web3Bridge;
    console.log(`did_host_to_real_host: ${didHost} -> ${result}`);
    return result;
  }
  return didHost;
}

function uniqueStrings(values: string[]): string[] {
  const seen = new Set<string>();
  const result: string[] = [];
  for (const value of values) {
    if (seen.has(value)) {
      continue;
    }
    seen.add(value);
    result.push(value);
  }
  return result;
}

function getZoneHostNames(zoneId: string, web3Bridge: string): string[] {
  return uniqueStrings([zoneId, didHostToRealHost(zoneId, web3Bridge)]);
}

export function makeMachineConfig(
  targetDir: string,
  web3BridgeHost: string,
  bnsHost: string,
  trustDid: string[],
  forceHttps: boolean,
): void {
  const etcDir = ensureDir(path.join(targetDir, "etc"));
  writeJson(path.join(etcDir, "machine.json"), {
    web3_bridge: { bns: web3BridgeHost },
    bns_host: bnsHost,
    force_https: forceHttps,
    trust_did: trustDid,
  });
}

function makeGlobalEnvConfig(
  targetDir: string,
  web3BridgeHost: string,
  bnsHost: string,
  trustDid: string[],
  forceHttps: boolean,
  snBaseHost?: string,
): void {
  const activeSnBaseHost = snBaseHost?.trim() ||
    extractBaseHost(web3BridgeHost);
  makeMachineConfig(
    targetDir,
    web3BridgeHost,
    bnsHost,
    trustDid,
    forceHttps,
  );

  const activeConfigPath = path.join(
    targetDir,
    "bin",
    "node-active",
    "active_config.json",
  );
  writeJson(activeConfigPath, {
    sn_base_host: activeSnBaseHost,
    http_schema: forceHttps ? "https" : "http",
  });
  console.log(`create active config at ${activeConfigPath}`);
}

// ============================================================================
// user env construction
// ============================================================================

// Build (or rebuild) the user env dir <envRoot>/<zone_id> with provision.
// Mirrors the create_user_env + create_node_configs part of make_identity_files.
export async function buildUserEnv(
  params: OODGroupParams,
  envRoot: string,
  keyPairs: BuildUserEnvKeyPairs = {},
): Promise<string> {
  const userDir = ensureDir(path.join(envRoot, params.zone_id));
  const oodNameForZone = params.netid !== "lan"
    ? `${params.node_name}@${params.netid}`
    : params.node_name;

  await createUserEnv({
    username: params.username,
    hostname: params.zone_id,
    oodName: oodNameForZone,
    snBaseHost: params.sn_base_host,
    rtcpPort: params.rtcp_port,
    outputDir: userDir,
    ownerKeyPair: keyPairs.ownerKeyPair,
    deviceKeyPair: keyPairs.deviceKeyPair,
  });
  await createNodeConfigs({
    deviceName: params.node_name,
    netId: params.netid,
    envDir: userDir,
    ownerKeyPair: keyPairs.ownerKeyPair,
    deviceKeyPair: keyPairs.deviceKeyPair,
  });
  return userDir;
}

// ============================================================================
// identity files + TLS
// ============================================================================

const NODE_IDENTITY_SCHEMA_V2 = "buckyos.node_identity.v2";
const DEVICE_DOC_JWT_FILE_NAME = "device_doc.jwt";
const DEVICE_MINI_DOC_JWT_FILE_NAME = "device_mini_doc.jwt";
const ZONE_DOCUMENT_JWT_FILE_NAME = "zone_document.jwt";
const MAX_INLINE_DOCUMENT_BYTES = 4096;

export interface LocalDeviceIdentityFiles {
  ownerDocument: Record<string, unknown>;
  bootDocument: Record<string, unknown>;
  bootDocumentJwt: string;
  deviceDocJwt: string;
  deviceMiniDocJwt: string;
  deviceDid: string;
  zoneDid: string;
  zoneDocument: Record<string, unknown>;
  zoneDocumentJwt: string;
}

function requireNumber(
  value: Record<string, unknown>,
  key: string,
  source: string,
): number {
  const field = value[key];
  if (typeof field !== "number" || !Number.isFinite(field)) {
    throw new Error(`${source} missing number field: ${key}`);
  }
  return field;
}

function requireArray(
  value: Record<string, unknown>,
  key: string,
  source: string,
): unknown[] {
  const field = value[key];
  if (!Array.isArray(field)) {
    throw new Error(`${source} missing array field: ${key}`);
  }
  return field;
}

function defaultPublicJwk(
  document: Record<string, unknown>,
  source: string,
): Record<string, unknown> {
  const methods = requireArray(document, "verificationMethod", source);
  const method = methods.find((item) =>
    item != null && typeof item === "object" && !Array.isArray(item) &&
    (item as Record<string, unknown>).id === "#main_key"
  );
  if (!method) {
    throw new Error(`${source} missing #main_key verification method`);
  }
  return requireObject(
    method as Record<string, unknown>,
    "publicKeyJwk",
    source,
  );
}

function normalizeOwnerDocument(
  userDir: string,
  zoneDid: string,
): Record<string, unknown> {
  const ownerPath = path.join(userDir, "user_config.json");
  const ownerDocument = structuredClone(readJsonObject(ownerPath));
  const ownerDid = requireString(ownerDocument, "id", ownerPath);
  const displayName = typeof ownerDocument.display_name === "string"
    ? ownerDocument.display_name
    : requireString(ownerDocument, "full_name", ownerPath);
  ownerDocument.display_name = displayName;
  delete ownerDocument.full_name;

  if (ownerDid !== zoneDid) {
    const boundZones = Array.isArray(ownerDocument.binded_zone_list)
      ? ownerDocument.binded_zone_list.filter((value) =>
        typeof value === "string" && value !== zoneDid
      )
      : [];
    ownerDocument.binded_zone_list = [zoneDid, ...boundZones];

    const lastDocId = `${ownerDid}#lastDoc`;
    const services = Array.isArray(ownerDocument.service)
      ? ownerDocument.service.filter((value) =>
        value == null || typeof value !== "object" || Array.isArray(value) ||
        (value as Record<string, unknown>).id !== lastDocId
      )
      : [];
    services.push({
      id: lastDocId,
      type: "DIDDoc",
      serviceEndpoint: `https://${didRawHostName(zoneDid)}/resolve/${ownerDid}`,
    });
    ownerDocument.service = services;
  }
  return ownerDocument;
}

function makeDeviceMiniDocument(
  deviceDocument: Record<string, unknown>,
  source: string,
): Record<string, unknown> {
  const deviceJwk = defaultPublicJwk(deviceDocument, source);
  const result: Record<string, unknown> = {
    n: requireString(deviceDocument, "name", source),
    x: requireString(deviceJwk, "x", source),
    p: requireNumber(deviceDocument, "rtcp_port", source),
    iat: requireNumber(deviceDocument, "iat", source),
    exp: requireNumber(deviceDocument, "exp", source),
  };
  return result;
}

function makeAccessHostname(params: OODGroupParams): string {
  return didHostToRealHost(params.zone_id, params.web3_bridge);
}

function makeSnApiUrl(params: OODGroupParams): string | undefined {
  const baseHost = params.sn_base_host.trim();
  if (!baseHost) {
    return undefined;
  }
  const schema = params.force_https ? "https" : "http";
  return `${schema}://sn.${baseHost}/kapi/sn`;
}

function writeLocalDeviceIdentityFiles(
  userDir: string,
  nodeDir: string,
  targetDir: string,
  params: OODGroupParams,
): LocalDeviceIdentityFiles {
  const nodeIdentityPath = path.join(nodeDir, "node_identity.json");
  const nodeIdentity = loadLocalNodeIdentityConfig(nodeIdentityPath);
  const zoneDid = nodeIdentity.zone_did;
  const ownerDid = nodeIdentity.owner_did;
  const ownerPublicKey = nodeIdentity.owner_public_key as unknown as Record<
    string,
    unknown
  >;
  const deviceName = nodeIdentity.device_name;
  const deviceDid = nodeIdentity.device_did;
  const sourceRoots = new IdentityRoots(
    path.join(nodeDir, "local", "identity"),
    path.join(nodeDir, "security"),
  );
  const sourcePaths = deviceIdentityPathsForRoots(sourceRoots, deviceDid);
  const deviceConfig = readJsonObject(sourcePaths.didJson);
  if (requireString(deviceConfig, "id", sourcePaths.didJson) !== deviceDid) {
    throw new Error(
      `${sourcePaths.didJson} id differs from node identity device_did`,
    );
  }
  const zoneConfigPath = path.join(userDir, "zone_config.json");
  const zoneConfig = readJsonObject(zoneConfigPath);
  const bootSeedPath = path.join(userDir, `${params.zone_id}.zone.json`);
  const bootDocument = structuredClone(readJsonObject(bootSeedPath));
  const ownerDocument = normalizeOwnerDocument(userDir, zoneDid);
  const ownerPrivateKeyPem = fs.readFileSync(
    path.join(userDir, "user_private_key.pem"),
    "utf8",
  );
  const devicePrivateKeyPem = fs.readFileSync(
    sourcePaths.authenticationPrivateKey,
    "utf8",
  );
  const ownerPublicKeyFromDocument = defaultPublicJwk(
    ownerDocument,
    path.join(userDir, "user_config.json"),
  );
  for (const key of ["kty", "crv", "x"]) {
    if (ownerPublicKeyFromDocument[key] !== ownerPublicKey[key]) {
      throw new Error(
        `OwnerDocument ${key} differs from node identity owner key`,
      );
    }
  }

  const documentIat = requireNumber(zoneConfig, "iat", zoneConfigPath);
  const documentExp = requireNumber(bootDocument, "exp", bootSeedPath);
  const usesSnRelay = params.netid !== "wan";
  const snApiUrl = makeSnApiUrl(params);
  if (usesSnRelay && !snApiUrl) {
    throw new Error(
      `group ${params.username} needs SN relay but has no SN base host`,
    );
  }
  bootDocument.id = zoneDid;
  delete bootDocument.owner;
  delete bootDocument.owner_key;
  delete bootDocument.iat;
  if (usesSnRelay) {
    bootDocument.sn = `sn.${params.sn_base_host.trim()}`;
  } else {
    delete bootDocument.sn;
  }
  const bootDocumentJwt = signJwtEdDSA(bootDocument, ownerPrivateKeyPem);

  deviceConfig.owner = ownerDid;
  deviceConfig.zone_did = zoneDid;
  deviceConfig.net_id = params.netid === "lan" ? "nat" : params.netid;
  deviceConfig.rtcp_port = params.rtcp_port;
  delete deviceConfig.support_container;
  delete deviceConfig.device_mini_document_jwt;
  deviceConfig.iat = documentIat;
  deviceConfig.exp = documentExp;
  deviceConfig.version_seq = 0;
  if (usesSnRelay) {
    deviceConfig.ddns_sn_url = snApiUrl;
  } else {
    delete deviceConfig.ddns_sn_url;
  }
  const deviceDocJwt = signJwtEdDSA(deviceConfig, ownerPrivateKeyPem);
  const deviceMiniDocument = makeDeviceMiniDocument(
    deviceConfig,
    sourcePaths.didJson,
  );
  const deviceMiniDocJwt = signJwtEdDSA(
    deviceMiniDocument,
    ownerPrivateKeyPem,
  );

  const zoneDocument = structuredClone(zoneConfig);
  zoneDocument.id = zoneDid;
  zoneDocument.owner = ownerDid;
  zoneDocument.hostname = makeAccessHostname(params);
  zoneDocument.oods = structuredClone(
    requireArray(bootDocument, "oods", bootSeedPath),
  );
  zoneDocument.boot_jwt = bootDocumentJwt;
  zoneDocument.devices = {
    [deviceName]: structuredClone(deviceConfig),
  };
  zoneDocument.mini_device_jwts = {
    [deviceName]: deviceMiniDocJwt,
  };
  zoneDocument.iat = documentIat;
  zoneDocument.exp = documentExp;
  zoneDocument.version_seq = 0;
  if (usesSnRelay) {
    zoneDocument.sn = `sn.${params.sn_base_host.trim()}`;
  } else {
    delete zoneDocument.sn;
  }
  const zoneDocumentJwt = signJwtEdDSA(zoneDocument, ownerPrivateKeyPem);
  if (Buffer.byteLength(zoneDocumentJwt, "utf8") >= MAX_INLINE_DOCUMENT_BYTES) {
    throw new Error(
      `zone document JWT exceeds ${MAX_INLINE_DOCUMENT_BYTES} byte BNS inline limit`,
    );
  }

  const roots = new IdentityRoots(
    path.join(targetDir, "local", "identity"),
    path.join(targetDir, "security"),
  );
  const publicDir = roots.publicDir(deviceDid);
  const securityDir = roots.securityDir(deviceDid);
  const didJsonPath = roots.publicFile(
    deviceDid,
    "authentication",
    "did-json",
  );
  const privateKeyPath = roots.securityFile(
    deviceDid,
    "authentication",
    "private",
  );

  const etcDir = ensureDir(path.join(targetDir, "etc"));
  ensureDir(publicDir);
  ensureDir(securityDir);
  writeJson(path.join(etcDir, "node_identity.json"), {
    schema: NODE_IDENTITY_SCHEMA_V2,
    zone_did: zoneDid,
    owner_did: ownerDid,
    owner_public_key: ownerPublicKey,
    device_name: deviceName,
    device_did: deviceDid,
    zone_iat: documentIat,
  });
  writeJson(path.join(etcDir, "node_gateway_params.json"), {
    params: {
      device_did: deviceDid,
    },
  });
  writeJson(didJsonPath, deviceConfig);
  writeText(path.join(publicDir, DEVICE_DOC_JWT_FILE_NAME), deviceDocJwt);
  writeText(
    path.join(publicDir, DEVICE_MINI_DOC_JWT_FILE_NAME),
    deviceMiniDocJwt,
  );
  writeText(privateKeyPath, devicePrivateKeyPem);
  requireFiles([
    path.join(etcDir, "node_identity.json"),
    path.join(etcDir, "node_gateway_params.json"),
    didJsonPath,
    path.join(publicDir, DEVICE_DOC_JWT_FILE_NAME),
    path.join(publicDir, DEVICE_MINI_DOC_JWT_FILE_NAME),
    privateKeyPath,
  ]);

  return {
    ownerDocument,
    bootDocument,
    bootDocumentJwt,
    deviceDocJwt,
    deviceMiniDocJwt,
    deviceDid,
    zoneDid,
    zoneDocument,
    zoneDocumentJwt,
  };
}

export function copyIdentityOutputs(
  userDir: string,
  nodeDir: string,
  targetDir: string,
  params: OODGroupParams,
): LocalDeviceIdentityFiles {
  const etcDir = ensureDir(path.join(targetDir, "etc"));

  const localIdentity = writeLocalDeviceIdentityFiles(
    userDir,
    nodeDir,
    targetDir,
    params,
  );

  const startConfigPath = path.join(nodeDir, "start_config.json");
  const startConfig = readJsonObject(startConfigPath);
  startConfig.owner_document = localIdentity.ownerDocument;
  startConfig.zone_name = localIdentity.zoneDid;
  startConfig.access_hostname = makeAccessHostname(params);
  startConfig.zone_document_jwt = localIdentity.zoneDocumentJwt;
  startConfig.boot_config_jwt = localIdentity.bootDocumentJwt;
  startConfig.device_doc_jwt = localIdentity.deviceDocJwt;
  startConfig.device_mini_doc_jwt = localIdentity.deviceMiniDocJwt;
  startConfig.ood_jwt = localIdentity.deviceDocJwt;
  startConfig.enabled_features ??= {};
  startConfig.ai_provider_config ??= {};
  startConfig.jarvis_msg_tunnel_config ??= {};
  delete startConfig.BUCKYOS_ROOT;
  delete startConfig.gateway_type;
  delete startConfig.private_key;
  delete startConfig.public_key;
  delete startConfig.device_private_key;
  delete startConfig.device_public_key;
  writeJson(path.join(etcDir, "start_config.json"), startConfig);
  writeText(
    path.join(etcDir, ZONE_DOCUMENT_JWT_FILE_NAME),
    localIdentity.zoneDocumentJwt,
  );
  writeJson(
    path.join(
      etcDir,
      `${didRawHostName(localIdentity.zoneDid)}.zone.json`,
    ),
    localIdentity.bootDocument,
  );
  removeIfExists(path.join(etcDir, "node_private_key.pem"));
  removeIfExists(path.join(etcDir, "node_device_config.json"));

  const buckycliDir = ensureDir(path.join(etcDir, ".buckycli"));
  writeJson(
    path.join(buckycliDir, "user_config.json"),
    localIdentity.ownerDocument,
  );
  writeJson(
    path.join(buckycliDir, "zone_config.json"),
    localIdentity.zoneDocument,
  );
  removeIfExists(path.join(buckycliDir, "user_private_key.pem"));
  console.log(
    `device identity ${localIdentity.deviceDid} copied to local identity roots`,
  );
  return localIdentity;
}

function makeUnactivatedIdentityConfig(targetDir: string): void {
  const etcDir = ensureDir(path.join(targetDir, "etc"));
  removeIfExists(path.join(etcDir, "node_identity.json"));
  removeIfExists(path.join(etcDir, "start_config.json"));
  removeIfExists(path.join(etcDir, ZONE_DOCUMENT_JWT_FILE_NAME));
  removeIfExists(path.join(etcDir, "node_private_key.pem"));
  removeIfExists(path.join(etcDir, "node_device_config.json"));
  removeTreeIfExists(path.join(etcDir, ".buckycli"));
  writeJson(path.join(etcDir, "node_gateway_params.json"), {
    params: {
      device_did: "did:bns:unactivated.local",
    },
  });
}

async function generateTls(
  zoneId: string,
  web3Bridge: string,
  caName: string,
  targetDir: string,
  caDir: string,
  writePostGateway: boolean,
): Promise<void> {
  const etcDir = ensureDir(path.join(targetDir, "etc"));
  const publicRoot = ensureDir(path.join(targetDir, "local", "identity"));
  const securityRoot = ensureDir(path.join(targetDir, "security"));
  const roots = {
    publicRoot,
    securityRoot,
    buckyosRoot: targetDir,
  };

  await ensureCa(caDir, caName);
  const certPaths: string[] = [];
  const zoneHosts = getZoneHostNames(zoneId, web3Bridge);
  for (const host of zoneHosts) {
    const exactCert = await createIdentityCertFromCa(caDir, host, roots, {
      usage: "server",
      hostnames: [host],
    });
    const wildcardHost = `*.${host}`;
    const wildcardCert = await createIdentityCertFromCa(
      caDir,
      wildcardHost,
      roots,
      {
        usage: "server",
        hostnames: [wildcardHost],
        uriSans: [],
      },
    );
    certPaths.push(
      exactCert.fullchainPath,
      exactCert.keyPath,
      exactCert.metadataPath,
      wildcardCert.fullchainPath,
      wildcardCert.keyPath,
      wildcardCert.metadataPath,
    );
  }
  requireFiles(certPaths);
  removeIfExists(path.join(etcDir, "zone_cert.cert"));
  removeIfExists(path.join(etcDir, "zone_cert_key.pem"));

  // Keep CA for trust
  copyIfExists(
    path.join(caDir, `${caName}_ca_cert.pem`),
    path.join(etcDir, "ca.cert"),
  );
  copyIfExists(
    path.join(caDir, `${caName}_ca_key.pem`),
    path.join(etcDir, "ca_key.pem"),
  );
  console.log(`tls identity certs generated under ${publicRoot}`);

  if (!writePostGateway) {
    writeText(path.join(etcDir, "post_gateway.yaml"), "--- {}\n");
    return;
  }

  const gatewayHosts = zoneHosts.flatMap((host) => [host, `*.${host}`]);
  const postGatewayConfig = `
stacks:
  zone_gateway_https:
    bind: 0.0.0.0:443
    protocol: tls
    hosts:
${gatewayHosts.map((host) => `      - "${host}"`).join("\n")}
    identity_manager:
      public_root_path: ../local/identity
      security_root_path: ../security
    hook_point:
      main:
        id: main
        priority: 1
        blocks:
          default:
            id: default
            priority: 1
            block: |
              return "server node_gateway";
    `;
  writeText(path.join(etcDir, "post_gateway.yaml"), postGatewayConfig);
}

async function makeIdentityFiles(
  targetDir: string,
  params: OODGroupParams,
  caDir: string,
): Promise<void> {
  const userDir = await buildUserEnv(params, ENV_ROOT_DIR);
  const nodeDir = path.join(userDir, params.node_name);
  copyIdentityOutputs(userDir, nodeDir, targetDir, params);

  await generateTls(
    params.zone_id,
    params.web3_bridge,
    params.ca_name,
    targetDir,
    caDir,
    params.sn_base_host.trim().length === 0,
  );
}

// ============================================================================
// dev boot template override (~/.buckycli/buckyos_boot.toml merge)
// ============================================================================

// Matches TOML entries of the form: "key" = """\n<value>\n"""
const BOOT_ENTRY_PATTERN =
  /^[^\S\n]*"([^"\n]+)"[^\S\n]*=[^\S\n]*"""\n([\s\S]*?)\n"""[ \t]*\n?/gm;

function escapeRegExp(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function applyDevBootTemplateOverride(
  targetDir: string,
  groupName: string,
): void {
  if (groupName !== "dev" && groupName !== "devtest_ood1") {
    return;
  }

  const localBootToml = path.join(
    os.homedir(),
    ".buckycli",
    "buckyos_boot.toml",
  );
  if (!fs.existsSync(localBootToml)) {
    console.log(`skip missing dev boot override: ${localBootToml}`);
    return;
  }

  const dstBootTemplate = path.join(
    targetDir,
    "etc",
    "scheduler",
    "boot.template.toml",
  );
  const srcText = fs.readFileSync(localBootToml, "utf8");
  const entries: Array<[string, string]> = [];
  for (const match of srcText.matchAll(BOOT_ENTRY_PATTERN)) {
    const key = match[1];
    console.log(`load ${key} from .buckycli/boot_template`);
    entries.push([key, `"${key}" = """\n${match[2]}\n"""`]);
  }

  if (entries.length === 0) {
    console.log(
      `skip invalid dev boot override (no key/value found): ${localBootToml}`,
    );
    return;
  }

  if (!fs.existsSync(dstBootTemplate)) {
    writeText(dstBootTemplate, srcText);
    console.log(`create boot template from local override: ${dstBootTemplate}`);
    return;
  }

  let merged = fs.readFileSync(dstBootTemplate, "utf8");
  let replaced = 0;
  let added = 0;
  for (const [key, block] of entries) {
    const targetPattern = new RegExp(
      `^[^\\S\\n]*"${
        escapeRegExp(key)
      }"[^\\S\\n]*=[^\\S\\n]*"""\\n[\\s\\S]*?\\n"""[ \\t]*\\n?`,
      "m",
    );
    if (targetPattern.test(merged)) {
      merged = merged.replace(targetPattern, block + "\n");
      replaced += 1;
      continue;
    }

    if (merged && !merged.endsWith("\n")) {
      merged += "\n";
    }
    if (merged && !merged.endsWith("\n\n")) {
      merged += "\n";
    }
    merged += block + "\n";
    added += 1;
  }

  writeText(dstBootTemplate, merged);
  console.log(
    `merge dev boot override into template: ${dstBootTemplate} (replaced=${replaced}, added=${added})`,
  );
}

// ============================================================================
// main
// ============================================================================

function printUsage(log: (message?: unknown) => void = console.error): void {
  log("usage: make_config.ts <group> [--rootfs <dir>] [--ca <dir>]");
  log(
    `groups: ${[...Object.keys(OOD_GROUPS), "nightly", "release"].join(" | ")}`,
  );
  log(
    "sn configs: moved to the cyfs-gateway repo: deno run -A src/make_sn_config.ts [--rootfs <dir>] [--ca <dir>] [--sn_ip <ip>]",
  );
  log(
    "RPC activation (SN-Auth two-phase, against node_daemon --enable_active): deno run -A src/active_ood.ts <group>",
  );
}

export async function makeConfigByGroupName(
  groupName: string,
  targetRoot: string | undefined,
  caDir: string | undefined,
): Promise<void> {
  if (groupName === "sn" || groupName === "sn_server") {
    throw new Error(
      "SN config generation moved to the cyfs-gateway repo: deno run --allow-all src/make_sn_config.ts",
    );
  }

  if (groupName === "nightly" || groupName === "release") {
    const targetDir = targetRoot ?? getBuckyosRoot();
    const envHosts = groupName === "nightly"
      ? {
        web3BridgeHost: "web3.buckyos.io",
        snBaseHost: "buckyos.io",
        bnsHost: "bns.buckyos.io",
      }
      : {
        web3BridgeHost: "web3.buckyos.ai",
        snBaseHost: "buckyos.ai",
        bnsHost: "bns.buckyos.ai",
      };
    console.log(`${groupName} mode, write basic configs to ${targetDir}`);
    makeGlobalEnvConfig(
      targetDir,
      envHosts.web3BridgeHost,
      envHosts.bnsHost,
      DEFAULT_TRUST_DID,
      true,
      envHosts.snBaseHost,
    );
    return;
  }

  const params = getParamsFromGroupName(groupName);
  const targetDir = targetRoot ?? getBuckyosRoot();
  const resolvedCaDir = caDir ?? ensureDir(path.join(ENV_ROOT_DIR, "ca"));

  console.log(
    `############ make config for group name: ${groupName} #########################`,
  );
  console.log(`rootfs dir : ${targetDir}`);
  console.log(`group      : ${groupName}`);
  console.log(`username   : ${params.username}`);
  console.log(`zone       : ${params.zone_id}`);
  console.log(`node       : ${params.node_name}`);
  console.log(`web3_bridge: ${params.web3_bridge}`);

  makeGlobalEnvConfig(
    targetDir,
    params.web3_bridge,
    params.bns_host,
    params.trust_did,
    params.force_https,
    params.sn_base_host,
  );
  if (params.preseed_identity === false) {
    makeUnactivatedIdentityConfig(targetDir);
  } else {
    await makeIdentityFiles(targetDir, params, resolvedCaDir);
  }
  applyDevBootTemplateOverride(targetDir, groupName);

  console.log(`config ${groupName} generation finished.`);
}

async function main(): Promise<void> {
  let values: { rootfs?: string; ca?: string; sn_ip?: string; help?: boolean };
  let positionals: string[];
  try {
    const parsed = parseArgs({
      args: Deno.args,
      options: {
        rootfs: { type: "string" },
        ca: { type: "string" },
        sn_ip: { type: "string" },
        help: { type: "boolean", short: "h" },
      },
      allowPositionals: true,
    }) as {
      values: { rootfs?: string; ca?: string; sn_ip?: string; help?: boolean };
      positionals: string[];
    };
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

  const groupName = positionals[0];
  if (values.sn_ip && groupName !== "sn" && groupName !== "sn_server") {
    console.error(
      "argument error: --sn_ip is only supported by cyfs-gateway's src/make_sn_config.ts",
    );
    printUsage();
    Deno.exit(1);
  }

  try {
    assertProvisionRuntime();
  } catch (e) {
    console.error(
      `runtime check failed: ${e instanceof Error ? e.message : e}`,
    );
    console.error(
      "make_config.ts requires Deno >= 2.2 (or Node >= 22.13 with a loader for the websdk import)",
    );
    Deno.exit(1);
  }

  try {
    await makeConfigByGroupName(groupName, values.rootfs, values.ca);
  } catch (e) {
    console.error(
      `config generation failed: ${e instanceof Error ? e.message : e}`,
    );
    printUsage();
    Deno.exit(1);
  }
}

if (import.meta.main) {
  await main();
}
