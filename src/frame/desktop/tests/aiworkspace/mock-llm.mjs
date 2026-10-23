// A scripted OpenAI-compatible model for the wish e2e specs (许愿格 §16.1 "可控 LLM 返回"): it plays a
// competent model of the two stages through the host's tools — it reads the context map, profiles
// the table the need names, submits an analysis, writes a program, runs it, writes the commentary
// from the program's facts and finishes. What the specs judge is the host and the UI, not the text.
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

/** The table the need talks about: named in the need, else the first table of the map's data list. */
function tableLine(user) {
  const need = (/## Original request\n\n([\s\S]*?)\n\n/.exec(user) ?? [])[1] ?? ''
  const data = user.split('## Data\n')[1] ?? ''
  const lines = data.split('\n').filter((l) => / table "/.test(l))
  return lines.find((l) => { const name = / table "(.+?)"/.exec(l)?.[1]; return name && need.includes(name) }) ?? lines[0]
}

function analysis(msgs, user, tools) {
  const line = tableLine(user)
  const table = handleOf(line)
  if (tools.length === 0) return toolCall('ws_profile', { target: table })
  const profile = JSON.parse(tools[0].slice(tools[0].indexOf('{')))
  const fields = profile.fields ?? []
  const num = fields.find((f) => f.type === 'number' || f.type === 'decimal')
  const cat = fields.find((f) => f.type === 'text' || f.type === 'select')
  if (tools.length === 1) {
    return toolCall('submit_analysis', { analysis: {
      schema_version: 'wish.analysis.v2', status: 'ready',
      context_prompt: `读取 ${table} 的全部行，按「${cat.name}」汇总「${num.name}」；写一段解读，数字取自 facts。`,
      inputs: [{ name: 'data', ref: table }],
      output_contract: { results: [
        { name: 'summary', type: 'table', title: '汇总', approach: 'program', key: [cat.name],
          views: [{ renderer: 'table' }, { renderer: 'sample.bar-chart', config: { value: '合计', by: cat.name } }] },
        { name: 'commentary', type: 'richtext', title: '解读', approach: 'direct' }] },
      checks: [{ id: 'total', kind: 'program', text: '汇总合计等于输入合计' }, { id: 'cited', kind: 'review', text: '解读中的数字可追溯' }],
      blockers: [], warnings: [],
    } })
  }
  return say('分析已提交。')
}

function program(cat, num, desc) {
  return `export default async function main(aiws) {
  const rows = aiws.input('data').rows({ fields: [${JSON.stringify(cat)}, ${JSON.stringify(num)}] })
  const by = new Map()
  for (const r of rows) by.set(r[${JSON.stringify(cat)}], (by.get(r[${JSON.stringify(cat)}]) ?? 0) + (r[${JSON.stringify(num)}] ?? 0))
  const out = [...by].map(([k, v]) => ({ ${JSON.stringify(cat)}: k, 合计: Math.round(v * 100) / 100 }))${desc ? '.sort((a, b) => b.合计 - a.合计)' : ''}
  aiws.result.table('summary', out, { key: [${JSON.stringify(cat)}], fields: { ${JSON.stringify(cat)}: 'text', 合计: 'decimal' } })
  const total = Math.round(rows.reduce((s, r) => s + (r[${JSON.stringify(num)}] ?? 0), 0) * 100) / 100
  aiws.facts({ total, groups: out.length })
  aiws.check('total', Math.abs(out.reduce((s, r) => s + r.合计, 0) - total) < 0.01, { total })
}
`
}

function execution(msgs, user, tools, fileTools) {
  const contract = JSON.parse((/## Output contract\n\n```json\n([\s\S]*?)\n```/.exec(user) ?? [])[1] ?? '{}')
  const key = contract.results?.[0]?.key?.[0] ?? '分组'
  const task = (/## Task description \(context prompt\)\n\n([\s\S]*?)\n\n/.exec(user) ?? [])[1] ?? ''
  const num = (/按「.+?」汇总「(.+?)」/.exec(task) ?? [])[1] ?? '销售额'
  const feedback = user.includes('## User feedback')
  const hasProgram = user.includes('program/main.js already has a program')
  const steps = []
  // the service's stage has file tools (the model writes program/main.js first); an executor outside the
  // service (the browser, T3-4) takes the program as run_program's `source`
  if (fileTools && (!hasProgram || feedback)) steps.push('write')
  steps.push('run', 'put', 'check', 'finish', 'done')
  const step = steps[tools.length] ?? 'done'
  if (step === 'write') return toolCall('write_file', { path: 'program/main.js', content: program(key, num, feedback) })
  if (step === 'run') return toolCall('run_program', fileTools ? {} : { source: program(key, num, feedback) })
  if (step === 'put') {
    const runOut = tools.find((t) => t.includes('\nfacts: ')) ?? ''
    const facts = JSON.parse((/\nfacts: (\{.*\})/.exec(runOut) ?? [])[1] ?? '{}')
    return toolCall('put_result', { name: 'commentary', type: 'text', markdown: `## 解读\n\n合计 ${facts.total}，共 ${facts.groups} 组。` })
  }
  if (step === 'check') return toolCall('check_results', {})
  if (step === 'finish') {
    const args = { summary: feedback ? '按反馈修改：表格按合计降序' : '完成', review_notes: [{ id: 'cited', note: '数字取自 facts' }] }
    if (feedback) args.refinements = ['汇总表按合计降序排列']
    return toolCall('finish', args)
  }
  return say('完成。')
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
      try {
        const r = JSON.parse(body)
        const msgs = r.messages ?? []
        const text = (m) => (typeof m.content === 'string' ? m.content : Array.isArray(m.content) ? m.content.map((p) => p.text ?? '').join('') : '')
        const system = msgs.filter((m) => m.role === 'system').map(text).join('\n')
        const user = msgs.filter((m) => m.role === 'user').map(text).join('\n')
        const tools = msgs.filter((m) => m.role === 'tool').map(text)
        const fileTools = (r.tools ?? []).some((t) => (t.function?.name ?? t.name) === 'write_file')
        const message = system.includes('item-by-item judge') ? mapJudge(user) : system.includes('the **analysis stage**') ? analysis(msgs, user, tools) : execution(msgs, user, tools, fileTools)
        res.writeHead(200, { 'content-type': 'application/json' })
        res.end(JSON.stringify({ id: `cmpl_${Date.now()}`, object: 'chat.completion', model: r.model, choices: [{ index: 0, message, finish_reason: message.tool_calls ? 'tool_calls' : 'stop' }], usage: { prompt_tokens: 100, completion_tokens: 20, total_tokens: 120 } }))
      } catch (error) {
        res.writeHead(500, { 'content-type': 'application/json' })
        res.end(JSON.stringify({ error: { message: String(error?.stack ?? error) } }))
      }
    })
  })
  server.listen(Number(port), '127.0.0.1')
  return server
}
