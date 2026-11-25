const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const ts = require("typescript");

const root = path.join(__dirname, "..");

/** Loads a TypeScript module with its relative imports; `stubs` replaces packages. */
async function load(entry, { stubs = {}, globals = {} } = {}) {
  const context = vm.createContext({ console, crypto: globalThis.crypto, URL, TextEncoder, atob, ...globals });
  const cache = new Map();
  const moduleFor = (specifier, parent) => {
    if (stubs[specifier]) {
      const exports = stubs[specifier];
      return new vm.SyntheticModule(Object.keys(exports), function () {
        for (const [name, value] of Object.entries(exports)) this.setExport(name, value);
      }, { context });
    }
    const base = path.resolve(path.dirname(parent), specifier);
    const file = [base, `${base}.ts`, `${base}.tsx`].find((name) => fs.existsSync(name) && fs.statSync(name).isFile());
    if (!file) throw new Error(`Unexpected dependency: ${specifier}`);
    if (!cache.has(file)) {
      const source = ts.transpileModule(fs.readFileSync(file, "utf8"), {
        compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.ReactJSX },
      }).outputText;
      cache.set(file, new vm.SourceTextModule(source, { context, identifier: file }));
    }
    return cache.get(file);
  };
  const module = moduleFor(`./${entry}`, path.join(root, "index.ts"));
  await module.link((specifier, parent) => moduleFor(specifier, parent.identifier));
  await module.evaluate();
  return module.namespace;
}

/** A clock whose timers run only when it is advanced. */
function fakeClock(start = 0) {
  let now = start;
  let next = 0;
  const pending = new Map();
  const settle = async () => {
    for (let i = 0; i < 20; i++) await new Promise(setImmediate);
  };
  return {
    now: () => now,
    timers: {
      set: (callback, ms) => {
        pending.set(++next, { at: now + ms, callback });
        return next;
      },
      clear: (id) => pending.delete(id),
    },
    /** The clock moves on without any timer firing: a throttled tab or a sleeping laptop. */
    jump: (ms) => {
      now += ms;
    },
    advance: async (ms) => {
      const until = now + ms;
      await settle();
      for (;;) {
        const due = [...pending].filter(([, timer]) => timer.at <= until).sort((a, b) => a[1].at - b[1].at)[0];
        if (!due) break;
        pending.delete(due[0]);
        now = Math.max(now, due[1].at);
        due[1].callback();
        await settle();
      }
      now = until;
    },
    settle,
  };
}

module.exports = { load, fakeClock, root };
