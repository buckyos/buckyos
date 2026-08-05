#!/usr/bin/env -S deno run --allow-all

import * as fs from "node:fs";
import * as net from "node:net";
import * as os from "node:os";
import * as path from "node:path";
import { createHash, generateKeyPairSync, randomBytes } from "node:crypto";
import { parseArgs } from "node:util";
import { assertProvisionRuntime } from "buckyos/provision";
import type { OODGroupParams } from "./devenv_config.ts";
import {
  buildUserEnv,
  copyIdentityOutputs,
  getBuckyosRoot,
  type LocalDeviceIdentityFiles,
  type ProvisionKeyPair,
} from "./make_config.ts";

const DEFAULT_RTCP_PORT = 2980;
const DOCUMENT_VALIDITY_SECONDS = 3600 * 24 * 365 * 5;
const DEVICE_NAME = "ood1";
const ZONE_TXT_RECORD_FILE_NAME = "zone_txt_record.json";

interface JsonObject {
  [key: string]: unknown;
}

export interface OfflineActivationOptions {
  rootDir: string;
  domain: string;
  ownerName: string;
  adminPassword: string;
  ownerKeyBackupPath: string;
  rtcpPort?: number;
  guestAccess?: boolean;
  publicIp?: string;
  ownerKeyPair?: ProvisionKeyPair;
  deviceKeyPair?: ProvisionKeyPair;
}

export interface DnsRecord {
  type: "A" | "AAAA" | "TXT";
  name: string;
  value: string;
}

export interface OfflineActivationResult {
  rootDir: string;
  ownerDid: string;
  zoneDid: string;
  deviceDid: string;
  accessHostname: string;
  ownerKeyBackupPath: string;
  dnsRecords: DnsRecord[];
}

function normalizeDomain(value: string): string {
  return value.trim().toLowerCase().replace(/\.$/, "");
}

export function validateDomain(value: string): string {
  const domain = normalizeDomain(value);
  if (
    domain.length === 0 ||
    domain.length > 253 ||
    !domain.includes(".") ||
    domain.split(".").some((label) =>
      label.length === 0 ||
      label.length > 63 ||
      label.startsWith("-") ||
      label.endsWith("-") ||
      !/^[a-z0-9-]+$/.test(label)
    )
  ) {
    throw new Error(`invalid did:web domain: ${value}`);
  }
  return domain;
}

export function validateOwnerName(value: string): string {
  const name = value.trim().toLowerCase();
  if (
    name.length === 0 ||
    name.length > 63 ||
    !/^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$/.test(name)
  ) {
    throw new Error(
      "owner name must be a lowercase DNS label (letters, digits, and hyphens)",
    );
  }
  return name;
}

export function hashAdminPassword(ownerName: string, password: string): string {
  return createHash("sha256")
    .update(password + ownerName + ".buckyos", "utf8")
    .digest("base64");
}

function generateEd25519KeyPair(): ProvisionKeyPair {
  const { privateKey, publicKey } = generateKeyPairSync("ed25519");
  const jwk = publicKey.export({ format: "jwk" });
  if (typeof jwk.x !== "string" || jwk.x.length === 0) {
    throw new Error("generated Ed25519 public key has no x coordinate");
  }
  return {
    privateKeyPem: privateKey.export({ format: "pem", type: "pkcs8" })
      .toString(),
    publicKeyX: jwk.x,
  };
}

function readJsonObject(filePath: string): JsonObject {
  const value = JSON.parse(fs.readFileSync(filePath, "utf8"));
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`invalid JSON object: ${filePath}`);
  }
  return value as JsonObject;
}

function writeJson(filePath: string, value: unknown): void {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, JSON.stringify(value, null, 2) + "\n");
}

function replaceExactString(value: unknown, from: string, to: string): unknown {
  if (value === from) return to;
  if (Array.isArray(value)) {
    return value.map((item) => replaceExactString(item, from, to));
  }
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value).map(([key, child]) => [
        key,
        replaceExactString(child, from, to),
      ]),
    );
  }
  return value;
}

function rewriteGeneratedOwnerDid(
  userDir: string,
  nodeDir: string,
  ownerName: string,
  ownerDid: string,
): void {
  const generatedOwnerDid = `did:bns:${ownerName}`;
  const candidates = [
    path.join(userDir, "user_config.json"),
    path.join(userDir, "zone_config.json"),
    path.join(nodeDir, "node_identity.json"),
  ];
  const visit = (dirPath: string): void => {
    for (const entry of fs.readdirSync(dirPath, { withFileTypes: true })) {
      const entryPath = path.join(dirPath, entry.name);
      if (entry.isDirectory()) visit(entryPath);
      else if (entry.isFile() && entry.name.endsWith(".json")) {
        candidates.push(entryPath);
      }
    }
  };
  visit(nodeDir);
  for (const filePath of new Set(candidates)) {
    const value = readJsonObject(filePath);
    writeJson(filePath, replaceExactString(value, generatedOwnerDid, ownerDid));
  }
}

function refreshGeneratedDocumentTimes(
  userDir: string,
  nodeDir: string,
  domain: string,
): void {
  const iat = Math.floor(Date.now() / 1000);
  const exp = iat + DOCUMENT_VALIDITY_SECONDS;
  for (
    const filePath of [
      path.join(userDir, "user_config.json"),
      path.join(userDir, "zone_config.json"),
      path.join(userDir, `${domain}.zone.json`),
    ]
  ) {
    const document = readJsonObject(filePath);
    document.iat = iat;
    document.exp = exp;
    writeJson(filePath, document);
  }
  const visit = (dirPath: string): void => {
    for (const entry of fs.readdirSync(dirPath, { withFileTypes: true })) {
      const entryPath = path.join(dirPath, entry.name);
      if (entry.isDirectory()) visit(entryPath);
      else if (entry.isFile() && entry.name === "did.json") {
        const document = readJsonObject(entryPath);
        document.iat = iat;
        document.exp = exp;
        writeJson(entryPath, document);
      }
    }
  };
  visit(nodeDir);
}

async function quietly<T>(operation: () => Promise<T> | T): Promise<T> {
  const originalLog = console.log;
  console.log = () => {};
  try {
    return await operation();
  } finally {
    console.log = originalLog;
  }
}

function listFiles(rootDir: string): string[] {
  const files: string[] = [];
  const visit = (dirPath: string): void => {
    for (const entry of fs.readdirSync(dirPath, { withFileTypes: true })) {
      const entryPath = path.join(dirPath, entry.name);
      if (entry.isDirectory()) visit(entryPath);
      else if (entry.isFile()) files.push(entryPath);
      else throw new Error(`unsupported staged file type: ${entryPath}`);
    }
  };
  visit(rootDir);
  return files;
}

function atomicCopyFile(
  source: string,
  destination: string,
  mode: number,
): void {
  fs.mkdirSync(path.dirname(destination), {
    recursive: true,
    mode: destination.includes(`${path.sep}security${path.sep}`)
      ? 0o700
      : 0o755,
  });
  const temporary = path.join(
    path.dirname(destination),
    `.${path.basename(destination)}.active-${randomBytes(6).toString("hex")}`,
  );
  try {
    fs.copyFileSync(source, temporary);
    fs.chmodSync(temporary, mode);
    fs.renameSync(temporary, destination);
  } finally {
    if (fs.existsSync(temporary)) fs.rmSync(temporary);
  }
}

function commitStagedTree(stagedRoot: string, targetRoot: string): void {
  for (const source of listFiles(stagedRoot)) {
    const relative = path.relative(stagedRoot, source);
    if (relative.startsWith("..") || path.isAbsolute(relative)) {
      throw new Error(`staged file escaped activation root: ${source}`);
    }
    const destination = path.join(targetRoot, relative);
    const isPrivate = relative.startsWith(`security${path.sep}`);
    atomicCopyFile(source, destination, isPrivate ? 0o600 : 0o644);
  }
}

function saveOwnerKeyBackup(filePath: string, privateKeyPem: string): void {
  const resolved = path.resolve(filePath);
  if (fs.existsSync(resolved)) {
    throw new Error(`owner key backup already exists: ${resolved}`);
  }
  fs.mkdirSync(path.dirname(resolved), { recursive: true, mode: 0o700 });
  const temporary = `${resolved}.active-${randomBytes(6).toString("hex")}`;
  try {
    fs.writeFileSync(temporary, privateKeyPem, { mode: 0o600, flag: "wx" });
    fs.renameSync(temporary, resolved);
  } finally {
    if (fs.existsSync(temporary)) fs.rmSync(temporary);
  }
}

function buildDnsRecords(
  domain: string,
  publicIp: string | undefined,
  identity: LocalDeviceIdentityFiles,
): DnsRecord[] {
  const records: DnsRecord[] = [];
  if (publicIp) {
    records.push({
      type: net.isIP(publicIp) === 6 ? "AAAA" : "A",
      name: domain,
      value: publicIp,
    });
  }
  const verificationMethods = identity.ownerDocument.verificationMethod;
  const mainKey = Array.isArray(verificationMethods)
    ? verificationMethods.find((item) =>
      item && typeof item === "object" &&
      (item as JsonObject).id === "#main_key"
    ) as JsonObject | undefined
    : undefined;
  const publicKeyJwk = mainKey?.publicKeyJwk as JsonObject | undefined;
  if (typeof publicKeyJwk?.x !== "string" || publicKeyJwk.x.length === 0) {
    throw new Error("OwnerDocument has no #main_key Ed25519 x value");
  }
  records.push(
    { type: "TXT", name: domain, value: `BOOT=${identity.bootDocumentJwt};` },
    { type: "TXT", name: domain, value: `PKX=${publicKeyJwk.x};` },
    { type: "TXT", name: domain, value: `DEV=${identity.deviceMiniDocJwt};` },
  );
  return records;
}

function requireUnactivatedRoot(rootDir: string): void {
  if (!fs.existsSync(rootDir) || !fs.statSync(rootDir).isDirectory()) {
    throw new Error(`BUCKYOS_ROOT does not exist: ${rootDir}`);
  }
  for (const requiredDir of ["bin", "etc"]) {
    const dirPath = path.join(rootDir, requiredDir);
    if (!fs.existsSync(dirPath) || !fs.statSync(dirPath).isDirectory()) {
      throw new Error(`installed BuckyOS directory is missing: ${dirPath}`);
    }
  }
  for (
    const marker of [
      "node_identity.json",
      "start_config.json",
      "zone_document.jwt",
    ]
  ) {
    const markerPath = path.join(rootDir, "etc", marker);
    if (fs.existsSync(markerPath)) {
      throw new Error(
        `BuckyOS is already or partially activated: ${markerPath}`,
      );
    }
  }
}

export async function activateOffline(
  options: OfflineActivationOptions,
): Promise<OfflineActivationResult> {
  assertProvisionRuntime();
  const rootDir = path.resolve(options.rootDir);
  const domain = validateDomain(options.domain);
  const ownerName = validateOwnerName(options.ownerName);
  const rtcpPort = options.rtcpPort ?? DEFAULT_RTCP_PORT;
  if (!Number.isInteger(rtcpPort) || rtcpPort < 1 || rtcpPort > 65535) {
    throw new Error(`invalid RTCP port: ${rtcpPort}`);
  }
  if (options.adminPassword.length < 8) {
    throw new Error(
      "administrator password must contain at least 8 characters",
    );
  }
  const publicIp = options.publicIp?.trim() || undefined;
  if (publicIp && net.isIP(publicIp) === 0) {
    throw new Error(`invalid public IP address: ${publicIp}`);
  }
  requireUnactivatedRoot(rootDir);

  const ownerDid = `did:web:${domain}`;
  const ownerKeyPair = options.ownerKeyPair ?? generateEd25519KeyPair();
  const deviceKeyPair = options.deviceKeyPair ?? generateEd25519KeyPair();
  const workDir = fs.mkdtempSync(path.join(os.tmpdir(), "buckyos-active-"));
  try {
    const params: OODGroupParams = {
      username: ownerName,
      zone_id: domain,
      node_name: DEVICE_NAME,
      netid: "wan",
      rtcp_port: rtcpPort,
      sn_base_host: "",
      bns_host: "",
      web3_bridge: "",
      trust_did: [],
      force_https: false,
      ca_name: "",
      sn_account: false,
    };
    const generatedRoot = path.join(workDir, "generated");
    const userDir = await quietly(() =>
      buildUserEnv(params, generatedRoot, {
        ownerKeyPair,
        deviceKeyPair,
      })
    );
    const nodeDir = path.join(userDir, DEVICE_NAME);
    rewriteGeneratedOwnerDid(userDir, nodeDir, ownerName, ownerDid);
    refreshGeneratedDocumentTimes(userDir, nodeDir, domain);

    const stagedRoot = path.join(workDir, "staged-root");
    fs.mkdirSync(stagedRoot, { recursive: true });
    const identity = await quietly(() =>
      copyIdentityOutputs(
        userDir,
        nodeDir,
        stagedRoot,
        params,
      )
    );
    if (identity.zoneDid !== ownerDid) {
      throw new Error(
        `generated zone DID mismatch: expected ${ownerDid}, got ${identity.zoneDid}`,
      );
    }

    const startConfigPath = path.join(stagedRoot, "etc", "start_config.json");
    const startConfig = readJsonObject(startConfigPath);
    startConfig.admin_password_hash = hashAdminPassword(
      ownerName,
      options.adminPassword,
    );
    startConfig.guest_access = options.guestAccess ?? false;
    startConfig.friend_passcode = "";
    startConfig.enabled_features = {};
    startConfig.ai_provider_config = {};
    startConfig.jarvis_msg_tunnel_config = {};
    writeJson(startConfigPath, startConfig);

    const dnsRecords = buildDnsRecords(domain, publicIp, identity);
    writeJson(path.join(stagedRoot, "etc", ZONE_TXT_RECORD_FILE_NAME), {
      hostname: domain,
      records: dnsRecords,
    });

    saveOwnerKeyBackup(options.ownerKeyBackupPath, ownerKeyPair.privateKeyPem);
    commitStagedTree(stagedRoot, rootDir);

    return {
      rootDir,
      ownerDid,
      zoneDid: identity.zoneDid,
      deviceDid: identity.deviceDid,
      accessHostname: domain,
      ownerKeyBackupPath: path.resolve(options.ownerKeyBackupPath),
      dnsRecords,
    };
  } finally {
    fs.rmSync(workDir, { recursive: true, force: true });
  }
}

function defaultOwnerName(domain: string): string {
  const firstLabel = domain.split(".")[0];
  try {
    return validateOwnerName(firstLabel);
  } catch {
    return "admin";
  }
}

function defaultOwnerKeyBackupPath(domain: string): string {
  return path.join(os.homedir(), ".buckycli", "owner-keys", `${domain}.pem`);
}

function ask(message: string, defaultValue?: string): string {
  const suffix = defaultValue ? ` [${defaultValue}]` : "";
  const value = globalThis.prompt(`${message}${suffix}:`)?.trim() ?? "";
  return value || defaultValue || "";
}

async function askSecret(message: string): Promise<string> {
  if (!Deno.stdin.isTerminal()) {
    throw new Error("password input requires a terminal or --password-stdin");
  }
  await Deno.stdout.write(new TextEncoder().encode(`${message}: `));
  const bytes: number[] = [];
  Deno.stdin.setRaw(true);
  try {
    const buffer = new Uint8Array(1);
    while (true) {
      const count = await Deno.stdin.read(buffer);
      if (count === null) throw new Error("password input closed");
      const byte = buffer[0];
      if (byte === 3) throw new Error("cancelled");
      if (byte === 13 || byte === 10) break;
      if (byte === 8 || byte === 127) {
        if (bytes.length > 0) {
          bytes.pop();
          await Deno.stdout.write(new TextEncoder().encode("\b \b"));
        }
        continue;
      }
      bytes.push(byte);
      await Deno.stdout.write(new TextEncoder().encode("*"));
    }
  } finally {
    Deno.stdin.setRaw(false);
    await Deno.stdout.write(new TextEncoder().encode("\n"));
  }
  return new TextDecoder().decode(new Uint8Array(bytes));
}

async function readPasswordFromStdin(): Promise<string> {
  const chunks: Uint8Array[] = [];
  for await (const chunk of Deno.stdin.readable) chunks.push(chunk);
  const size = chunks.reduce((sum, chunk) => sum + chunk.length, 0);
  const bytes = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.length;
  }
  return new TextDecoder().decode(bytes).split(/\r?\n/, 1)[0];
}

function printUsage(log: (message?: unknown) => void = console.log): void {
  log("usage: ./src/active.ts [options]");
  log(
    "  --domain <host>             required did:web domain (prompted if omitted)",
  );
  log("  --owner-name <name>         local administrator/OwnerDocument name");
  log("  --public-ip <ip>            fixed public IP for A/AAAA instructions");
  log("  --rtcp-port <port>          RTCP port, default 2980");
  log("  --root <dir>                BUCKYOS_ROOT override");
  log("  --owner-key-backup <file>   Owner private-key recovery backup");
  log("  --guest-access              enable guest access");
  log("  --password-stdin            read administrator password from stdin");
  log(
    "  -y, --yes                   accept defaults and skip final confirmation",
  );
  log("  -h, --help                  show this help");
}

async function main(): Promise<void> {
  let parsed;
  try {
    parsed = parseArgs({
      args: Deno.args,
      options: {
        domain: { type: "string" },
        "owner-name": { type: "string" },
        "public-ip": { type: "string" },
        "rtcp-port": { type: "string" },
        root: { type: "string" },
        "owner-key-backup": { type: "string" },
        "guest-access": { type: "boolean" },
        "password-stdin": { type: "boolean" },
        yes: { type: "boolean", short: "y" },
        help: { type: "boolean", short: "h" },
      },
      allowPositionals: false,
      strict: true,
    });
  } catch (error) {
    console.error(
      `argument error: ${error instanceof Error ? error.message : error}`,
    );
    printUsage(console.error);
    Deno.exit(1);
  }
  const values = parsed.values;
  if (values.help) {
    printUsage();
    return;
  }

  try {
    const rootDir = path.resolve(values.root ?? getBuckyosRoot());
    const rawDomain = values.domain ??
      ask("did:web domain (fixed public WAN host)");
    const domain = validateDomain(rawDomain);
    const suggestedOwner = defaultOwnerName(domain);
    const ownerName = validateOwnerName(
      values["owner-name"] ??
        (values.yes
          ? suggestedOwner
          : ask("Local administrator name", suggestedOwner)),
    );
    const rawPort = values["rtcp-port"] ??
      (values.yes
        ? String(DEFAULT_RTCP_PORT)
        : ask("RTCP port", String(DEFAULT_RTCP_PORT)));
    const rtcpPort = Number(rawPort);
    if (!Number.isInteger(rtcpPort)) {
      throw new Error(`invalid RTCP port: ${rawPort}`);
    }
    const publicIp = (values["public-ip"] ??
      (values.yes ? undefined : ask("Fixed public IP (optional)"))) ||
      undefined;
    const suggestedBackup = defaultOwnerKeyBackupPath(domain);
    const ownerKeyBackupPath = path.resolve(
      values["owner-key-backup"] ??
        (values.yes
          ? suggestedBackup
          : ask("Owner key backup file", suggestedBackup)),
    );
    const guestAccess = values["guest-access"] ??
      (!values.yes && globalThis.confirm("Enable guest access?"));
    const adminPassword = values["password-stdin"]
      ? await readPasswordFromStdin()
      : await askSecret("Administrator password");
    if (!values["password-stdin"]) {
      const confirmation = await askSecret("Confirm administrator password");
      if (adminPassword !== confirmation) {
        throw new Error("passwords do not match");
      }
    }

    console.log("\nActivation summary");
    console.log(`  BUCKYOS_ROOT : ${rootDir}`);
    console.log(`  Owner/Zone   : did:web:${domain}`);
    console.log(`  Admin        : ${ownerName}`);
    console.log(`  Network      : wan (no SN, no BNS)`);
    console.log(`  RTCP port    : ${rtcpPort}`);
    console.log(`  Owner backup : ${ownerKeyBackupPath}`);
    if (!values.yes && !globalThis.confirm("Write this activation to disk?")) {
      console.log("Activation cancelled.");
      return;
    }

    const result = await activateOffline({
      rootDir,
      domain,
      ownerName,
      adminPassword,
      ownerKeyBackupPath,
      rtcpPort,
      guestAccess,
      publicIp,
    });
    console.log("\nActivation completed.");
    console.log(`Owner/Zone DID: ${result.zoneDid}`);
    console.log(`Device DID:     ${result.deviceDid}`);
    console.log(`Owner key:      ${result.ownerKeyBackupPath}`);
    console.log("\nConfigure these DNS records before public access:");
    for (const record of result.dnsRecords) {
      console.log(`  ${record.type} ${record.name} ${record.value}`);
    }
    if (!publicIp) {
      console.log(`  A/AAAA ${domain} <this node's fixed public IP>`);
    }
    console.log(
      `\nThe records are also saved in ${
        path.join(result.rootDir, "etc", ZONE_TXT_RECORD_FILE_NAME)
      }.`,
    );
    console.log(
      "Restart BuckyOS to leave activation mode and boot the new Zone.",
    );
    console.log(
      "TLS/ACME is not provided by SN in this mode; configure it on the public gateway.",
    );
  } catch (error) {
    console.error(
      `activation failed: ${error instanceof Error ? error.message : error}`,
    );
    Deno.exit(1);
  }
}

if (import.meta.main) {
  await main();
}
