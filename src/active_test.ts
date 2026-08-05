import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { Buffer } from "node:buffer";
import { createPublicKey, verify } from "node:crypto";
import {
  activateOffline,
  hashAdminPassword,
  validateDomain,
} from "./active.ts";

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function readJson(filePath: string): Record<string, any> {
  return JSON.parse(fs.readFileSync(filePath, "utf8"));
}

function decodeJwt(jwt: string): Record<string, any> {
  const parts = jwt.split(".");
  assert(parts.length === 3, "expected compact JWT");
  return JSON.parse(Buffer.from(parts[1], "base64url").toString("utf8"));
}

function verifyJwt(jwt: string, publicJwk: Record<string, unknown>): boolean {
  const parts = jwt.split(".");
  if (parts.length !== 3) return false;
  return verify(
    null,
    Buffer.from(`${parts[0]}.${parts[1]}`, "utf8"),
    createPublicKey({ key: publicJwk, format: "jwk" }),
    Buffer.from(parts[2], "base64url"),
  );
}

function listFiles(rootDir: string): string[] {
  const result: string[] = [];
  for (const entry of fs.readdirSync(rootDir, { withFileTypes: true })) {
    const entryPath = path.join(rootDir, entry.name);
    if (entry.isDirectory()) result.push(...listFiles(entryPath));
    else if (entry.isFile()) result.push(entryPath);
  }
  return result;
}

Deno.test("offline activation writes a self-contained did:web identity", async () => {
  const sandbox = fs.mkdtempSync(path.join(os.tmpdir(), "active-test-"));
  try {
    const rootDir = path.join(sandbox, "root");
    fs.mkdirSync(path.join(rootDir, "bin"), { recursive: true });
    fs.mkdirSync(path.join(rootDir, "etc"), { recursive: true });
    const ownerKeyBackupPath = path.join(sandbox, "backup", "owner.pem");
    const before = Math.floor(Date.now() / 1000);
    const result = await activateOffline({
      rootDir,
      domain: "Home.Example.com.",
      ownerName: "alice",
      adminPassword: "test-password",
      ownerKeyBackupPath,
      publicIp: "203.0.113.10",
      rtcpPort: 2981,
    });
    const after = Math.floor(Date.now() / 1000);

    assert(
      result.ownerDid === "did:web:home.example.com",
      "owner DID mismatch",
    );
    assert(result.zoneDid === result.ownerDid, "owner and zone DID must match");
    assert(
      result.deviceDid === "did:web:ood1.home.example.com",
      "device DID mismatch",
    );
    assert(fs.existsSync(ownerKeyBackupPath), "owner key backup missing");

    const startConfig = readJson(
      path.join(rootDir, "etc", "start_config.json"),
    );
    assert(
      startConfig.zone_name === result.zoneDid,
      "start config zone mismatch",
    );
    assert(
      startConfig.access_hostname === "home.example.com",
      "hostname mismatch",
    );
    assert(
      startConfig.owner_document.id === result.ownerDid,
      "owner document mismatch",
    );
    assert(
      startConfig.owner_document.verificationMethod[0].controller ===
        result.ownerDid,
      "owner key controller mismatch",
    );
    assert(
      startConfig.admin_password_hash ===
        hashAdminPassword("alice", "test-password"),
      "password hash mismatch",
    );
    const ownerPublicJwk =
      startConfig.owner_document.verificationMethod[0].publicKeyJwk;
    for (
      const key of [
        "private_key",
        "device_private_key",
        "sn_access_token",
        "bns_evm_private_key",
      ]
    ) {
      assert(!(key in startConfig), `start config leaked ${key}`);
    }

    const boot = decodeJwt(startConfig.boot_config_jwt);
    const device = decodeJwt(startConfig.device_doc_jwt);
    const mini = decodeJwt(startConfig.device_mini_doc_jwt);
    const zone = decodeJwt(startConfig.zone_document_jwt);
    for (
      const jwt of [
        startConfig.boot_config_jwt,
        startConfig.device_doc_jwt,
        startConfig.device_mini_doc_jwt,
        startConfig.zone_document_jwt,
      ]
    ) {
      assert(verifyJwt(jwt, ownerPublicJwk), "owner JWT signature is invalid");
    }
    assert(boot.owner === result.ownerDid, "boot owner mismatch");
    assert(boot.id === result.zoneDid, "boot zone mismatch");
    assert(!("sn" in boot), "boot document must not contain SN");
    assert(device.owner === result.ownerDid, "device owner mismatch");
    assert(device.zone_did === result.zoneDid, "device zone mismatch");
    assert(device.net_id === "wan", "device must use wan topology");
    assert(!("ddns_sn_url" in device), "device must not contain SN DDNS URL");
    assert(zone.owner === result.ownerDid, "zone owner mismatch");
    assert(zone.id === result.zoneDid, "zone DID mismatch");
    assert(!("sn" in zone), "zone document must not contain SN");
    for (const document of [boot, device, mini, zone]) {
      assert(
        document.iat >= before && document.iat <= after,
        "document iat is stale",
      );
      assert(document.exp > document.iat, "document expiry is invalid");
    }

    const dns = readJson(path.join(rootDir, "etc", "zone_txt_record.json"));
    assert(dns.hostname === "home.example.com", "DNS hostname mismatch");
    assert(dns.records.length === 4, "expected A plus three TXT records");
    assert(
      dns.records.every((record: Record<string, unknown>) =>
        !JSON.stringify(record).includes("buckyos.ai")
      ),
      "DNS records must not depend on BNS/SN hosts",
    );
    for (const filePath of listFiles(rootDir)) {
      const content = fs.readFileSync(filePath, "utf8");
      assert(
        !content.includes("did:bns:"),
        `committed file contains did:bns: ${filePath}`,
      );
    }

    let repeatedError = "";
    try {
      await activateOffline({
        rootDir,
        domain: "home.example.com",
        ownerName: "alice",
        adminPassword: "test-password",
        ownerKeyBackupPath: path.join(sandbox, "backup", "second.pem"),
      });
    } catch (error) {
      repeatedError = error instanceof Error ? error.message : String(error);
    }
    assert(
      repeatedError.includes("already or partially activated"),
      "repeat activation should be rejected",
    );
  } finally {
    fs.rmSync(sandbox, { recursive: true, force: true });
  }
});

Deno.test("did:web domain validation normalizes and rejects unsupported names", () => {
  assert(
    validateDomain("HOME.Example.com.") === "home.example.com",
    "normalization failed",
  );
  for (
    const invalid of ["localhost", "-home.example.com", "home_example.com"]
  ) {
    let failed = false;
    try {
      validateDomain(invalid);
    } catch {
      failed = true;
    }
    assert(failed, `expected invalid domain to fail: ${invalid}`);
  }
});
