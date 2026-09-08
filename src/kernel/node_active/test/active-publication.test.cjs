const assert = require("node:assert/strict");
const { test } = require("node:test");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const ts = require("typescript");

async function setup(useSelfDomain = true) {
  const calls = [];
  let prepared;
  let signed;
  let failCommit = false;
  const context = vm.createContext({ console, crypto: globalThis.crypto });
  const sdk = new vm.SyntheticModule(["buckyos", "namelib", "sn"], function () {
    this.setExport("buckyos", { kRPCClient: class {
      async call(method, request) {
        calls.push({ method, request });
        if (method === "prepare_active_documents") {
          prepared = {
            owner_document: request.owner_document, names: request.names, topology: request.topology,
            device_document: {
              id: useSelfDomain ? "did:web:ood1.home.example.com" : "did:bns:ood1.alice",
              verificationMethod: [{ publicKeyJwk: request.device_public_key }],
            },
          };
          return prepared;
        }
        if (method === "sign_web_active_documents") {
          signed = { device_document_jwt: "original.signed.device", zone_document_jwt: "original.signed.zone" };
          return signed;
        }
        if (method === "commit_active") {
          if (failCommit) throw new Error("authority publication is not ready");
          return { status: "completed", access_hostname: request.prepared.names.access_hostname };
        }
        throw new Error(method);
      }
    }});
    this.setExport("namelib", {});
    this.setExport("sn", {});
  }, { context });
  const types = new vm.SyntheticModule(["GatewayType"], function () {
    this.setExport("GatewayType", { WAN: "WAN", PortForward: "PortForward", BuckyForward: "BuckyForward" });
  }, { context });
  const source = fs.readFileSync(path.join(__dirname, "../active_lib.ts"), "utf8");
  const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 }}).outputText;
  const mod = new vm.SourceTextModule(compiled, { context });
  await mod.link((specifier) => specifier === "buckyos" ? sdk : types);
  await mod.evaluate();
  const jwk = { kty: "OKP", crv: "Ed25519", x: "public-test-key" };
  const data = {
    use_self_domain: useSelfDomain, self_domain: "home.example.com",
    domain_binding: { state: "verified" }, gatewy_type: "BuckyForward", rtcp_port: 2980,
    port_mapping_mode: "full",
    owner_document: { id: "did:bns:alice", name: "alice", verificationMethod: [{ publicKeyJwk: jwk }] },
    device_public_key: jwk, device_private_key: "private-not-for-publication",
    web_owner_material: { mnemonic_words: ["not-for-publication"] }, is_wallet_runtime: false,
    admin_password_hash: "", sn_access_token: "private-session", sn_refresh_token: null,
    sn_user_name: "alice", prepared_documents: null, signed_documents: null,
  };
  return { api: mod.namespace, data, calls, failNextCommit: () => { failCommit = true; }, allowCommit: () => { failCommit = false; } };
}

test("custom-domain activation requires publication preparation", async () => {
  const { api, data, calls } = await setup();
  await assert.rejects(api.activateNode(data), /Prepare and publish/);
  assert.equal(calls.length, 0);
});

test("export and activation retries use the same signed bytes without signing again", async () => {
  const { api, data, calls, failNextCommit, allowCommit } = await setup();
  const material = await api.prepareSignedActivation(data);
  data.prepared_documents = material.prepared;
  data.signed_documents = material.signed;
  data.admin_password_hash = material.adminPasswordHash;
  const publication = api.customDomainPublication(data);
  assert.equal(publication.url, "https://ood1.home.example.com/.well-known/did.json");
  assert.equal(publication.content, "original.signed.device");
  assert(!publication.content.includes(data.device_private_key));
  failNextCommit();
  await assert.rejects(api.activateNode(data), /not ready/);
  allowCommit();
  await api.activateNode(data);
  assert.deepEqual(calls.map(({ method }) => method), [
    "prepare_active_documents", "sign_web_active_documents", "commit_active", "commit_active",
  ]);
  for (const { request } of calls.filter(({ method }) => method === "commit_active")) {
    assert.equal(request.signed_documents.device_document_jwt, publication.content);
    assert.equal(request.prepared, material.prepared);
  }
  assert.throws(() => api.customDomainPublication({ ...data, self_domain: "other.example.com" }), /settings changed/);
  assert.throws(() => api.customDomainPublication({ ...data, rtcp_port: 3980 }), /settings changed/);
});

test("BNS activation retains the prepare-sign-commit flow", async () => {
  const { api, data, calls } = await setup(false);
  await api.activateNode(data);
  assert.deepEqual(calls.map(({ method }) => method), [
    "prepare_active_documents", "sign_web_active_documents", "commit_active",
  ]);
});
