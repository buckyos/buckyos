/* AI Workspace i18n guard (src/app/aiworkspace/i18n.ts):
 *
 *   deno test --allow-read --allow-env tests/datamodel/aiworkspace-i18n.test.ts [-- <path prefix>…]
 *
 * - each area table (src/i18n/aiworkspace/<area>.ts) has the same keys and placeholders in English and Chinese,
 *   its keys use the area's own prefixes, English text has no Chinese, and no key is in two areas;
 * - every area file is listed here, and each other language's table (src/i18n/aiworkspace/locales/<language>.ts) has
 *   every English key with the same placeholders, plus only the plural forms its language has (CLDR) of `{{count}}` texts;
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
import * as blockui from '../../src/i18n/aiworkspace/blockui.ts'
import * as blockprops from '../../src/i18n/aiworkspace/blockprops.ts'
import * as canvas from '../../src/i18n/aiworkspace/canvas.ts'
import * as common from '../../src/i18n/aiworkspace/common.ts'
import * as core from '../../src/i18n/aiworkspace/core.ts'
import * as data from '../../src/i18n/aiworkspace/data.ts'
import * as demos from '../../src/i18n/aiworkspace/demos.ts'
import * as directory from '../../src/i18n/aiworkspace/directory.ts'
import * as ext from '../../src/i18n/aiworkspace/ext.ts'
import * as imports from '../../src/i18n/aiworkspace/import.ts'
import * as pdf from '../../src/i18n/aiworkspace/pdf.ts'
import * as presentation from '../../src/i18n/aiworkspace/presentation.ts'
import * as office from '../../src/i18n/aiworkspace/office.ts'
import * as richtext from '../../src/i18n/aiworkspace/richtext.ts'
import * as schedule from '../../src/i18n/aiworkspace/schedule.ts'
import * as framework from '../../src/i18n/aiworkspace/framework.ts'
import * as home from '../../src/i18n/aiworkspace/home.ts'
import * as table from '../../src/i18n/aiworkspace/table.ts'
import * as phone from '../../src/i18n/aiworkspace/phone.ts'
import * as pathnode from '../../src/i18n/aiworkspace/pathnode.ts'
import * as cover from '../../src/i18n/aiworkspace/cover.ts'
import * as session from '../../src/i18n/aiworkspace/session.ts'
import * as shell from '../../src/i18n/aiworkspace/shell.ts'
import * as wish from '../../src/i18n/aiworkspace/wish.ts'
import * as wishcard from '../../src/i18n/aiworkspace/wishcard.ts'
import * as wishlinks from '../../src/i18n/aiworkspace/wishlinks.ts'
import * as samplewish from '../../src/i18n/aiworkspace/samplewish.ts'
import * as runlog from '../../src/i18n/aiworkspace/runlog.ts'
import * as jarvis from '../../src/i18n/aiworkspace/jarvis.ts'
import * as sourcereq from '../../src/i18n/aiworkspace/sourcereq.ts'
import * as datasource from '../../src/i18n/aiworkspace/datasource.ts'
import * as print from '../../src/i18n/aiworkspace/print.ts'
import * as printrender from '../../src/i18n/aiworkspace/printrender.ts'
import * as pdfframe from '../../src/i18n/aiworkspace/pdfframe.ts'
import * as activity from '../../src/i18n/aiworkspace/activity.ts'
import * as customize from '../../src/i18n/aiworkspace/customize.ts'
import * as datamode from '../../src/i18n/aiworkspace/datamode.ts'
import * as fielddefault from '../../src/i18n/aiworkspace/fielddefault.ts'
import * as members from '../../src/i18n/aiworkspace/members.ts'
import * as mindmap from '../../src/i18n/aiworkspace/mindmap.ts'
import * as objecttree from '../../src/i18n/aiworkspace/objecttree.ts'
import * as permissions from '../../src/i18n/aiworkspace/permissions.ts'
import * as settings from '../../src/i18n/aiworkspace/settings.ts'
import * as surfaces from '../../src/i18n/aiworkspace/surfaces.ts'
import * as tablecurrent from '../../src/i18n/aiworkspace/tablecurrent.ts'
import * as worklog from '../../src/i18n/aiworkspace/worklog.ts'

type Table = Record<string, string>
interface Part { name: string; prefixes: string[]; en: Table; zh: Table }

/** Each area file and the key prefixes (`aiworkspace.<segment>.`) it owns. */
const PARTS: Part[] = [
  { name: 'common', prefixes: ['common'], ...common },
  { name: 'shell', prefixes: ['shell'], ...shell },
  { name: 'canvas', prefixes: ['canvas', 'drawio'], ...canvas },
  { name: 'blocks', prefixes: ['blocks', 'connector'], ...blocks },
  { name: 'blockui', prefixes: ['blockui'], ...blockui },
  { name: 'blockprops', prefixes: ['blockprops'], ...blockprops },
  { name: 'wish', prefixes: ['wish', 'extension'], ...wish },
  { name: 'data', prefixes: ['data'], ...data },
  { name: 'directory', prefixes: ['directory'], ...directory },
  { name: 'presentation', prefixes: ['presentation'], ...presentation },
  { name: 'richtext', prefixes: ['richtext'], ...richtext },
  { name: 'session', prefixes: ['session'], ...session },
  { name: 'core', prefixes: ['core'], ...core },
  { name: 'pdf', prefixes: ['pdf'], ...pdf },
  { name: 'demos', prefixes: ['demo'], ...demos },
  { name: 'ext', prefixes: ['ext'], ...ext },
  { name: 'import', prefixes: ['import'], ...imports },
  { name: 'office', prefixes: ['office'], ...office },
  { name: 'schedule', prefixes: ['label', 'datactl', 'todo', 'calendar', 'broadcast'], ...schedule },
  { name: 'framework', prefixes: ['framework'], ...framework },
  { name: 'home', prefixes: ['home'], ...home },
  { name: 'table', prefixes: ['table'], ...table },
  { name: 'phone', prefixes: ['phone'], ...phone },
  { name: 'pathnode', prefixes: ['pathnode'], ...pathnode },
  { name: 'cover', prefixes: ['cover'], ...cover },
  { name: 'wishcard', prefixes: ['wishcard'], ...wishcard },
  { name: 'wishlinks', prefixes: ['wishlinks'], ...wishlinks },
  { name: 'samplewish', prefixes: ['samplewish'], ...samplewish },
  { name: 'runlog', prefixes: ['runlog'], ...runlog },
  { name: 'jarvis', prefixes: ['jarvis'], ...jarvis },
  { name: 'sourcereq', prefixes: ['sourcereq'], ...sourcereq },
  { name: 'datasource', prefixes: ['datasource'], ...datasource },
  { name: 'print', prefixes: ['print'], ...print },
  { name: 'printrender', prefixes: ['printrender'], ...printrender },
  { name: 'pdfframe', prefixes: ['pdfframe'], ...pdfframe },
  { name: 'activity', prefixes: ['activity'], ...activity },
  { name: 'customize', prefixes: ['customize'], ...customize },
  { name: 'datamode', prefixes: ['datamode'], ...datamode },
  { name: 'fielddefault', prefixes: ['fielddefault'], ...fielddefault },
  { name: 'members', prefixes: ['members'], ...members },
  { name: 'mindmap', prefixes: ['mindmap'], ...mindmap },
  { name: 'objecttree', prefixes: ['objecttree'], ...objecttree },
  { name: 'permissions', prefixes: ['perm'], ...permissions },
  { name: 'settings', prefixes: ['settings'], ...settings },
  { name: 'surfaces', prefixes: ['surfaces'], ...surfaces },
  { name: 'tablecurrent', prefixes: ['tablecurrent'], ...tablecurrent },
  { name: 'worklog', prefixes: ['worklog'], ...worklog },
]

/** The languages with a table of their own besides English and Simplified Chinese (src/app/aiworkspace/i18n.ts). */
const LOCALES = ['zh-TW', 'ja', 'ko', 'fr', 'de', 'es', 'ru', 'ar']

const ROOT = new URL('../../', import.meta.url).pathname
const SOURCE_DIRS = ['src/app/aiworkspace', '../aiworkspace/import-drawio', '../aiworkspace/import-pdf', '../aiworkspace/import-table', '../aiworkspace/import-office']
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
      // ＋ names the add button by its glyph
      if (CJK.test(text.replaceAll('＋', ''))) problems.push(`${key}: Chinese in the English text: ${text}`)
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

Deno.test('aiworkspace i18n: every area file is checked here', () => {
  const listed = new Set(PARTS.map((p) => p.name))
  const files = [...Deno.readDirSync(new URL('../../src/i18n/aiworkspace/', import.meta.url))]
    .filter((e) => e.isFile && e.name.endsWith('.ts') && e.name !== 'index.ts').map((e) => e.name.replace(/\.ts$/, ''))
  const missing = files.filter((name) => !listed.has(name))
  fail(missing, 'area files not in PARTS (import them here with their key prefixes)')
})

for (const locale of LOCALES) {
  Deno.test(`aiworkspace i18n table: ${locale}`, async () => {
    const table = (await import(`../../src/i18n/aiworkspace/locales/${locale}.ts`)).default as Table
    const english: Table = Object.assign({}, ...PARTS.map((p) => p.en))
    const forms: string[] = new Intl.PluralRules(locale).resolvedOptions().pluralCategories.filter((c) => c !== 'other')
    const problems: string[] = []
    for (const [key, text] of Object.entries(english)) {
      if (key.endsWith('_one')) continue
      if (!(key in table)) { problems.push(`${key}: missing`); continue }
      if (!table[key].trim()) problems.push(`${key}: empty`)
      const a = [...new Set(placeholders(text))].join(','), b = [...new Set(placeholders(table[key]))].join(',')
      if (a !== b) problems.push(`${key}: placeholders differ (en: ${a || '-'}; ${locale}: ${b || '-'})`)
    }
    for (const [key, text] of Object.entries(table)) {
      if (key in english && !key.endsWith('_one')) continue
      const m = /^(.*)_(zero|one|two|few|many)$/.exec(key)
      if (!m || !(m[1] in english)) { problems.push(`${key}: not an English key`); continue }
      if (!forms.includes(m[2]) || m[2] === 'many') problems.push(`${key}: ${locale} has no “_${m[2]}” form here (the plain key is the general form)`)
      if (!placeholders(english[m[1]]).includes('count')) problems.push(`${key}: a plural form of a text without {{count}}`)
      if (!text.trim()) problems.push(`${key}: empty`)
      const extra = placeholders(text).filter((p) => !placeholders(english[m[1]]).includes(p))
      if (extra.length) problems.push(`${key}: placeholders ${extra} not in the plain key`)
    }
    fail(problems, `problems in src/i18n/aiworkspace/locales/${locale}.ts`)
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
  for (const file of ['aiws/prompts/analyze.md', 'aiws/prompts/code.md', 'aiws/README.md', 'aiws/renderers.json']) {
    Deno.readTextFileSync(`${ROOT}${BACKEND}/${file}`).split('\n').forEach((line, i) => { if (CJK.test(line)) problems.push(`${BACKEND}/${file}:${i + 1}: ${line.trim().slice(0, 70)}`) })
  }
  for (const file of ['aiws/aiws.js', 'aiws/run.js']) {
    for (const l of literals(`${BACKEND}/${file}`)) if (!l.exempt && CJK.test(l.text)) problems.push(`${l.file}:${l.line}: ${l.text.trim().slice(0, 70)}`)
  }
  fail(problems, 'Chinese texts in the backend (it speaks English; mark input matching i18n-exempt with a reason)')
})
