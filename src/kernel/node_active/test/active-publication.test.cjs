const assert = require("node:assert/strict");
const { test } = require("node:test");
const { load } = require("./load.cjs");

const plain = (value) => JSON.parse(JSON.stringify(value));
const jwk = { kty: "OKP", crv: "Ed25519", x: "public-test-key" };
const ownerDocument = {
  id: "did:bns:alice",
  name: "alice",
  verificationMethod: [{ publicKeyJwk: jwk }],
  wallets: { main: { type: "eth", address: `0x${"1".repeat(40)}` } },
};

async function setup() {
  const calls = [];
  const state = { failCommit: null, walletPrompts: 0, logins: 0, refreshes: 0, loginError: null };
  const answers = {
    prepare_active_documents: (request) => ({
      owner_document: request.owner_document,
      names: request.names,
      topology: request.topology,
      boot_document: {},
      device_document: {},
      device_mini_document: {},
    }),
    sign_web_active_documents: () => ({ device_document_jwt: "signed.device", zone_document_jwt: "signed.zone" }),
    assemble_zone_document: () => ({ id: "zone" }),
    commit_active: (request) => {
      if (state.failCommit) throw new Error(state.failCommit);
      return { status: "completed", access_hostname: request.prepared.names.access_hostname };
    },
    derive_web_owner_material: () => ({ mnemonic_words: ["w"], owner_public_jwk: jwk, evm_address: "0x1" }),
    load_owner_document: () => ({ owner_document: ownerDocument }),
  };
  class SnClient {
    constructor(url, token) {
      this.token = token;
    }
    async refresh(token) {
      state.refreshes++;
      if (token === "expired") throw new Error("expired");
      return { code: 0, access_token: "refreshed" };
    }
    async login(request) {
      state.logins++;
      calls.push({ method: "auth.login", request });
      if (state.loginError) throw state.loginError;
      return { code: 0, access_token: "login-access", refresh_token: "login-refresh" };
    }
    async getZoneInfo() {
      return { relay_sn: `relay-for-${this.token}` };
    }
  }
  const api = await load("active_lib.ts", {
    stubs: {
      buckyos: {
        buckyos: {
          kRPCClient: class {
            async call(method, request) {
              calls.push({ method, request });
              return answers[method](request);
            }
          },
          hashPassword: async (name, password) => `hash:${name}:${password}`,
          walletSignWithActiveDid: async (payloads) => {
            state.walletPrompts++;
            return { signatures: payloads.map((_, index) => `sig-${index}`), pwd_hash: "wallet-hash" };
          },
        },
        namelib: {},
        sn: { SnClient },
      },
    },
  });
  const domain = (await load("src/domain.ts")).emptyDomainState();
  const data = {
    use_self_domain: false,
    self_domain: "",
    domain,
    gatewy_type: "BuckyForward",
    port_mapping_mode: "full",
    rtcp_port: 2980,
    public_ips: [],
    support_container: true,
    owner_document: ownerDocument,
    device_public_key: jwk,
    device_private_key: "private",
    web_owner_material: { mnemonic_words: ["secret-word"] },
    is_wallet_runtime: false,
    admin_password_hash: "hash",
    sn_access_token: "session",
    sn_access_token_at: Date.now(),
    sn_refresh_token: null,
    sn_user_name: "alice",
    sn_active_code: "code",
    host_account: "lzc",
  };
  return { api, data, calls, state, methods: () => calls.map(({ method }) => method) };
}

async function customDomain(t, overrides = {}) {
  const ctx = await setup();
  ctx.data.use_self_domain = true;
  ctx.data.self_domain = "home.example.com";
  ctx.data.domain = { ...ctx.data.domain, mode: "sn", ...overrides };
  const { checkKey } = await load("src/domain.ts");
  ctx.markReady = () => {
    const target = ctx.api.deriveDomainSetup(ctx.data);
    ctx.data.domain.check = { ready: true, items: [] };
    ctx.data.domain.check_key = checkKey(target, ctx.data.domain);
  };
  return ctx;
}

test("the default domain is prepared, signed and committed without a domain", async () => {
  const { api, data, calls, methods } = await setup();
  const result = await api.activateNode(data, { current: null });
  assert.deepEqual(methods(), ["prepare_active_documents", "sign_web_active_documents", "auth.login", "commit_active"]);
  const commit = calls.at(-1).request;
  assert.equal(commit.domain, null);
  assert.deepEqual(Object.keys(commit.system_settings).sort(), [
    "admin_password_hash", "enabled_features", "friend_passcode", "guest_access", "host_account",
  ]);
  assert.deepEqual(plain(result), { accessHostname: "alice.web3.buckyos.ai", zoneDid: "did:bns:alice" });
});

test("the mnemonic is committed only when the web user asks for it", async () => {
  const { api, data, calls } = await setup();
  const committed = async (overrides) => {
    await api.activateNode({ ...data, ...overrides }, { current: null });
    return plain(calls.at(-1).request);
  };
  assert.equal("owner_mnemonic_words" in (await committed({})), false);
  assert.equal("owner_mnemonic_words" in (await committed({ save_owner_mnemonic: false })), false);
  assert.deepEqual((await committed({ save_owner_mnemonic: true })).owner_mnemonic_words, ["secret-word"]);
  const wallet = { save_owner_mnemonic: true, is_wallet_runtime: true, sn_refresh_token: "r" };
  assert.equal("owner_mnemonic_words" in (await committed(wallet)), false);
});

test("the topology carries the public IPs of a fixed address only", async () => {
  const { api, data } = await setup();
  data.public_ips = ["203.0.113.7"];
  assert.deepEqual(plain(api.deriveGatewayTopology(data)), {
    net_id: "nat", rtcp_port: 2980, support_container: true, sn_url: "https://sn.buckyos.ai/kapi/sn", public_ips: [],
  });
  data.gatewy_type = "WAN";
  assert.deepEqual(plain(api.deriveGatewayTopology(data).public_ips), ["203.0.113.7"]);
});

test("a custom domain that has not passed every check is not activated", async () => {
  const { api, data, calls } = await customDomain();
  await assert.rejects(api.activateNode(data, { current: null }), /\[active:domain_not_ready\]/);
  assert.equal(calls.length, 0);
});

test("a custom domain takes the same path and commits its hosting", async () => {
  const { api, data, calls, methods, markReady } = await customDomain();
  markReady();
  const result = await api.activateNode(data, { current: null });
  assert.deepEqual(methods(), ["prepare_active_documents", "sign_web_active_documents", "auth.login", "commit_active"]);
  assert.deepEqual(plain(calls.at(-1).request.domain), {
    setup: {
      domain: "home.example.com", mode: "sn", certificate: "acme", net_id: "nat", public_ips: [], owner_name: "alice",
    },
    accept_warnings: false,
  });
  assert.equal(result.zoneDid, "did:web:home.example.com");
});

test("DNS API hosting commits the credential id, an uploaded certificate its id and the confirmed warnings", async () => {
  const inspection = { items: [{ code: "public_trust", level: "warn" }], publicly_trusted: false };
  const ctx = await customDomain(null, {
    mode: "dns_api", certificate: "upload", credential_id: "cred", certificate_id: "cert",
    certificate_inspection: inspection,
  });
  ctx.markReady();
  await assert.rejects(ctx.api.activateNode(ctx.data, { current: null }), /domain_not_ready/);
  ctx.data.domain.accept_warnings = true;
  await ctx.api.activateNode(ctx.data, { current: null });
  const { domain } = ctx.calls.at(-1).request;
  assert.equal(domain.setup.mode, "dns_api");
  assert.equal(domain.credential_id, "cred");
  assert.equal(domain.certificate_id, "cert");
  assert.equal(domain.accept_warnings, true);
});

test("a retry sends the same signed documents; changed settings are signed again", async () => {
  const { api, data, calls, state, methods } = await setup();
  const memo = { current: null };
  state.failCommit = "Failed due to reason: [active:sn_challenge_failed] timeout";
  await assert.rejects(api.activateNode(data, memo), /sn_challenge_failed/);
  await assert.rejects(api.activateNode(data, memo), /sn_challenge_failed/);
  state.failCommit = null;
  await api.activateNode(data, memo);
  assert.equal(methods().filter((method) => method === "sign_web_active_documents").length, 1);
  const commits = calls.filter(({ method }) => method === "commit_active");
  assert.equal(commits[0].request.signed_documents, commits[2].request.signed_documents);
  state.failCommit = "x";
  await assert.rejects(api.activateNode(data, memo));
  await assert.rejects(api.activateNode({ ...data, rtcp_port: 3980 }, memo));
  assert.equal(methods().filter((method) => method === "sign_web_active_documents").length, 3);
});

test("a wallet signs twice for an activation and not again for its retry", async () => {
  const { api, data, state } = await setup();
  Object.assign(data, { is_wallet_runtime: true, web_owner_material: null, admin_password_hash: "", sn_refresh_token: "r" });
  const memo = { current: null };
  state.failCommit = "x";
  await assert.rejects(api.activateNode(data, memo));
  await assert.rejects(api.activateNode(data, memo));
  assert.equal(state.walletPrompts, 2);
});

test("the SN session is reused, then refreshed, and never opens the wallet", async () => {
  const { api, data, state } = await setup();
  assert.equal((await api.ensureSnSession(data)).sn_access_token, "session");
  assert.equal(state.refreshes + state.logins, 0);
  data.sn_access_token_at = null;
  data.sn_refresh_token = "refresh";
  assert.equal((await api.ensureSnSession(data)).sn_access_token, "refreshed");
  data.sn_refresh_token = "expired";
  assert.equal((await api.ensureSnSession(data)).sn_access_token, "login-access");
  data.is_wallet_runtime = true;
  assert.equal(await api.ensureSnSession(data), null);
  assert.equal(state.walletPrompts, 0);
  const session = await api.authorizeWalletDomainSession(data, "home.example.com");
  assert.equal(session.sn_refresh_token, "login-refresh");
  assert.equal(state.walletPrompts, 1);
});

test("restoring an identity needs the password, the phrase and the owner key on BNS to agree", async () => {
  const { api, calls, state, methods } = await setup();
  const restored = await api.restoreWebOwner({ name: "alice", password: "pw", mnemonicWords: ["w"] });
  assert.deepEqual(methods(), ["derive_web_owner_material", "auth.login", "load_owner_document"]);
  assert.equal(calls[1].request.pwd_hash, "hash:alice:pw");
  assert.equal(restored.ownerDocument.name, "alice");
  assert.equal(restored.refreshToken, "login-refresh");
  state.loginError = new Error("[SN:1005:invalid_password] invalid password");
  await assert.rejects(api.restoreWebOwner({ name: "alice", password: "no", mnemonicWords: ["w"] }), /invalid_password/);
});

test("a phrase that belongs to another key is refused", async () => {
  const ctx = await setup();
  ownerDocument.verificationMethod[0] = { publicKeyJwk: { ...jwk, x: "another-key" } };
  try {
    await assert.rejects(
      ctx.api.restoreWebOwner({ name: "alice", password: "pw", mnemonicWords: ["w"] }),
      /\[active:mnemonic_owner_mismatch\]/,
    );
  } finally {
    ownerDocument.verificationMethod[0] = { publicKeyJwk: jwk };
  }
});
