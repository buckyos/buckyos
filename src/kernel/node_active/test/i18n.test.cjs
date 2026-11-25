const assert = require("node:assert/strict");
const { test } = require("node:test");
const fs = require("node:fs");
const path = require("node:path");
const ts = require("typescript");
const { root } = require("./load.cjs");

const languages = [...fs.readFileSync(path.join(root, "i18n.ts"), "utf8").matchAll(/code: '([^']+)'/g)].map((match) => match[1]);
const read = (file) => JSON.parse(fs.readFileSync(path.join(root, "res", file), "utf8"));
const common = Object.fromEntries(languages.map((language) => [language, read(`${language}.json`)]));

const withPrefix = (prefix, names) => names.map((name) => prefix + name);

const SN_BIND_ERRORS = [
  "invalid_domain", "domain_proof_failed", "user_not_found", "user_not_activated", "auth_required", "invalid_token",
  "internal_error", "unavailable", "other",
];
const CODES = [
  "txt_missing", "txt_mismatch", ...withPrefix("bind_", SN_BIND_ERRORS), "ns_missing", "ns_extra", "ns_not_answering",
  "record_missing", "record_mismatch", "record_conflict", "stale_record", "proxied", "apex_needs_subdomain", "caa_denied",
  "certificate_missing", "certificate_rejected", "challenge_write_failed", "challenge_not_visible", "challenge_busy", "credential_missing",
  "credential_rejected", "no_session", "lookup_failed", "commit_time",
  "credential_invalid", "zone_not_found", "rate_limited", "network", "provider_error",
  "dns_lookup_failed", "domain_not_ready", "sn_challenge_failed", "certificate_warning_unconfirmed",
  "mnemonic_invalid", "owner_not_found", "mnemonic_owner_mismatch",
  "domain_not_found", "invalid_setup", "bns_unavailable", "certificate_unreadable", "certificate_too_large",
  "install_failed", "owner_document_invalid", "owner_key_invalid", "unknown",
];

/** Keys built at run time, by the prefix they are built with. */
const DYNAMIC = {
  code_: CODES,
  title_step_: ["gateway", "security", "domain", "review"],
  region_name_: ["jp", "us", "us-west", "eu-central"],
  blocked_: ["apex", "apex_needs_subdomain"],
  mode_: ["sn", "dns_api", "manual"],
  cert_source_: ["acme", "upload"],
  check_status_: ["pass", "waiting", "fail", "skipped"],
  check_item_: ["ownership", "delegation", "entry", "challenge", "caa", "certificate"],
  write_status_: ["written", "unchanged", "needs_confirm", "failed"],
  result_cause_: ["node", "dns", "service"],
  sn_error_: ["invalid_password", "user_auth_not_found", "user_not_activated", "email_already_bound"],
  cert_check_: [
    "readable", "key_matches", "covers_domain", "covers_wildcard", "validity", "server_usage", "not_ca", "public_trust",
    "key_algorithm",
  ],
  cert_reason_: [
    "empty", "unparsable", "key_missing", "key_encrypted", "pfx_password_wrong", "key_mismatch", "parent_wildcard",
    "single_domain", "wildcard_only", "ev", "not_covered", "expired", "not_yet_valid", "expires_soon", "no_server_auth",
    "is_ca", "self_signed", "chain_not_public", "unsupported_key", "other",
  ],
};

/** Keys kept in a variable before they are translated. */
const INDIRECT = {
  "App.tsx:initError": ["wallet_identity_load_failed", "wallet_identity_incomplete"],
  "CertificatePanel.tsx:key": ["cert_untrusted_browser", "cert_untrusted_zone", "cert_untrusted_app", "cert_untrusted_check"],
  "CustomDomain.tsx:key": [
    "check_all_passed", "cert_confirm_pending", "check_auto", "check_wait_hint", "check_leave_hint",
  ],
  "ResultStep.tsx:key": ["result_hint_tunnel", "result_hint_dns", "result_hint_loopback"],
};

const PROVIDER_PREFIXES = ["steps.", "fields."];

function sources(directory) {
  return fs.readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(directory, entry.name);
    return entry.isDirectory() ? sources(file) : /\.tsx?$/.test(entry.name) ? [file] : [];
  });
}

function usedKeys() {
  const keys = new Set();
  const collect = (node, file) => {
    if (ts.isStringLiteralLike(node)) keys.add(node.text);
    else if (ts.isConditionalExpression(node)) [node.whenTrue, node.whenFalse].forEach((child) => collect(child, file));
    else if (ts.isArrayLiteralExpression(node)) node.elements.forEach((child) => collect(child, file));
    else if (ts.isTemplateExpression(node)) {
      const prefix = node.head.text;
      if (PROVIDER_PREFIXES.includes(prefix)) return;
      assert(DYNAMIC[prefix], `${file}: list the keys built with "${prefix}"`);
      withPrefix(prefix, DYNAMIC[prefix]).forEach((key) => keys.add(key));
    } else {
      const name = `${path.basename(file)}:${node.getText()}`;
      assert(INDIRECT[name], `list the keys behind ${name}`);
      INDIRECT[name].forEach((key) => keys.add(key));
    }
  };
  for (const file of sources(path.join(root, "src"))) {
    const tree = ts.createSourceFile(file, fs.readFileSync(file, "utf8"), ts.ScriptTarget.ESNext, true);
    const visit = (node) => {
      if (ts.isCallExpression(node) && node.expression.getText() === "t" && node.arguments[0]) {
        collect(node.arguments[0], file);
      }
      ts.forEachChild(node, visit);
    };
    visit(tree);
  }
  return keys;
}

test("all nine languages have the same keys", () => {
  assert.equal(languages.length, 9);
  const expected = Object.keys(common.en).sort();
  for (const language of languages) {
    assert.deepEqual(Object.keys(common[language]).sort(), expected, language);
    for (const [key, text] of Object.entries(common[language])) assert(text.trim(), `${language}: ${key} is empty`);
  }
});

test("every translation keeps the placeholders of the English text", () => {
  const placeholders = (text) => [...new Set([...text.matchAll(/{{(\w+)}}/g)].map((match) => match[1]))].sort();
  for (const language of languages) {
    for (const [key, text] of Object.entries(common.en)) {
      assert.deepEqual(placeholders(common[language][key]), placeholders(text), `${language}: ${key}`);
    }
  }
});

test("every key the code uses exists, and nothing else is kept", () => {
  const used = [...usedKeys()].sort();
  assert.deepEqual(used, Object.keys(common.en).sort());
});

test("no visible text outside the language files", () => {
  for (const file of sources(path.join(root, "src"))) {
    const source = fs.readFileSync(file, "utf8");
    assert(!/\bt\(\s*"[^"]+",\s*"/.test(source), `${path.basename(file)} has a default text`);
  }
});

test("the DNS service guides exist in all nine languages with the same services and fields", () => {
  const shape = (guide) => ({
    fields: Object.fromEntries(Object.entries(guide.fields).map(([id, fields]) => [id, Object.keys(fields).sort()])),
    steps: Object.keys(guide.steps).sort(),
  });
  const expected = shape(read("dns_providers/en.json"));
  for (const language of languages) {
    const guide = read(`dns_providers/${language}.json`);
    assert.deepEqual(shape(guide), expected, language);
    for (const [id, steps] of Object.entries(guide.steps)) {
      assert(Array.isArray(steps) && steps.length >= 2 && steps.length <= 5, `${language}: ${id}`);
    }
  }
});
