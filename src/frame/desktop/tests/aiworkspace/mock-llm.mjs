// A scripted OpenAI-compatible model for the wish e2e specs (许愿格 §16.1 "可控 LLM 返回"; 许愿格交互升级 §3.10 C): it plays a
// competent model of the analysis and coding stages (AI Native 第一期 §2.1) through the host's tools — it reads the
// context map, profiles the table the need names, submits an analysis (each program check with its level), writes the
// macro (which declares what it reads and writes the commentary from its own facts), trial-runs it and finishes. What
// the specs judge is the host and the UI, not the text.
//
// Marks in the request inject what a spec needs (they replace the Mock executor's `#fail` / `#invalid` / `#slow`):
//   #slow[=ms]   every answer of the run waits (default 1500 ms): the stage stays under way long enough to watch or cancel
//   #warn        the analysis carries a warning
//   #noinput     the need names no data: unless an input is bound in the wish's properties, the analysis finds none and
//                submits `needs_input` (the run fails, naming the input)
//   #checkwarn   a warning-level program check: every row's value is positive
//   #checkerror  the same check at error level
//   #ai          the macro writes its commentary with `aiws.ai.complete`
//   #codefail    the coding never finishes (the model gives up)
// A coding round on a stored macro (the coding message says one exists) or with a "降序" refinement sorts the summary
// by its total, high to low: a second version of the macro, different code.
import { createServer } from 'node:http'

let calls = 0

function toolCall(name, args) {
  calls += 1
  return { role: 'assistant', content: null, tool_calls: [{ id: `call_${calls}`, type: 'function', function: { name, arguments: JSON.stringify(args) } }] }
}

function say(text) {
  return { role: 'assistant', content: text }
}

function handleOf(line) {
  const m = /@[A-Z][0-9]+/.exec(line ?? '')
  return m ? m[0] : null
}

function sectionOf(user, title) {
  return (new RegExp(`## ${title.replace(/[()]/g, '\\$&')}[^\\n]*\\n\\n([\\s\\S]*?)\\n\\n`).exec(user) ?? [])[1] ?? ''
}

/** The marks of the request (the "Original request" section both stages have). */
export function marks(user) {
  const need = sectionOf(user, 'Original request')
  const slow = /#slow(?:=(\d+))?/.exec(need)
  return {
    need,
    slow: slow ? Number(slow[1] ?? 1500) : 0,
    warn: need.includes('#warn'),
    noinput: need.includes('#noinput'),
    check: need.includes('#checkerror') ? 'error' : need.includes('#checkwarn') ? 'warning' : null,
    ai: need.includes('#ai'),
    codefail: need.includes('#codefail'),
  }
}

/** The model answers in the request's language: Chinese for a Chinese request, else English (the English UI spec). */
const WORDS = {
  zh: {
    missing: '找不到需求里说的数据。', missingTable: '工作区里找不到「成本明细」表', missingLabel: '成本明细', creative: '按需求写一段说明：', note: '说明',
    noData: '需求没有给出数据，说明只能按常识写。', group: '分组', all: '全部', totalCheck: '汇总合计等于输入合计', countCheck: '汇总计数等于输入行数',
    positive: (f) => `每一行的「${f}」都大于 0`, cited: '解读中的数字可追溯',
    task: (t, by, num) => (num ? `读取 ${t} 的全部行，按「${by}」汇总「${num}」；写一段解读，数字取自 facts。` : `读取 ${t} 的全部行，按「${by}」计数；写一段解读，数字取自 facts。`),
    summary: '汇总', commentary: '解读', total: '合计', warn: '没有找到成本列，利润率无法计算。',
    said: (total, groups) => `合计 ' + ${total} + '，共 ' + ${groups} + ' 组。`, writer: '你是解读撰写人：只用给出的数字。',
    written: '按需求写成。', giveUp: '这个宏我写不出来。', done: '完成', sorted: '改为按合计降序', feedback: '按反馈修改：表格按合计降序', refinement: '汇总表按合计降序排列', review: '数字取自 facts',
  },
  en: {
    missing: 'The data the request names cannot be found.', missingTable: 'The workspace has no table "Cost details"', missingLabel: 'Cost details', creative: 'Write a note as asked: ', note: 'Note',
    noData: 'The request gives no data: the note can only follow common sense.', group: 'Group', all: 'All', totalCheck: 'The summary total equals the input total', countCheck: 'The summary count equals the input rows',
    positive: (f) => `Every row's "${f}" is above 0`, cited: 'Every number of the commentary can be traced',
    task: (t, by, num) => (num ? `Read all rows of ${t}, group them by "${by}" and total "${num}"; write a commentary with the numbers from facts.` : `Read all rows of ${t} and count them by "${by}"; write a commentary with the numbers from facts.`),
    summary: 'Summary', commentary: 'Commentary', total: 'Total', warn: 'No cost column was found: the margin cannot be computed.',
    said: (total, groups) => `Total ' + ${total} + ' in ' + ${groups} + ' groups.`, writer: 'You are the commentary writer: use only the numbers given.',
    written: 'Written as asked.', giveUp: 'I cannot write this macro.', done: 'Done', sorted: 'Sorted by the total, high to low', feedback: 'Changed as asked: the table by its total, high to low', refinement: 'The summary is sorted by its total, high to low', review: 'Numbers from facts',
  },
}
const wordsOf = (need) => (/[\u3400-\u9fff]/.test(need) ? WORDS.zh : WORDS.en)

/** The tables of the map's data list. */
function tableLines(user) {
  const data = user.split('## Data\n')[1] ?? ''
  return data.split('\n').filter((l) => / table "/.test(l))
}

/** The table the need talks about: named in the need, else the first table of the map's data list. */
function tableOf(user, m) {
  const lines = tableLines(user)
  const named = lines.find((l) => { const name = / table "(.+?)"/.exec(l)?.[1]; return name && m.need.includes(name) })
  return handleOf(named ?? lines[0])
}

function analysis(msgs, user, tools) {
  const m = marks(user)
  const w = wordsOf(m.need)
  // `#noinput`: the wish's own inputs decide (bound by the user in its properties): the model reads the wish first
  if (m.noinput && tools.length === 0) return toolCall('ws_read', { target: '@W' })
  const table = m.noinput ? boundTable(tools[0], user) : tableOf(user, m)
  const base = m.noinput ? 1 : 0
  if (!table) {
    if (tools.length > base) return say('submitted')
    if (m.noinput) {
      return toolCall('submit_analysis', { analysis: {
        schema_version: 'wish.analysis.v3', status: 'needs_input', context_prompt: w.missing, inputs: [],
        output_contract: { results: [{ name: 'commentary', type: 'richtext', title: w.commentary }] }, checks: [],
        blockers: [{ code: 'NEEDS_INPUT', message: w.missingTable, input_label: w.missingLabel }], warnings: [],
      } })
    }
    // a purely creative request: no data
    return toolCall('submit_analysis', { analysis: {
      schema_version: 'wish.analysis.v3', status: 'ready', context_prompt: `${w.creative}${m.need.replace(/#\S+/g, '').trim()}`, inputs: [],
      output_contract: { results: [{ name: 'commentary', type: 'richtext', title: w.note }] }, checks: [], blockers: [], warnings: m.warn ? [w.noData] : [],
    } })
  }
  if (tools.length === base) return toolCall('ws_profile', { target: table })
  if (tools.length === base + 1) {
    const profile = JSON.parse(tools[base].slice(tools[base].indexOf('{')))
    const fields = profile.fields ?? []
    const num = fields.find((f) => f.type === 'number' || f.type === 'decimal' || f.type === 'currency')
    const cat = fields.find((f) => f.type === 'text' || f.type === 'select')
    const by = cat?.name ?? w.group
    const checks = [{ id: 'total', kind: 'program', level: 'error', text: num ? w.totalCheck : w.countCheck }]
    if (m.check && num) checks.push({ id: 'positive', kind: 'program', level: m.check, text: w.positive(num.name) })
    checks.push({ id: 'cited', kind: 'review', text: w.cited })
    return toolCall('submit_analysis', { analysis: {
      schema_version: 'wish.analysis.v3', status: 'ready', context_prompt: w.task(table, by, num?.name), inputs: [{ name: 'data', ref: table }],
      output_contract: { results: [
        { name: 'summary', type: 'table', title: w.summary, key: [by],
          views: [{ renderer: 'table' }, { renderer: 'sample.bar-chart', config: { value: w.total, by } }] },
        { name: 'commentary', type: 'richtext', title: w.commentary }] },
      checks, blockers: [], warnings: m.warn ? [w.warn] : [],
    } })
  }
  return say('submitted')
}

/** `#noinput`: the table the wish has bound, by the label `ws_read @W` lists its inputs with (`- input <label>: …`). */
function boundTable(read, user) {
  const labels = [...(read ?? '').matchAll(/^- input (.+?):/gm)].map((x) => x[1])
  const line = tableLines(user).find((l) => labels.includes(/ table "(.+?)"/.exec(l)?.[1]))
  return line ? handleOf(line) : null
}

function program({ cat, num, desc, check, ai, w }) {
  const c = JSON.stringify(cat)
  const n = num ? JSON.stringify(num) : null
  const t = JSON.stringify(w.total)
  const all = JSON.stringify(w.all)
  const value = n ? `(r[${n}] ?? 0)` : '1'
  const fields = [c, n].filter(Boolean).join(', ')
  const declare = [`inputs: { data: [${fields}] }`, ...(ai ? ['ai: { complete: { max_calls: 2, max_tokens: 200 } }'] : [])].join(', ')
  const said = `'${w.said('total', 'out.length')}'`
  return `export default async function main(aiws) {
  aiws.declare({ ${declare} })
  const rows = aiws.input('data').rows({ fields: [${fields}] })
  const by = new Map()
  for (const r of rows) by.set(r[${c}] ?? ${all}, (by.get(r[${c}] ?? ${all}) ?? 0) + ${value})
  const out = [...by].map(([k, v]) => ({ ${c}: k, [${t}]: Math.round(v * 100) / 100 }))${desc ? `.sort((a, b) => b[${t}] - a[${t}])` : ''}
  aiws.result.table('summary', out, { key: [${c}], fields: { ${c}: 'text', [${t}]: 'decimal' } })
  const total = Math.round(rows.reduce((s, r) => s + ${value}, 0) * 100) / 100
  aiws.facts({ total, groups: out.length })
  aiws.check('total', Math.abs(out.reduce((s, r) => s + r[${t}], 0) - total) < 0.01, { total })
${check && n ? `  const bad = rows.filter((r) => !((r[${n}] ?? 0) > 0))\n  aiws.check('positive', bad.length === 0, { rows: bad.length })\n` : ''}${ai
    ? `  const said = await aiws.ai.complete({ system: ${JSON.stringify(w.writer)}, prompt: ${said}, max_tokens: 100 })
  aiws.result.text('commentary', '## ${w.commentary}\\n\\n' + said.text)
`
    : `  aiws.result.text('commentary', '## ${w.commentary}\\n\\n' + ${said})
`}}
`
}

function creative(w) {
  return `export default async function main(aiws) {
  aiws.declare({ inputs: {} })
  aiws.result.text('commentary', '## ${w.note}\\n\\n${w.written}')
}
`
}

function coding(msgs, user, tools) {
  const m = marks(user)
  const w = wordsOf(m.need)
  if (m.codefail) return say(w.giveUp)
  const contract = JSON.parse((/## Output contract[^\n]*\n\n```json\n([\s\S]*?)\n```/.exec(user) ?? [])[1] ?? '{}')
  const checks = JSON.parse((/## Acceptance checks[^\n]*\n\n```json\n([\s\S]*?)\n```/.exec(user) ?? [])[1] ?? '[]')
  const table = contract.results?.find((r) => r.type === 'table')
  const key = table?.key?.[0] ?? w.group
  const task = sectionOf(user, 'Task description (context prompt)')
  const num = (/按「.+?」汇总「(.+?)」/.exec(task) ?? / and total "(.+?)"/.exec(task) ?? [])[1] ?? null
  const feedback = user.includes('## User feedback')
  const stored = /program\/main\.js already has a macro/.test(user)
  const desc = feedback || stored || sectionOf(user, 'Refinements (must be followed)').includes('降序')
  const check = checks.some((c) => c.id === 'positive')
  // the macro is trial-run by the executor (run_program's `source`); there is no way to write a result directly
  const steps = ['run', 'check', 'finish', 'done']
  const step = steps[tools.length] ?? 'done'
  if (step === 'run') return toolCall('run_program', { source: table ? program({ cat: key, num, desc, check, ai: m.ai, w }) : creative(w) })
  if (step === 'check') return toolCall('check_results', {})
  if (step === 'finish') {
    const args = { summary: feedback ? w.feedback : desc ? w.sorted : w.done, review_notes: checks.filter((c) => c.kind === 'review').map((c) => ({ id: c.id, note: w.review })) }
    if (feedback) args.refinements = [w.refinement]
    return toolCall('finish', args)
  }
  return say(w.done)
}

function mapJudge(user) {
  const items = JSON.parse((/Input items \(a JSON array of \d+ items\):\n([\s\S]*?)\n\n/.exec(user) ?? [])[1] ?? '[]')
  return say(JSON.stringify(items.map((i) => (String(i).includes('好') ? '正面' : '负面'))))
}

export function startMockLlm(port) {
  const server = createServer((req, res) => {
    // the browser's local executor calls this endpoint cross-origin
    res.setHeader('Access-Control-Allow-Origin', '*')
    res.setHeader('Access-Control-Allow-Headers', 'content-type, authorization')
    if (req.method === 'OPTIONS') { res.writeHead(204); res.end(); return }
    let body = ''
    req.on('data', (c) => { body += c })
    req.on('end', () => {
      let r = {}
      let message
      let wait = 0
      try {
        r = JSON.parse(body)
        const msgs = r.messages ?? []
        const text = (m) => (typeof m.content === 'string' ? m.content : Array.isArray(m.content) ? m.content.map((p) => p.text ?? '').join('') : '')
        const system = msgs.filter((m) => m.role === 'system').map(text).join('\n')
        const user = msgs.filter((m) => m.role === 'user').map(text).join('\n')
        const tools = msgs.filter((m) => m.role === 'tool').map(text)
        if (system.includes('item-by-item judge')) message = mapJudge(user)
        else if (system.includes('解读撰写人') || system.includes('commentary writer')) message = say(`（模型写的解读）${user}`)
        else {
          wait = marks(user).slow
          message = system.includes('the **analysis stage**') ? analysis(msgs, user, tools) : coding(msgs, user, tools)
        }
      } catch (error) {
        res.writeHead(500, { 'content-type': 'application/json' })
        res.end(JSON.stringify({ error: { message: String(error?.stack ?? error) } }))
        return
      }
      setTimeout(() => {
        if (res.destroyed) return
        res.writeHead(200, { 'content-type': 'application/json' })
        // the model that answered is named by the provider (the record shows it rather than the logical name)
        res.end(JSON.stringify({ id: `cmpl_${Date.now()}`, object: 'chat.completion', model: `${r.model ?? 'scripted'}@mock-llm`, choices: [{ index: 0, message, finish_reason: message.tool_calls ? 'tool_calls' : 'stop' }], usage: { prompt_tokens: 1200, completion_tokens: 80, total_tokens: 1280 } }))
      }, wait)
    })
  })
  server.listen(Number(port), '127.0.0.1')
  return server
}
