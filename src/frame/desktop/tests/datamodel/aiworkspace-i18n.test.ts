/* AI Workspace i18n guard (src/app/aiworkspace/i18n.ts):
 *
 *   deno test --allow-read --allow-env tests/datamodel/aiworkspace-i18n.test.ts [-- <path prefix>…]
 *
 * - each area table (src/i18n/aiworkspace/<area>.ts) has the same keys and placeholders in English and Chinese,
 *   its keys use the area's own prefixes, English text has no Chinese, and no key is in two areas;
 * - the app's source (and the shared draw.io importer) has no Chinese in string literals, template text or JSX
 *   text — comments are fine; a literal that must stay (data, a CJK regex) carries `i18n-exempt: <reason>` on
 *   its line or the line above;
 * - every `aiworkspace.<area>.…` literal in the source is a key, and every key is written out as a literal
 *   somewhere (keys are never assembled from pieces, so this check can see them);
 * - the backend speaks English: no Chinese in the string literals of src/frame/aiworkspace/{core,store,server}/src
 *   (comments are fine; a literal that recognises Chinese input carries `i18n-exempt: <reason>`), nor in the
 *   program-host assets the server compiles in (aiws/prompts, aiws/README.md, renderers.json, aiws.js, run.js).
 *
 * Path prefixes (relative to src/frame/desktop) limit the source checks to those files and skip the unused-key
 * check, for working on one part of the app. */

import ts from 'typescript'
import * as blocks from '../../src/i18n/aiworkspace/blocks.ts'
import * as canvas from '../../src/i18n/aiworkspace/canvas.ts'
import * as common from '../../src/i18n/aiworkspace/common.ts'
import * as core from '../../src/i18n/aiworkspace/core.ts'
import * as data from '../../src/i18n/aiworkspace/data.ts'
import * as demos from '../../src/i18n/aiworkspace/demos.ts'
import * as ext from '../../src/i18n/aiworkspace/ext.ts'
import * as imports from '../../src/i18n/aiworkspace/import.ts'
import * as pdf from '../../src/i18n/aiworkspace/pdf.ts'
import * as presentation from '../../src/i18n/aiworkspace/presentation.ts'
import * as richtext from '../../src/i18n/aiworkspace/richtext.ts'
import * as shell from '../../src/i18n/aiworkspace/shell.ts'
import * as wish from '../../src/i18n/aiworkspace/wish.ts'

type Table = Record<string, string>
interface Part { name: string; prefixes: string[]; en: Table; zh: Table }

/** Each area file and the key prefixes (`aiworkspace.<segment>.`) it owns. */
const PARTS: Part[] = [
  { name: 'common', prefixes: ['common'], ...common },
  { name: 'shell', prefixes: ['shell'], ...shell },
  { name: 'canvas', prefixes: ['canvas', 'drawio'], ...canvas },
  { name: 'blocks', prefixes: ['blocks', 'connector'], ...blocks },
  { name: 'wish', prefixes: ['wish', 'extension'], ...wish },
  { name: 'data', prefixes: ['data'], ...data },
  { name: 'presentation', prefixes: ['presentation'], ...presentation },
  { name: 'richtext', prefixes: ['richtext'], ...richtext },
  { name: 'core', prefixes: ['core'], ...core },
  { name: 'pdf', prefixes: ['pdf'], ...pdf },
  { name: 'demos', prefixes: ['demo'], ...demos },
  { name: 'ext', prefixes: ['ext'], ...ext },
  { name: 'import', prefixes: ['import'], ...imports },
]

const ROOT = new URL('../../', import.meta.url).pathname
const SOURCE_DIRS = ['src/app/aiworkspace', '../aiworkspace/import-drawio', '../aiworkspace/import-pdf', '../aiworkspace/import-table']
const SKIP_DIRS = new Set(['wasm', 'node_modules', 'tools', 'fixtures'])
const CJK = /[\u3000-\u303f\u3400-\u4dbf\u4e00-\u9fff\uff00-\uffef]/
const EXEMPT = 'i18n-exempt'
const filters = Deno.args.filter((arg) => !arg.startsWith('-')).map((arg) => arg.replace(/^\.\//, ''))

const placeholders = (text: string) => [...text.matchAll(/\{\{(\w+)\}\}/g)].map((m) => m[1]).sort()
const baseKey = (key: string) => key.replace(/_one$/, '')

function fail(problems: string[], what: string) {
  if (problems.length) throw new Error(`${problems.length} ${what}:\n  ${problems.slice(0, 200).join('\n  ')}${problems.length > 200 ? '\n  …' : ''}`)
}

for (const part of PARTS) {
  Deno.test(`aiworkspace i18n table: ${part.name}`, () => {
    const problems: string[] = []
    const owned = part.prefixes.map((p) => `aiworkspace.${p}.`)
    for (const [key, text] of Object.entries(part.en)) {
      if (!owned.some((p) => key.startsWith(p))) problems.push(`${key}: not under ${owned.join(' / ')}`)
      if (!text.trim()) problems.push(`${key}: empty English text`)
      if (CJK.test(text)) problems.push(`${key}: Chinese in the English text: ${text}`)
      if (key.endsWith('_one')) {
        if (!(baseKey(key) in part.en)) problems.push(`${key}: singular without its base key`)
        const extra = placeholders(text).filter((p) => !placeholders(part.en[baseKey(key)] ?? '').includes(p))
        if (extra.length) problems.push(`${key}: placeholders ${extra} not in the base key`)
        continue
      }
      if (!(key in part.zh)) { problems.push(`${key}: missing in Chinese`); continue }
      if (!part.zh[key].trim()) problems.push(`${key}: empty Chinese text`)
      const a = placeholders(text).join(','), b = placeholders(part.zh[key]).join(',')
      if (a !== b) problems.push(`${key}: placeholders differ (en: ${a || '-'}; zh: ${b || '-'})`)
    }
    for (const key of Object.keys(part.zh)) {
      if (!(key in part.en)) problems.push(`${key}: only in Chinese`)
    }
    fail(problems, `problems in src/i18n/aiworkspace/${part.name}.ts`)
  })
}

Deno.test('aiworkspace i18n: no key in two areas', () => {
  const seen = new Map<string, string>()
  const problems: string[] = []
  for (const part of PARTS) {
    for (const key of Object.keys(part.en)) {
      const other = seen.get(key)
      if (other) problems.push(`${key}: in ${other} and ${part.name}`)
      seen.set(key, part.name)
    }
  }
  fail(problems, 'duplicated keys')
})

function sourceFiles(): string[] {
  const out: string[] = []
  const walk = (dir: string) => {
    for (const entry of Deno.readDirSync(ROOT + dir)) {
      const path = `${dir}/${entry.name}`
      if (entry.isDirectory) { if (!SKIP_DIRS.has(entry.name)) walk(path); continue }
      if (/\.(ts|tsx)$/.test(entry.name) && !/(_test|\.test|\.d)\.tsx?$/.test(entry.name)) out.push(path)
    }
  }
  for (const dir of SOURCE_DIRS) walk(dir)
  return out.sort()
}

interface Literal { file: string; line: number; text: string; exempt: boolean }

function literals(file: string): Literal[] {
  const text = Deno.readTextFileSync(ROOT + file)
  const lines = text.split('\n')
  const source = ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true, file.endsWith('.tsx') ? ts.ScriptKind.TSX : ts.ScriptKind.TS)
  const out: Literal[] = []
  const visit = (node: ts.Node) => {
    let value: string | null = null
    if (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node) || ts.isTemplateHead(node) || ts.isTemplateMiddle(node) || ts.isTemplateTail(node) || ts.isJsxText(node)) value = node.text
    if (value !== null) {
      const start = source.getLineAndCharacterOfPosition(node.getStart(source)).line
      const end = source.getLineAndCharacterOfPosition(node.getEnd()).line
      const exempt = lines.slice(Math.max(0, start - 1), end + 1).some((l) => l.includes(EXEMPT))
      out.push({ file, line: start + 1, text: value, exempt })
    }
    ts.forEachChild(node, visit)
  }
  visit(source)
  return out
}

const files = sourceFiles()
const all = files.map((file) => ({ file, found: literals(file) }))
const scanned = all.filter(({ file }) => !filters.length || filters.some((f) => file.startsWith(f)))
const en: Table = Object.assign({}, ...PARTS.map((p) => p.en))
const keyPrefixes = PARTS.flatMap((p) => p.prefixes.map((s) => `aiworkspace.${s}.`))
const isKeyLiteral = (text: string) => keyPrefixes.some((p) => text.startsWith(p)) && /^[\w.-]+$/.test(text)

Deno.test('aiworkspace i18n: no hard-coded Chinese in the source', () => {
  const problems = scanned.flatMap(({ found }) => found.filter((l) => !l.exempt && CJK.test(l.text)).map((l) => `${l.file}:${l.line}: ${l.text.trim().slice(0, 70)}`))
  fail(problems, 'literals with Chinese text (move them to src/i18n/aiworkspace, or mark i18n-exempt with a reason)')
})

Deno.test('aiworkspace i18n: every key the source names exists', () => {
  const problems = scanned.flatMap(({ found }) => found.filter((l) => isKeyLiteral(l.text) && !(l.text in en)).map((l) => `${l.file}:${l.line}: ${l.text}`))
  fail(problems, 'unknown keys')
})

Deno.test({
  name: 'aiworkspace i18n: every key is used',
  ignore: filters.length > 0,
  fn: () => {
    const used = new Set(all.flatMap(({ found }) => found.filter((l) => isKeyLiteral(l.text)).map((l) => l.text)))
    const problems = Object.keys(en).filter((key) => !key.endsWith('_one') && !used.has(key))
    fail(problems, 'keys no source names as a literal (unused, or assembled from pieces)')
  },
})

/** String and char literals of a Rust file (comments skipped), with their line numbers. */
function rustLiterals(file: string): Literal[] {
  const text = Deno.readTextFileSync(ROOT + file)
  const lines = text.split('\n')
  const out: Literal[] = []
  const lineAt = (i: number) => text.slice(0, i).split('\n').length
  const push = (start: number, end: number, value: string) => {
    const a = lineAt(start), b = lineAt(end)
    out.push({ file, line: a, text: value, exempt: lines.slice(Math.max(0, a - 2), b).some((l) => l.includes(EXEMPT)) })
  }
  let i = 0
  while (i < text.length) {
    const c = text[i], next = text[i + 1]
    if (c === '/' && next === '/') { i = text.indexOf('\n', i); if (i < 0) break; continue }
    if (c === '/' && next === '*') {
      let depth = 1; i += 2
      while (i < text.length && depth > 0) {
        if (text[i] === '/' && text[i + 1] === '*') { depth += 1; i += 2 } else if (text[i] === '*' && text[i + 1] === '/') { depth -= 1; i += 2 } else i += 1
      }
      continue
    }
    const raw = /^b?r(#*)"/.exec(text.slice(i, i + 40))
    if (raw && !/[\w]/.test(text[i - 1] ?? '')) {
      const close = '"' + raw[1]
      const start = i + raw[0].length
      const end = text.indexOf(close, start)
      push(i, end, text.slice(start, end))
      i = end + close.length
      continue
    }
    if (c === '"') {
      let j = i + 1
      while (j < text.length && text[j] !== '"') j += text[j] === '\\' ? 2 : 1
      push(i, j, text.slice(i + 1, j))
      i = j + 1
      continue
    }
    if (c === "'") {
      // a char literal ('x', '\n', '\u{..}'), else a lifetime or label
      const m = /^'(\\(u\{[0-9a-fA-F]+\}|x[0-9a-fA-F]{2}|.)|[^\\'\n])'/.exec(text.slice(i, i + 16))
      if (m) { push(i, i, m[0].slice(1, -1)); i += m[0].length; continue }
    }
    i += 1
  }
  return out
}

const BACKEND = '../aiworkspace'
function backendFiles(): string[] {
  const out: string[] = []
  const walk = (dir: string) => {
    for (const entry of Deno.readDirSync(ROOT + dir)) {
      const path = `${dir}/${entry.name}`
      if (entry.isDirectory) walk(path)
      else if (entry.name.endsWith('.rs')) out.push(path)
    }
  }
  for (const crate of ['core', 'store', 'server']) walk(`${BACKEND}/${crate}/src`)
  return out.sort()
}

Deno.test('aiworkspace i18n: the backend speaks English', () => {
  const problems: string[] = []
  for (const file of backendFiles()) {
    for (const l of rustLiterals(file)) if (!l.exempt && CJK.test(l.text)) problems.push(`${file}:${l.line}: ${l.text.trim().slice(0, 70)}`)
  }
  // the program host's assets the server compiles in: prompts and API text are read whole, scripts by literal
  for (const file of ['aiws/prompts/analyze.md', 'aiws/prompts/execute.md', 'aiws/README.md', 'aiws/renderers.json']) {
    Deno.readTextFileSync(`${ROOT}${BACKEND}/${file}`).split('\n').forEach((line, i) => { if (CJK.test(line)) problems.push(`${BACKEND}/${file}:${i + 1}: ${line.trim().slice(0, 70)}`) })
  }
  for (const file of ['aiws/aiws.js', 'aiws/run.js']) {
    for (const l of literals(`${BACKEND}/${file}`)) if (!l.exempt && CJK.test(l.text)) problems.push(`${l.file}:${l.line}: ${l.text.trim().slice(0, 70)}`)
  }
  fail(problems, 'Chinese texts in the backend (it speaks English; mark input matching i18n-exempt with a reason)')
})
