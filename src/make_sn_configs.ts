#!/usr/bin/env -S deno run --allow-all
// make_sn_configs.ts - standalone minimal SN (Super Node) deployment tool.
//
// Design intent:
// This script is the SN-side companion to make_config.ts. It pre-seeds the
// local state that a dev Super Node owns before web3-gateway starts, using only
// the public Web SDK provision surface. That keeps dev setup close to what a
// third-party tool or agent would do when it registers users, devices, and zone
// data through the data-plane API instead of relying on process-private setup
// code.
//
// The user/zone/device cases come from devenv_config.ts. That file is the shared
// user-data seed description; this script materializes the SN-owned view of that
// seed, while make_config.ts materializes the OOD/rootfs-owned view. The boundary
// is intentional: do not duplicate another module's derived config here just to
// make boot pass. If web3-gateway can reconstruct indexes, caches, or dependent
// state from its seed database and protocol data, it should do so lazily at boot.
//
// Iteration direction:
// Keep this script short and owned by the SN/web3-gateway boundary. It may write
// minimal SN service parameters, SN identity material, and seed registrations in
// the SN database. Runtime-derived data, compatibility shims, caches, and state
// that belongs to OOD-side services should live in the owning module's lazy/seed
// initialization path. When a version cannot start from the old seed data, treat
// that as a data-format compatibility signal to review before release.
//
// Runtime: Deno >= 2.2 (node:sqlite). websdk import mapped in src/deno.json.
//
// Usage: deno run --allow-all src/make_sn_configs.ts
//          [--rootfs <dir>] [--ca <dir>] [--sn_ip <ip>]
//          [--sn_base_host <host>] [--env_root <dir>] [--no-preregister]
//
// Output layout:
//   <rootfs>/sn_device_config.json    SN server device config
//   <rootfs>/sn_private_key.pem       device private key (rtcp stack)
//   <rootfs>/params.json              SN service parameters (incl. sn_ip)
//   <rootfs>/sn.sqlite3               SN sqlite database with pre-registered users
//   <rootfs>/sn_token_key/            server JWT signing key directory
//   <rootfs>/fullchain.cert/.pem      TLS cert+key for sn.$base, web3.$base, *.web3.$base
//   <rootfs>/ca/                      dev CA cert+key (client trust install)
//   <rootfs>/sn_server/.buckycli/     sn admin owner config
//   <rootfs>/web3_gateway.yaml        patched when present to load sn.sqlite3
// Still created manually/by app template: website.yaml, local_dns.toml.
//
// User pre-registration: alice.ood1 / bob.ood1 / charlie.ood1 dev zones are
// registered into sn.sqlite3. Their user envs are reused from <env_root> when
// present (so JWTs match rootfs built by make_config.ts) and generated there
// otherwise (DEV_TEST_KEYS are deterministic, so devices still verify).

import * as fs from "node:fs";
import * as path from "node:path";
import * as os from "node:os";
import { DatabaseSync } from "node:sqlite";
import { parseArgs } from "node:util";
import {
  assertProvisionRuntime,
  createCertFromCa,
  createSnConfigs,
  ensureCa,
  registerDeviceToSn,
  registerUserToSn,
} from "buckyos/provision";
import {
  DEFAULT_TRUST_DID,
  ENV_ROOT_DIR,
  getParamsFromGroupName,
} from "./devenv_config.ts";
import { buildUserEnv, ensureDir, makeMachineConfig } from "./make_config.ts";

const DEFAULT_SN_BASE_HOST = "devtests.org";
const DEFAULT_CA_NAME = "buckyos_test_ca";
const PREREGISTER_GROUPS = ["alice.ood1", "bob.ood1", "charlie.ood1"];
const LEGACY_SN_DB_FILE = "sn_db.sqlite3";
const SN_DB_FILE = "sn.sqlite3";
const SN_AUTH_DATA_DIR = "sn_token_key";
const WEB3_GATEWAY_CONFIG_FILE = "web3_gateway.yaml";

type SqliteValue = string | number | bigint | null;
type SqliteRow = Record<string, SqliteValue>;

function printUsage(log: (message?: unknown) => void = console.error): void {
  log(
    "usage: make_sn_configs.ts [--rootfs <dir>] [--ca <dir>] [--sn_ip <ip>] [--sn_base_host <host>] [--env_root <dir>] [--no-preregister]",
  );
}

function defaultTargetRoot(): string {
  if (os.platform() === "win32") {
    const appData = Deno.env.get("APPDATA") ?? Deno.env.get("USERPROFILE") ??
      ".";
    return path.join(appData, "web3-gateway");
  }
  return "/opt/web3-gateway";
}

function getLocalIp(): string {
  for (const list of Object.values(os.networkInterfaces())) {
    for (const item of list ?? []) {
      if (!item.internal && item.family === "IPv4") {
        return item.address;
      }
    }
  }
  return "127.0.0.1";
}

function writeJson(file: string, data: unknown): void {
  fs.writeFileSync(file, `${JSON.stringify(data, null, 2)}\n`);
  console.log(`# Write file: ${file}`);
}

function readJson(file: string): Record<string, unknown> {
  return JSON.parse(fs.readFileSync(file, "utf8"));
}

function sqliteRows(db: DatabaseSync, sql: string): SqliteRow[] {
  return db.prepare(sql).all() as SqliteRow[];
}

function ensureColumn(
  db: DatabaseSync,
  table: string,
  column: string,
  definition: string,
): void {
  const rows = sqliteRows(db, `PRAGMA table_info(${table})`);
  const exists = rows.some((row) => row.name === column);
  if (!exists) {
    db.exec(`ALTER TABLE ${table} ADD COLUMN ${column} ${definition}`);
  }
}

function tableExists(db: DatabaseSync, table: string): boolean {
  const rows = db.prepare(
    "SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?",
  ).all(table) as SqliteRow[];
  return rows.length > 0;
}

function migrateLegacyUserAuthTable(db: DatabaseSync): void {
  if (!tableExists(db, "user_auth_v2")) {
    return;
  }
  db.exec(`
    INSERT OR IGNORE INTO user_auth
      (username, password_hash, password_salt, password_algo,
       created_at, updated_at, last_login_at)
    SELECT
      username, password_hash, password_salt, password_algo,
      created_at, updated_at, last_login_at
    FROM user_auth_v2;

    DROP TABLE user_auth_v2;
  `);
}

function initializeSnDatabaseSchema(db: DatabaseSync): void {
  db.exec(`
    PRAGMA foreign_keys = ON;

    CREATE TABLE IF NOT EXISTS activation_codes (
      code TEXT PRIMARY KEY,
      used INTEGER NOT NULL DEFAULT 0
    );

    CREATE TABLE IF NOT EXISTS users (
      username TEXT PRIMARY KEY,
      state TEXT NOT NULL DEFAULT 'active',
      bns_name TEXT,
      public_key TEXT NOT NULL DEFAULT '',
      activation_code TEXT,
      owner_key_ref TEXT,
      zone_config TEXT NOT NULL DEFAULT '',
      self_cert INTEGER NOT NULL DEFAULT 0,
      user_domain TEXT,
      sn_ips TEXT,
      created_at INTEGER NOT NULL DEFAULT 0,
      updated_at INTEGER NOT NULL DEFAULT 0,
      last_login_at INTEGER NULL
    );

    CREATE TABLE IF NOT EXISTS user_auth (
      username TEXT PRIMARY KEY,
      password_hash TEXT NOT NULL,
      password_salt TEXT NOT NULL,
      password_algo TEXT NOT NULL,
      created_at INTEGER NOT NULL,
      updated_at INTEGER NOT NULL,
      last_login_at INTEGER NULL
    );

    CREATE TABLE IF NOT EXISTS user_domain_history (
      domain TEXT PRIMARY KEY,
      owner TEXT NOT NULL,
      created_at INTEGER NOT NULL
    );

    CREATE TABLE IF NOT EXISTS user_domain_bindings (
      domain TEXT PRIMARY KEY,
      owner TEXT NOT NULL,
      state TEXT NOT NULL,
      pkx TEXT NOT NULL,
      pkx_record_name TEXT NOT NULL,
      verified_at INTEGER NULL,
      created_at INTEGER NOT NULL,
      updated_at INTEGER NOT NULL
    );

    CREATE INDEX IF NOT EXISTS idx_user_domain_bindings_owner_state
      ON user_domain_bindings (owner, state);

    CREATE TABLE IF NOT EXISTS zone_info (
      username TEXT PRIMARY KEY,
      bns_name TEXT NOT NULL,
      zone TEXT NULL,
      relay_sn TEXT NULL,
      self_cert INTEGER NOT NULL DEFAULT 0,
      cert_checked_at INTEGER NULL,
      cert_expires_at INTEGER NULL,
      sn_ips TEXT NULL,
      source_version TEXT NULL,
      updated_at INTEGER NOT NULL
    );

    CREATE TABLE IF NOT EXISTS account_sessions (
      session_id TEXT PRIMARY KEY,
      username TEXT NOT NULL,
      token_aud TEXT NOT NULL,
      state TEXT NOT NULL,
      issued_at INTEGER NOT NULL,
      expires_at INTEGER NOT NULL,
      revoked_at INTEGER NULL
    );

    CREATE INDEX IF NOT EXISTS idx_account_sessions_username_state
      ON account_sessions (username, state);

    CREATE TABLE IF NOT EXISTS devices (
      owner TEXT NOT NULL,
      device_name TEXT NOT NULL,
      did TEXT PRIMARY KEY,
      ip TEXT NOT NULL,
      description TEXT NOT NULL,
      mini_config_jwt TEXT NOT NULL,
      created_at INTEGER NOT NULL,
      updated_at INTEGER NOT NULL,
      UNIQUE(owner, device_name)
    );

    CREATE TABLE IF NOT EXISTS user_dns_records (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      owner TEXT NOT NULL,
      domain TEXT NOT NULL,
      record_type TEXT NOT NULL,
      record TEXT NOT NULL,
      ttl INTEGER NOT NULL,
      created_at INTEGER NOT NULL,
      updated_at INTEGER NOT NULL
    );

    CREATE UNIQUE INDEX IF NOT EXISTS idx_user_domain_record_type
      ON user_dns_records (owner, domain, record_type);

    CREATE INDEX IF NOT EXISTS user_dns_records_domain_index
      ON user_dns_records (domain, id);

    CREATE TABLE IF NOT EXISTS did_documents (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      obj_id TEXT NOT NULL,
      owner_user TEXT NOT NULL,
      obj_name TEXT NOT NULL,
      did_document TEXT NOT NULL,
      doc_type TEXT NULL,
      update_time INTEGER NOT NULL
    );

    CREATE INDEX IF NOT EXISTS idx_did_documents_owner_obj
      ON did_documents (owner_user, obj_name, update_time);

    CREATE TABLE IF NOT EXISTS device_indexes (
      did TEXT PRIMARY KEY,
      zone TEXT NOT NULL,
      device_name TEXT NOT NULL,
      device_role TEXT NOT NULL DEFAULT 'unknown',
      created_at INTEGER NOT NULL,
      updated_at INTEGER NOT NULL,
      UNIQUE(zone, device_name)
    );

    CREATE TABLE IF NOT EXISTS device_runtime_states (
      did TEXT PRIMARY KEY,
      state TEXT NOT NULL,
      reported_ip TEXT NULL,
      reported_ips TEXT NULL,
      from_ip TEXT NULL,
      wan_ip TEXT NULL,
      lan_ips TEXT NULL,
      nat_type TEXT NOT NULL DEFAULT 'unknown',
      is_wan_device INTEGER NOT NULL DEFAULT 0,
      last_seen_at INTEGER NULL,
      last_report_at INTEGER NULL,
      expires_at INTEGER NULL,
      report_seq INTEGER NULL,
      raw_report TEXT NULL,
      created_at INTEGER NOT NULL,
      updated_at INTEGER NOT NULL,
      FOREIGN KEY(did) REFERENCES device_indexes(did)
    );

    CREATE TABLE IF NOT EXISTS device_endpoints (
      did TEXT NOT NULL,
      endpoint_id TEXT NOT NULL,
      protocol TEXT NOT NULL,
      host TEXT NOT NULL,
      port INTEGER NULL,
      scope TEXT NOT NULL DEFAULT 'unknown',
      priority INTEGER NOT NULL DEFAULT 100,
      source TEXT NOT NULL,
      state TEXT NOT NULL,
      last_seen_at INTEGER NULL,
      expires_at INTEGER NULL,
      created_at INTEGER NOT NULL,
      updated_at INTEGER NOT NULL,
      PRIMARY KEY(did, endpoint_id),
      FOREIGN KEY(did) REFERENCES device_indexes(did)
    );

    CREATE TABLE IF NOT EXISTS device_state_events (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      did TEXT NOT NULL,
      event_type TEXT NOT NULL,
      old_state TEXT NULL,
      new_state TEXT NULL,
      reason TEXT NULL,
      event_at INTEGER NOT NULL,
      detail TEXT NULL
    );

    CREATE INDEX IF NOT EXISTS idx_device_indexes_zone_name
      ON device_indexes(zone, device_name);

    CREATE INDEX IF NOT EXISTS idx_device_runtime_state_expires
      ON device_runtime_states(state, expires_at);

    CREATE INDEX IF NOT EXISTS idx_device_endpoints_did_state_priority
      ON device_endpoints(did, state, priority);

    CREATE INDEX IF NOT EXISTS idx_device_state_events_did_time
      ON device_state_events(did, event_at);

    CREATE TABLE IF NOT EXISTS relay_nodes (
      relay_id TEXT PRIMARY KEY,
      relay_sn TEXT NOT NULL UNIQUE,
      public_host TEXT NOT NULL,
      http_endpoint TEXT NULL,
      rtcp_endpoint TEXT NULL,
      region TEXT NULL,
      isp TEXT NULL,
      tags TEXT NULL,
      capabilities TEXT NOT NULL,
      status TEXT NOT NULL,
      capacity_score INTEGER NOT NULL DEFAULT 100,
      current_load INTEGER NOT NULL DEFAULT 0,
      last_heartbeat_at INTEGER NULL,
      drain_until INTEGER NULL,
      created_at INTEGER NOT NULL,
      updated_at INTEGER NOT NULL
    );

    CREATE TABLE IF NOT EXISTS relay_assignments (
      zone TEXT PRIMARY KEY,
      relay_id TEXT NOT NULL,
      relay_sn TEXT NOT NULL,
      state TEXT NOT NULL,
      source TEXT NOT NULL,
      reason TEXT NULL,
      generation INTEGER NOT NULL,
      backup_relay_id TEXT NULL,
      sticky_until INTEGER NULL,
      lease_expires_at INTEGER NULL,
      migrated_from TEXT NULL,
      migration_deadline INTEGER NULL,
      source_version TEXT NULL,
      created_at INTEGER NOT NULL,
      updated_at INTEGER NOT NULL
    );

    CREATE INDEX IF NOT EXISTS idx_relay_assignments_relay
      ON relay_assignments(relay_id, state);

    CREATE TABLE IF NOT EXISTS relay_admission_events (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      request_id TEXT NULL,
      relay_id TEXT NOT NULL,
      zone TEXT NOT NULL,
      device_name TEXT NULL,
      did TEXT NULL,
      decision TEXT NOT NULL,
      reason TEXT NOT NULL,
      expected_relay_sn TEXT NULL,
      assignment_generation INTEGER NULL,
      observed_ip TEXT NULL,
      created_at INTEGER NOT NULL
    );

    CREATE INDEX IF NOT EXISTS idx_relay_admission_events_zone_time
      ON relay_admission_events(zone, created_at);
  `);

  ensureColumn(db, "users", "bns_name", "TEXT");
  ensureColumn(db, "users", "owner_key_ref", "TEXT");
  ensureColumn(db, "users", "created_at", "INTEGER NOT NULL DEFAULT 0");
  ensureColumn(db, "users", "updated_at", "INTEGER NOT NULL DEFAULT 0");
  ensureColumn(db, "users", "last_login_at", "INTEGER NULL");
  migrateLegacyUserAuthTable(db);
}

function backfillSnDerivedTables(db: DatabaseSync): void {
  const now = Math.floor(Date.now() / 1000);
  db.prepare(`
    INSERT OR IGNORE INTO user_domain_history (domain, owner, created_at)
    SELECT lower(trim(user_domain)), username, ?
    FROM users
    WHERE user_domain IS NOT NULL AND trim(user_domain) != ''
  `).run(now);

  db.prepare(`
    INSERT OR IGNORE INTO zone_info
      (username, bns_name, zone, relay_sn, self_cert, cert_checked_at,
       cert_expires_at, sn_ips, source_version, updated_at)
    SELECT
      username,
      COALESCE(NULLIF(bns_name, ''), username),
      CASE WHEN trim(COALESCE(zone_config, '')) = '' THEN NULL ELSE zone_config END,
      NULL,
      COALESCE(self_cert, 0),
      NULL,
      NULL,
      sn_ips,
      NULL,
      CASE WHEN COALESCE(updated_at, 0) > 0 THEN updated_at ELSE ? END
    FROM users
  `).run(now);

  const devices = sqliteRows(
    db,
    "SELECT owner, device_name, did, ip, description, created_at, updated_at FROM devices",
  );
  const insertIndex = db.prepare(`
    INSERT INTO device_indexes
      (did, zone, device_name, device_role, created_at, updated_at)
    VALUES (?, ?, ?, ?, ?, ?)
    ON CONFLICT(did) DO UPDATE SET
      zone = excluded.zone,
      device_name = excluded.device_name,
      device_role = excluded.device_role,
      updated_at = excluded.updated_at
  `);
  const insertState = db.prepare(`
    INSERT INTO device_runtime_states
      (did, state, reported_ip, reported_ips, from_ip, wan_ip, lan_ips, nat_type,
       is_wan_device, last_seen_at, last_report_at, expires_at, report_seq,
       raw_report, created_at, updated_at)
    VALUES (?, 'offline', NULL, '[]', NULL, NULL, '[]', 'unknown', 0,
            NULL, NULL, NULL, NULL, NULL, ?, ?)
    ON CONFLICT(did) DO UPDATE SET
      state = 'offline',
      reported_ip = NULL,
      reported_ips = '[]',
      from_ip = NULL,
      wan_ip = NULL,
      lan_ips = '[]',
      nat_type = 'unknown',
      is_wan_device = 0,
      last_seen_at = NULL,
      last_report_at = NULL,
      expires_at = NULL,
      report_seq = NULL,
      raw_report = NULL,
      updated_at = excluded.updated_at
  `);
  const deleteProvisionedEndpoint = db.prepare(`
    DELETE FROM device_endpoints
    WHERE did = ? AND endpoint_id = 'provisioned' AND source = 'admin'
  `);
  const hasDeviceReportEndpoint = db.prepare(`
    SELECT 1 FROM device_endpoints
    WHERE did = ? AND source = 'device_report'
    LIMIT 1
  `);

  for (const device of devices) {
    const did = String(device.did ?? "");
    const owner = String(device.owner ?? "");
    const deviceName = String(device.device_name ?? "");
    if (!did || !owner || !deviceName) {
      continue;
    }
    const createdAt = Number(device.created_at ?? 0) || now;
    const updatedAt = Number(device.updated_at ?? 0) || now;
    const role = deviceName === "ood1" ? "ood" : "normal";

    insertIndex.run(did, owner, deviceName, role, createdAt, updatedAt);
    deleteProvisionedEndpoint.run(did);
    if (!hasDeviceReportEndpoint.get(did)) {
      insertState.run(did, createdAt, updatedAt);
    }
  }
}

function syncSnDatabase(dbPath: string): void {
  const db = new DatabaseSync(dbPath);
  try {
    initializeSnDatabaseSchema(db);
    backfillSnDerivedTables(db);
  } finally {
    db.close();
  }
}

function replaceGeneratedSnDb(targetDir: string): string {
  const legacyDbPath = path.join(targetDir, LEGACY_SN_DB_FILE);
  const snDbPath = path.join(targetDir, SN_DB_FILE);
  if (!fs.existsSync(legacyDbPath)) {
    throw new Error(`generated SN database not found: ${legacyDbPath}`);
  }
  for (const suffix of ["", "-shm", "-wal"]) {
    fs.rmSync(`${snDbPath}${suffix}`, { force: true });
  }
  fs.renameSync(legacyDbPath, snDbPath);
  for (const suffix of ["-shm", "-wal"]) {
    const legacySidecar = `${legacyDbPath}${suffix}`;
    if (fs.existsSync(legacySidecar)) {
      fs.renameSync(legacySidecar, `${snDbPath}${suffix}`);
    }
  }
  console.log(`Move database: ${LEGACY_SN_DB_FILE} -> ${SN_DB_FILE}`);
  syncSnDatabase(snDbPath);
  return snDbPath;
}

function readStagedParams(targetDir: string): Record<string, unknown> {
  const paramsPath = path.join(targetDir, "params.json");
  if (!fs.existsSync(paramsPath)) {
    return {};
  }
  try {
    const json = readJson(paramsPath);
    return (json.params && typeof json.params === "object")
      ? json.params as Record<string, unknown>
      : {};
  } catch (err) {
    console.warn(`read staged params.json failed, ignore: ${err}`);
    return {};
  }
}

function updateParamsJson(
  targetDir: string,
  snDbPath: string,
  authDataDir: string,
  stagedParams: Record<string, unknown>,
): void {
  const paramsPath = path.join(targetDir, "params.json");
  const json = readJson(paramsPath);
  const params = (json.params && typeof json.params === "object")
    ? json.params as Record<string, unknown>
    : {};
  // createSnConfigs 重写 params.json 时会丢掉仓库模板新增的参数
  // (例如 bns_server_url),这里把 staged(仓库)里有而生成结果缺失的
  // key 补回来;身份类参数以生成结果为准。
  for (const [key, value] of Object.entries(stagedParams)) {
    if (!(key in params)) {
      console.log(`# params.json: restore staged param ${key}`);
      params[key] = value;
    }
  }
  params.sn_db_path = snDbPath;
  delete params.sn_v2_auth_data_dir;
  params.sn_auth_data_dir = authDataDir;
  json.params = params;
  writeJson(paramsPath, json);
}

function leadingSpaces(line: string): number {
  const match = line.match(/^ */);
  return match ? match[0].length : 0;
}

function patchWeb3GatewayConfigText(text: string): string | null {
  const lines = text.split(/\r?\n/);
  const start = lines.findIndex((line) => line.trim() === "web3_sn:");
  if (start < 0) {
    return null;
  }

  const indent = leadingSpaces(lines[start]);
  let end = start + 1;
  while (end < lines.length) {
    const line = lines[end];
    if (
      line.trim() !== "" &&
      !line.trimStart().startsWith("#") &&
      leadingSpaces(line) <= indent
    ) {
      break;
    }
    end++;
  }

  const childIndent = indent + 2;
  const nestedIndent = childIndent + 2;
  const block = lines.slice(start + 1, end);
  const filtered: string[] = [];
  for (let i = 0; i < block.length; i++) {
    const line = block[i];
    const trim = line.trim();
    const lineIndent = leadingSpaces(line);
    if (
      lineIndent === childIndent &&
      (trim.startsWith("v2_auth_data_dir:") ||
        trim.startsWith("auth_data_dir:") ||
        trim.startsWith("db_type:") ||
        trim.startsWith("db_path:"))
    ) {
      continue;
    }
    if (lineIndent === childIndent && trim === "db_params:") {
      i++;
      while (i < block.length && leadingSpaces(block[i]) > childIndent) {
        i++;
      }
      i--;
      continue;
    }
    filtered.push(line);
  }

  const insertLines = [
    `${" ".repeat(childIndent)}auth_data_dir: "{{sn_auth_data_dir}}"`,
    `${" ".repeat(childIndent)}db_type: sqlite`,
    `${" ".repeat(childIndent)}db_params:`,
    `${" ".repeat(nestedIndent)}db_path: "{{sn_db_path}}"`,
  ];
  const ipLine = filtered.findIndex((line) =>
    leadingSpaces(line) === childIndent && line.trim().startsWith("ip:")
  );
  const insertAt = ipLine >= 0 ? ipLine + 1 : filtered.length;
  const patchedBlock = [
    ...filtered.slice(0, insertAt),
    ...insertLines,
    ...filtered.slice(insertAt),
  ];

  const patchedLines = [
    ...lines.slice(0, start + 1),
    ...patchedBlock,
    ...lines.slice(end),
  ];
  return patchedLines.join("\n");
}

function patchWeb3GatewayConfig(targetDir: string): void {
  const configPath = path.join(targetDir, WEB3_GATEWAY_CONFIG_FILE);
  if (!fs.existsSync(configPath)) {
    console.log(
      `skip ${WEB3_GATEWAY_CONFIG_FILE} patch: file not found in ${targetDir}`,
    );
    return;
  }

  const original = fs.readFileSync(configPath, "utf8");
  const patched = patchWeb3GatewayConfigText(original);
  if (patched === null) {
    console.log(
      `skip ${WEB3_GATEWAY_CONFIG_FILE} patch: web3_sn server block not found`,
    );
    return;
  }
  if (patched !== original) {
    fs.writeFileSync(configPath, patched);
    console.log(`Patched ${configPath} to use ${SN_DB_FILE}`);
  } else {
    console.log(`${configPath} already uses ${SN_DB_FILE}`);
  }
}

async function makeSnConfigs(
  targetDir: string,
  snBaseHost: string,
  snIp: string,
  caDir: string,
  caName: string,
): Promise<void> {
  console.log(`Generating SN configuration files to ${targetDir} ...`);
  console.log(`  SN base domain: ${snBaseHost}`);
  console.log(`  SN IP address: ${snIp}`);
  ensureDir(targetDir);

  console.log("# Step 1: Create SN machine configuration...");
  makeMachineConfig(targetDir, `web3.${snBaseHost}`, DEFAULT_TRUST_DID, false);

  // createSnConfigs 会生成新的 params.json 并覆盖 buckyos-install 从仓库
  // staged 过来的那份;先快照 staged 参数,稍后在 updateParamsJson 里补回。
  const stagedParams = readStagedParams(targetDir);

  // 2. SN device identity configuration (written under <targetDir>/sn_server)
  console.log("# Step 2: Create SN device identity configuration...");
  await createSnConfigs({ outputDir: targetDir, snIp, snBaseHost });

  // move generated files up to targetDir; .buckycli stays in sn_server
  const snServerDir = path.join(targetDir, "sn_server");
  if (fs.existsSync(snServerDir)) {
    for (const name of fs.readdirSync(snServerDir)) {
      const src = path.join(snServerDir, name);
      if (fs.statSync(src).isFile()) {
        fs.renameSync(src, path.join(targetDir, name));
        console.log(`Move file: ${name} -> ${targetDir}/`);
      }
    }
    if (fs.readdirSync(snServerDir).length === 0) {
      fs.rmdirSync(snServerDir);
    }
  }

  const snDbPath = replaceGeneratedSnDb(targetDir);
  const authDataDir = ensureDir(path.join(targetDir, SN_AUTH_DATA_DIR));
  updateParamsJson(targetDir, snDbPath, authDataDir, stagedParams);
  patchWeb3GatewayConfig(targetDir);

  // 3. TLS certificates for sn.$base and *.web3.$base
  console.log("# Step 3: Generate TLS certificates...");
  await ensureCa(caDir, caName);
  const snHostname = `sn.${snBaseHost}`;
  const { certPath, keyPath } = await createCertFromCa(
    caDir,
    snHostname,
    targetDir,
    [
      snHostname,
      `web3.${snBaseHost}`,
      `*.web3.${snBaseHost}`,
    ],
  );
  fs.renameSync(certPath, path.join(targetDir, "fullchain.cert"));
  fs.renameSync(keyPath, path.join(targetDir, "fullchain.pem"));

  // keep dev CA cert+key next to the SN config for client trust install
  const caOutputDir = ensureDir(path.join(targetDir, "ca"));
  for (const name of [`${caName}_ca_cert.pem`, `${caName}_ca_key.pem`]) {
    fs.copyFileSync(path.join(caDir, name), path.join(caOutputDir, name));
  }
  console.log("TLS certificates generated:");
  console.log(`  - ${path.join(targetDir, "fullchain.cert")}`);
  console.log(`  - ${path.join(targetDir, "fullchain.pem")}`);
  console.log(`  - ${path.join(caOutputDir, `${caName}_ca_cert.pem`)}`);
}

async function preregisterDevUsers(
  envRoot: string,
  snDbPath: string,
): Promise<void> {
  console.log("# Step 4: Pre-register dev users to SN database...");
  for (const groupName of PREREGISTER_GROUPS) {
    const params = getParamsFromGroupName(groupName);
    const userDir = path.join(envRoot, params.zone_id);
    if (
      !fs.existsSync(path.join(userDir, params.node_name, "node_identity.json"))
    ) {
      console.log(`user env missing, generate: ${userDir}`);
      await buildUserEnv(params, envRoot);
    } else {
      console.log(`reuse existing user env: ${userDir}`);
    }
    await registerUserToSn(envRoot, params.zone_id, snDbPath);
    await registerDeviceToSn(
      envRoot,
      params.zone_id,
      params.node_name,
      snDbPath,
    );
    syncSnDatabase(snDbPath);
  }
}

async function main(): Promise<void> {
  let values: {
    rootfs?: string;
    ca?: string;
    sn_ip?: string;
    sn_base_host?: string;
    env_root?: string;
    "no-preregister"?: boolean;
    help?: boolean;
  };
  let positionals: string[];
  try {
    const parsed = parseArgs({
      args: Deno.args,
      options: {
        rootfs: { type: "string" },
        ca: { type: "string" },
        sn_ip: { type: "string" },
        sn_base_host: { type: "string" },
        env_root: { type: "string" },
        "no-preregister": { type: "boolean" },
        help: { type: "boolean", short: "h" },
      },
      allowPositionals: true,
    }) as {
      values: {
        rootfs?: string;
        ca?: string;
        sn_ip?: string;
        sn_base_host?: string;
        env_root?: string;
        "no-preregister"?: boolean;
        help?: boolean;
      };
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

  if (positionals.length > 0) {
    console.error(
      `argument error: expected no positional args, got ${positionals.length}`,
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
    console.error("make_sn_configs.ts requires Deno >= 2.2 (node:sqlite)");
    Deno.exit(1);
  }

  const targetDir = values.rootfs ?? defaultTargetRoot();
  const snBaseHost = values.sn_base_host ?? DEFAULT_SN_BASE_HOST;
  const snIp = values.sn_ip ?? Deno.env.get("BUCKYOS_SN_IP") ?? getLocalIp();
  const envRoot = values.env_root ?? ENV_ROOT_DIR;
  const caDir = values.ca ?? ensureDir(path.join(ENV_ROOT_DIR, "ca"));

  await makeSnConfigs(targetDir, snBaseHost, snIp, caDir, DEFAULT_CA_NAME);
  const snDbPath = path.join(targetDir, SN_DB_FILE);

  if (!values["no-preregister"]) {
    await preregisterDevUsers(envRoot, snDbPath);
  }
  syncSnDatabase(snDbPath);

  console.log("\n[OK] SN configuration files generation completed!");
  console.log(`  Output directory: ${targetDir}`);
  console.log(`  SN database: ${snDbPath}`);
  console.log(
    `  SN token key dir: ${path.join(targetDir, SN_AUTH_DATA_DIR)}`,
  );
  console.log("Template/operator files that should be present:");
  console.log(`  - ${path.join(targetDir, WEB3_GATEWAY_CONFIG_FILE)}`);
  console.log(`  - ${path.join(targetDir, "website.yaml")}`);
  console.log(`  - ${path.join(targetDir, "local_dns.toml")}`);
  console.log("Other notes:");
  console.log(
    "  - Test environment needs to install the CA certificate into the client trust list",
  );
}

if (import.meta.main) {
  await main();
}
