const assert = require("node:assert/strict");
const { test } = require("node:test");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const ts = require("typescript");

const identity = {
  sn_username: "alice",
  owner_document: { id: "did:bns:alice", name: "alice" },
  public_key: { kty: "OKP", crv: "Ed25519", x: "test-owner-key" },
};

async function mount({ runtime = "AppRuntime", readUser = async () => identity } = {}) {
  const slots = [];
  const timers = new Map();
  const effects = [];
  let cursor = 0;
  let dirty = true;
  let tree;
  let language = "en";
  let requestCount = 0;
  let nextTimer = 0;
  const i18n = { language };
  const t = (key) => JSON.parse(fs.readFileSync(path.join(__dirname, `../res/${language}.json`), "utf8"))[key] || key;
  const context = vm.createContext({
    console: { warn() {} },
    document: { body: { dataset: {} }, title: "" },
    setTimeout(callback) { timers.set(++nextTimer, callback); return nextTimer; },
    clearTimeout(id) { timers.delete(id); },
  });
  const module = (exports) => new vm.SyntheticModule(Object.keys(exports), function () {
    for (const [name, value] of Object.entries(exports)) this.setExport(name, value);
  }, { context });
  const node = (type, props) => ({ type, props });
  const muiNames = ["Alert", "Box", "Button", "Container", "CssBaseline", "Paper", "Stack", "Chip", "ThemeProvider", "Typography"];
  const modules = {
    react: module({
      useState(initial) {
        const index = cursor++;
        if (!(index in slots)) slots[index] = { value: typeof initial === "function" ? initial() : initial };
        return [slots[index].value, (next) => {
          const value = typeof next === "function" ? next(slots[index].value) : next;
          if (!Object.is(value, slots[index].value)) { slots[index].value = value; dirty = true; }
        }];
      },
      useEffect(callback, deps) {
        const index = cursor++;
        const previous = slots[index];
        if (!previous || deps.some((dep, i) => !Object.is(dep, previous.deps[i]))) {
          effects.push(() => {
            previous?.cleanup?.();
            slots[index] = { deps, cleanup: callback() };
          });
        }
      },
      useMemo(factory) { cursor++; return factory(); },
    }),
    "react/jsx-runtime": module({ jsx: node, jsxs: node, Fragment: "Fragment" }),
    "@mui/material": module({
      ...Object.fromEntries(muiNames.map((name) => [name, name])),
      useMediaQuery: () => false,
      createTheme: (options) => options,
    }),
    "react-i18next": module({ useTranslation: () => ({ t, i18n }) }),
    buckyos: module({
      RuntimeType: { AppRuntime: "AppRuntime", Browser: "Browser" },
      buckyos: { getRuntimeType: () => runtime, getCurrentWalletUser: () => { requestCount++; return readUser(); } },
    }),
    "../active_lib": module({ validateOwnerDocument: (owner) => {
      if (owner.invalid) throw new Error("invalid owner document");
      return owner;
    } }),
    "./components/ActiveWizard": module({ default: "ActiveWizard" }),
    "./components/LanguageSwitch": module({ default: "LanguageSwitch" }),
    "./components/ThemeToggle": module({ default: "ThemeToggle" }),
  };
  const source = fs.readFileSync(path.join(__dirname, "../src/App.tsx"), "utf8");
  const compiled = ts.transpileModule(source, { compilerOptions: {
    module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.ReactJSX,
  } }).outputText;
  const app = new vm.SourceTextModule(compiled, { context });
  await app.link((specifier) => {
    assert(modules[specifier], `Unexpected dependency: ${specifier}`);
    return modules[specifier];
  });
  await app.evaluate();
  async function flush() {
    for (let pass = 0; pass < 8; pass++) {
      if (dirty) {
        dirty = false;
        cursor = 0;
        tree = app.namespace.default();
        for (const effect of effects.splice(0)) effect();
      }
      await new Promise(setImmediate);
    }
    assert(!dirty, "Rendering did not settle");
  }
  function nodes(value) {
    if (Array.isArray(value)) return value.flatMap(nodes);
    if (!value || typeof value !== "object") return [];
    return [value, ...nodes(value.props?.children)];
  }
  function text(value) {
    if (Array.isArray(value)) return value.map(text).join(" ");
    if (!value || typeof value !== "object") return value == null ? "" : String(value);
    return text(value.props?.children);
  }
  await flush();
  return {
    flush, nodes: () => nodes(tree), text: () => text(tree),
    wizard: () => nodes(tree).find((item) => item.type === "ActiveWizard"),
    error: () => nodes(tree).find((item) => item.type === "Alert"),
    retry: async () => { nodes(tree).find((item) => item.type === "Button").props.onClick(); await flush(); },
    expire: async () => { const [id, callback] = [...timers][0]; timers.delete(id); callback(); await flush(); },
    changeLanguage: async (next) => { language = next; i18n.language = next; dirty = true; await flush(); },
    requests: () => requestCount,
    timers: () => timers.size,
  };
}

test("ordinary browsers retain the registration flow without requesting a wallet identity", async () => {
  const app = await mount({ runtime: "Browser" });
  assert.equal(app.requests(), 0);
  assert.equal(app.wizard().props.isWalletRuntime, false);
});

test("wallet identity loading blocks the wizard and then selects the wallet flow", async () => {
  let resolve;
  const app = await mount({ readUser: () => new Promise((done) => { resolve = done; }) });
  assert.equal(app.wizard(), undefined);
  assert.match(app.text(), /Reading wallet identity/);
  resolve(identity);
  await app.flush();
  assert.equal(app.wizard().props.isWalletRuntime, true);
  assert.equal(app.wizard().props.walletUser.sn_username, "alice");
  assert.equal(app.timers(), 0);
});

for (const [name, readUser] of [
  ["null identity", async () => null],
  ["resolver exception", async () => { throw new Error("private resolver diagnostic"); }],
]) {
  test(`${name} shows a localized retry error instead of the registration flow`, async () => {
    const app = await mount({ readUser });
    assert.equal(app.wizard(), undefined);
    assert.equal(app.error().props.severity, "error");
    assert.match(app.text(), /Unable to read the wallet identity/);
    assert.match(app.text(), /create, import or select a BNS identity/);
    assert(!app.text().includes("private resolver diagnostic"));
    assert.equal(app.timers(), 0);
  });
}

test("retry recovers a failed wallet request without changing the flow", async () => {
  let attempts = 0;
  const app = await mount({ readUser: async () => ++attempts === 1 ? null : identity });
  await app.retry();
  assert.equal(app.requests(), 2);
  assert.equal(app.error(), undefined);
  assert.equal(app.wizard().props.isWalletRuntime, true);
});

test("a stalled request times out and late results cannot replace a successful retry", async () => {
  let resolveOld;
  let attempts = 0;
  const app = await mount({ readUser: () => ++attempts === 1 ? new Promise((done) => { resolveOld = done; }) : Promise.resolve(identity) });
  await app.expire();
  assert.equal(app.wizard(), undefined);
  assert(app.error());
  await app.retry();
  resolveOld({ ...identity, sn_username: "old-user" });
  await app.flush();
  assert.equal(app.wizard().props.walletUser.sn_username, "alice");
});

for (const user of [
  { ...identity, sn_username: "" },
  { ...identity, owner_document: null },
  { ...identity, owner_document: { invalid: true } },
]) {
  test("incomplete or invalid wallet identities require selecting an identity in the App", async () => {
    const app = await mount({ readUser: async () => user });
    assert.equal(app.wizard(), undefined);
    assert.match(app.text(), /identity is incomplete/);
    assert.match(app.text(), /create, import or select a BNS identity/);
  });
}

test("language changes update the error and retry button without repeating the identity request", async () => {
  const app = await mount({ readUser: async () => null });
  await app.changeLanguage("zh");
  assert.match(app.text(), /无法读取钱包身份/);
  assert.match(app.text(), /重试/);
  assert.equal(app.requests(), 1);
  assert.equal(app.wizard(), undefined);
});

test("all supported languages translate wallet states and browser registration", () => {
  const i18nSource = fs.readFileSync(path.join(__dirname, "../i18n.ts"), "utf8");
  const languages = [...i18nSource.matchAll(/code: '([^']+)'/g)].map((match) => match[1]);
  assert.equal(languages.length, 9);
  const securitySource = fs.readFileSync(path.join(__dirname, "../src/components/steps/SecurityStep.tsx"), "utf8");
  const registrationKeys = [...securitySource.matchAll(/t\(\s*"([^"]+)"/g)].map((match) => match[1]);
  for (const language of languages) {
    const translations = JSON.parse(fs.readFileSync(path.join(__dirname, `../res/${language}.json`), "utf8"));
    for (const key of ["wallet_identity_loading", "wallet_identity_load_failed", "wallet_identity_incomplete", "wallet_identity_help", "loading", "retry_button", ...registrationKeys]) {
      assert.equal(typeof translations[key], "string", `${language}: ${key}`);
      assert(translations[key].trim());
    }
  }
});
