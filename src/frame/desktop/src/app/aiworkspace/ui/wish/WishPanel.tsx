/* The wish as the user works with it (许愿格详细设计 §14.1): the need, long-lived knowledge and
 * refinements; inputs (what is read and in which range); the analysis — context prompt, output
 * contract, acceptance checks, blockers; 分析 / 执行 / 只重跑程序 / 让 AI 修程序 / 取消; progress while
 * the service works; the candidate with its preview; and the state of the applied results. Shown by
 * the data-source detail and by the wish Block after activation. */

import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from 'react'
import { describeError, type ReadOk } from '../../api/session'
import type { Json, WishInput, WishPayloadRead, WishRunView } from '../../api/types'
import { EDIT_STATE_LABEL } from '../../state/edits'
import { useEdit, useEntity, useFreshness, useLoad, useOutlineVersion, useStore, useVersion, useWorkspaceUi } from '../../state/hooks'
import { consumeIntent, useIntent } from '../blocks/editorToolbar'
import { FreshnessBadge } from '../sources/FreshnessBadge'
import { WishCandidate } from './WishCandidate'
import { isActive, isWaiting, type WishBusy } from './WishService'

const BUSY_LABEL: Record<WishBusy, string> = { analyzing: '分析中…', executing: '执行中…', rerunning: '重跑程序中…', repairing: '修复程序中…', previewing: '预览中…', applying: '应用中…', cancelling: '取消中…' }
const PHASE_LABEL: Record<string, string> = { queued: '排队中', snapshotting: '固定数据快照', running: '运行中', validating: '校验结果', waiting_confirmation: '待确认', applying: '应用中' }
const ACTIVITY_LABEL: Record<string, string> = { reading: '阅读数据', writing_program: '编写程序', running_program: '运行程序', shell: '运行命令', writing: '撰写结果', checking: '自查', submitting: '提交分析', working: '处理中' }
const STATUS_LABEL: Record<string, string> = { current: '最新', stale: '需要刷新', upstream_stale: '上游需要刷新', unavailable: '引用不可用', unknown: '无法确认', none: '无' }

function scopeText(input: WishInput, title: (id: string) => string): string {
  const s = input.selector as { kind?: string; cell_id?: string; filter?: Json; fields?: string[] } | undefined
  if (!s || s.kind === 'entity') return '整体'
  if (s.kind === 'table_view') return `按视图「${title(s.cell_id ?? '')}」读取`
  if (s.kind === 'table_query') return `查询${s.fields ? `（${s.fields.length} 列）` : ''}${s.filter ? '，带筛选' : ''}`
  return s.kind ?? ''
}

function Progress({ run, onCancel }: { run: WishRunView; onCancel: () => void }) {
  const p = run.progress ?? {}
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => { const t = window.setInterval(() => setNow(Date.now()), 1000); return () => window.clearInterval(t) }, [])
  const elapsed = Math.max(0, Math.round((now - Date.parse(run.created_at)) / 1000))
  const doing = p.waiting_model ? '等待模型' : ACTIVITY_LABEL[p.activity ?? ''] ?? ''
  return (
    <div className="aiws-wish-progress" data-testid="aiws-wish-progress" data-phase={p.phase ?? run.state}>
      <span className="aiws-chip">{PHASE_LABEL[p.phase ?? run.state] ?? p.phase ?? run.state}</span>
      {doing && <span>{doing}</span>}
      <span className="aiws-muted">
        {elapsed}s{p.tool_calls ? ` · 工具 ${p.tool_calls} 次` : ''}{p.program_runs ? ` · 程序运行 ${p.program_runs} 次` : ''}
        {p.llm_map ? ` · 逐项判断 ${p.llm_map.items} 项（缓存 ${p.llm_map.cached}）` : ''}{p.model ? ` · ${p.model}` : ''}
      </span>
      {p.last_command && <code className="aiws-muted" title={p.last_command}>{p.last_command.slice(0, 60)}</code>}
      <button type="button" data-testid="aiws-wish-cancel" onClick={onCancel}>取消</button>
    </div>
  )
}

function ProgramView({ wishId, canEdit }: { wishId: string; canEdit: boolean }) {
  const store = useStore()
  const [source, setSource] = useState<string | null>(null)
  const [draft, setDraft] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const load = async () => {
    setError(null)
    try { setSource(await store.wish.programSource(wishId) ?? '') } catch (e) { setError(describeError(e)) }
  }
  const save = async () => {
    if (draft === null) return
    try { await store.wish.saveProgram(wishId, draft); setSource(draft); setDraft(null) } catch (e) { setError(describeError(e)) }
  }
  if (source === null) return <button type="button" className="aiws-link" data-testid="aiws-wish-program" onClick={() => { void load() }}>查看程序</button>
  return (
    <div className="aiws-wish-program" data-testid="aiws-wish-program-view">
      {draft === null ? <pre className="aiws-wish-context">{source || '（没有程序）'}</pre> : <textarea data-testid="aiws-wish-program-edit" rows={14} value={draft} onChange={(e) => setDraft(e.target.value)} spellCheck={false} />}
      {error && <div className="aiws-error">{error}</div>}
      <div className="aiws-inline-form">
        {canEdit && draft === null && <button type="button" onClick={() => setDraft(source)}>编辑</button>}
        {draft !== null && <button type="button" data-testid="aiws-wish-program-save" onClick={() => { void save() }}>保存（结果将显示为需要刷新）</button>}
        {draft !== null && <button type="button" onClick={() => setDraft(null)}>放弃修改</button>}
        <button type="button" onClick={() => { setSource(null); setDraft(null) }}>收起</button>
      </div>
    </div>
  )
}

/** `canvasSelection`: what is selected on the canvas while the wish is open in the right panel — offered as inputs.
 * A "start" sent to `cellId` (the Block's "run") runs the next step once the wish is loaded. */
export function WishPanel({ wishId, readOnly, compact, cellId, canvasSelection }: { wishId: string; readOnly: boolean; compact?: boolean; cellId?: string; canvasSelection?: string[] }) {
  const store = useStore()
  const ui = useWorkspaceUi()
  const entity = useEntity(wishId)
  const version = useVersion(`e:${wishId}`)
  const load = useCallback(() => store.session.read<WishPayloadRead>(wishId), [store, wishId])
  const read = useLoad<ReadOk<WishPayloadRead>>(load, version)
  useSyncExternalStore(store.wish.subscribe, store.wish.snapshot)
  useOutlineVersion()
  const freshness = useFreshness(wishId)
  const [prompt, setPrompt] = useState<string | null>(null)
  const [knowledge, setKnowledge] = useState<string | null>(null)
  const [message, setMessage] = useState<{ kind: 'info' | 'error'; text: string } | null>(null)
  const [showKnowledge, setShowKnowledge] = useState(false)
  const editEntry = useEdit(`wish:${wishId}:apply`)
  const startIntent = useIntent(cellId ?? '')
  const startsDone = useRef(0)
  useEffect(() => { void store.wish.restore(wishId) }, [store, wishId])
  // "run" from the canvas: the next step of the two passes — analyse first, execute once the analysis is ready
  useEffect(() => {
    const payload = read.data?.content.payload
    if (!cellId || startIntent?.value !== 'start' || startIntent.seq <= startsDone.current || !payload || !entity || freshness === undefined) return
    startsDone.current = startIntent.seq
    consumeIntent(cellId, startIntent)
    if (readOnly || !entity.capabilities.includes('update') || isActive(store.wish.run(wishId)) || store.wish.busy.get(wishId)) return
    const analysis = payload.analysis
    const work = (freshness.needs_analysis ?? !analysis) || analysis?.status !== 'ready'
      ? store.wish.analyze(wishId, cellId).then(() => setMessage({ kind: 'info', text: '分析已写回。' }))
      : store.wish.execute(wishId, { cellId })
    work.catch((error: unknown) => setMessage({ kind: 'error', text: describeError(error) }))
  }, [startIntent, read.data, entity, freshness, readOnly, store, wishId, cellId])
  const run = store.wish.run(wishId)
  const busy = store.wish.busy.get(wishId)
  const flowError = store.wish.errors.get(wishId)
  if (read.error && !read.data) return <div className="aiws-error" role="alert">无法读取许愿格：{read.error}</div>
  if (!read.data || !entity) return <div className="aiws-muted">载入许愿格…</div>
  const payload = read.data.content.payload
  const keyRevs = read.data.content.key_revs
  const canEdit = !readOnly && entity.capabilities.includes('update')
  const analysis = payload.analysis
  const needsAnalysis = freshness?.needs_analysis ?? !analysis
  const ready = analysis?.status === 'ready' && !needsAnalysis
  const inputs = payload.inputs ?? []
  const title = (id: string) => { const e = store.outline.get(id); return e?.title ?? e?.name ?? id }
  const running = isActive(run) || busy === 'analyzing' || busy === 'executing' || busy === 'rerunning' || busy === 'repairing'
  const act = async (work: () => Promise<unknown>) => {
    setMessage(null)
    try { await work() } catch (error) { setMessage({ kind: 'error', text: describeError(error) }) }
  }
  const setKey = (key: string, value: Json | null, label: string) => act(async () => {
    const op = value === null
      ? { op: 'entity.unset_keys' as const, entity_id: wishId, keys: [{ key, expect: { rev: keyRevs[key] ?? 0 } }] }
      : { op: 'entity.set_keys' as const, entity_id: wishId, keys: [{ key, value, expect: { rev: keyRevs[key] ?? 0 } }] }
    const outcome = await store.submit({ editId: `wish:${wishId}:${key}`, label, operations: [op] })
    if (outcome.status === 'conflict' || outcome.status === 'rejected') throw new Error(`没有保存（${outcome.code}）`)
  })
  const savePrompt = () => {
    if (prompt === null || prompt === payload.prompt) { setPrompt(null); return }
    void setKey('prompt', prompt, '修改提示词').then(() => setPrompt(null))
  }
  const saveKnowledge = () => {
    if (knowledge === null || knowledge === (payload.knowledge ?? '')) { setKnowledge(null); return }
    void setKey('knowledge', knowledge.trim() ? knowledge : null, '修改许愿格知识').then(() => setKnowledge(null))
  }
  const setInputs = (next: WishInput[], label: string) => setKey('inputs', next as unknown as Json, label)
  const addInput = (entityId: string) => {
    if (!entityId || inputs.some((input) => input.entity_id === entityId)) return
    void setInputs([...inputs, { entity_id: entityId, label: title(entityId), version: { mode: 'follow' } }], '添加输入')
  }
  const removeRefinement = (index: number) => {
    const next = (payload.refinements ?? []).filter((_, i) => i !== index)
    void setKey('refinements', next.length ? (next as unknown as Json) : null, '删除修改意见')
  }
  const editRefinement = (index: number, text: string) => {
    const next = (payload.refinements ?? []).map((r, i) => (i === index ? { ...r, text } : r))
    void setKey('refinements', next as unknown as Json, '修改修改意见')
  }
  const analyze = () => act(async () => { await store.wish.analyze(wishId, cellId); setMessage({ kind: 'info', text: '分析已写回。' }) })
  const execute = () => act(() => store.wish.execute(wishId, { cellId }))
  const selectedInputs = canEdit ? [...new Set((canvasSelection ?? []).map((id) => { const e = store.outline.get(id); return e?.type_id === 'buckyos.cell' ? e.source_id ?? null : e && e.type_id !== 'buckyos.container' ? e.entity_id : null })
    .filter((id): id is string => Boolean(id) && id !== wishId && !inputs.some((input) => input.entity_id === id)))] : []
  const rerun = () => act(() => store.wish.rerunProgram(wishId, cellId))
  const repair = () => act(async () => { if (run) await store.wish.repairProgram(wishId, run.run_id, cellId) })
  const cancel = () => act(async () => { if (run) await store.wish.cancel(run) })
  const candidates = store.outline.all().filter((e) => !e.deleted && e.entity_id !== wishId && e.type_id !== 'buckyos.cell' && e.type_id !== 'buckyos.wish' && e.type_id !== 'buckyos.block-def' && (e.type_id !== 'buckyos.container' || e.kind === 'folder') && store.outline.ancestors(e.entity_id).includes('data') && e.entity_id !== 'canvas-content')
  const lastRun = payload.last_run
  const programFailed = run?.stage === 'rerun_program' && run.state === 'failed' && run.error?.sub_code === 'PROGRAM_FAILED'
  const contract = analysis?.output_contract.results ?? []
  return (
    <div className={`aiws-wish${compact ? ' is-compact' : ''}`} data-testid={`aiws-wish-${wishId}`} data-busy={busy ?? (running ? 'running' : '')} data-needs-analysis={needsAnalysis ? 'true' : 'false'}>
      <div className="aiws-wish-head">
        <select data-testid="aiws-wish-executor" aria-label="执行器" value={payload.executor} disabled={!canEdit || running} onChange={(e) => { void setKey('executor', e.target.value, '切换执行器') }}>
          <option value="xllm">模型执行器（xllm）</option>
          <option value="mock">模拟执行器</option>
        </select>
        <FreshnessBadge entityId={wishId} />
        {busy && <span className="aiws-muted" data-testid="aiws-wish-busy">{BUSY_LABEL[busy]}</span>}
      </div>
      <label className="aiws-wish-prompt">
        <span className="aiws-muted">需求</span>
        <textarea data-testid="aiws-wish-prompt" rows={compact ? 2 : 3} value={prompt ?? payload.prompt} disabled={!canEdit} onChange={(event) => setPrompt(event.target.value)} onBlur={savePrompt} />
      </label>
      <div className="aiws-wish-knowledge">
        <button type="button" className="aiws-link" data-testid="aiws-wish-knowledge-toggle" onClick={() => setShowKnowledge((v) => !v)}>{showKnowledge ? '▾' : '▸'} 知识（口径、背景、偏好）{payload.knowledge ? '' : '：未填写'}</button>
        {showKnowledge && <textarea data-testid="aiws-wish-knowledge" rows={3} placeholder="如：销售额指含税金额；季度按自然季度；金额单位为元" value={knowledge ?? payload.knowledge ?? ''} disabled={!canEdit} onChange={(e) => setKnowledge(e.target.value)} onBlur={saveKnowledge} />}
        {!showKnowledge && payload.knowledge && <div className="aiws-muted aiws-wish-oneline">{payload.knowledge}</div>}
      </div>
      {(payload.refinements?.length ?? 0) > 0 && (
        <div className="aiws-wish-section" data-testid="aiws-wish-refinements">
          <div className="aiws-muted">修改意见（以后每次都会遵守；修改后需要重新分析）</div>
          {payload.refinements?.map((r, i) => (
            <div key={`${i}:${r.text}`} className="aiws-inline-form" data-testid="aiws-wish-refinement">
              <input defaultValue={r.text} disabled={!canEdit} onBlur={(e) => { if (e.target.value.trim() && e.target.value !== r.text) editRefinement(i, e.target.value) }} />
              {canEdit && <button type="button" className="aiws-icon" aria-label="删除修改意见" onClick={() => removeRefinement(i)}>×</button>}
            </div>
          ))}
        </div>
      )}
      <div className="aiws-wish-section">
        <div className="aiws-muted">输入 {needsAnalysis && <span className="aiws-chip aiws-chip-warn" data-testid="aiws-wish-needs-analysis">需要重新分析</span>}</div>
        {inputs.length === 0 && <div className="aiws-muted">尚无输入：点“分析”由执行器从需求、画布位置和可见数据中选取，或手动添加。</div>}
        <ul className="aiws-wish-inputs">
          {inputs.map((input, index) => {
            const target = store.outline.get(input.entity_id)
            const problem = freshness?.input_problems?.find((p) => p.entity_id === input.entity_id)
            return (
              <li key={`${input.entity_id}:${index}`} data-testid="aiws-wish-input" data-problem={problem?.reason ?? ''}>
                {input.name && <code>{input.name}</code>}
                <button type="button" className="aiws-link" onClick={() => ui.openEntity(input.entity_id)}>{input.label ?? target?.title ?? target?.name ?? input.entity_id}</button>
                <span className="aiws-muted">{target?.type_id.replace('buckyos.', '') ?? '?'} · {scopeText(input, title)}{input.appended_by ? ' · 执行时追加' : ''}</span>
                {problem && <span className="aiws-error" data-testid="aiws-wish-input-problem">{problem.reason === 'missing' ? '不存在' : problem.reason === 'view_missing' ? '视图已删除' : '不可读'}</span>}
                {canEdit && <button type="button" className="aiws-icon" aria-label="移除输入" onClick={() => { void setInputs(inputs.filter((_, i) => i !== index), '移除输入') }}>×</button>}
              </li>
            )
          })}
        </ul>
        {selectedInputs.length > 0 && (
          <button type="button" data-testid="aiws-wish-add-selection" onClick={() => { void setInputs([...inputs, ...selectedInputs.map((id) => ({ entity_id: id, label: title(id), version: { mode: 'follow' as const } }))], '添加画布上选中的对象为输入') }}>
            添加画布上选中的 {selectedInputs.length} 项为输入
          </button>
        )}
        {canEdit && (
          <select aria-label="添加输入" value="" data-testid="aiws-wish-add-input" onChange={(event) => addInput(event.target.value)}>
            <option value="">添加输入…</option>
            {candidates.map((e) => <option key={e.entity_id} value={e.entity_id}>{e.title ?? e.name ?? e.entity_id}（{e.type_id.replace('buckyos.', '')}）</option>)}
          </select>
        )}
      </div>
      {analysis && (
        <div className="aiws-wish-section" data-testid="aiws-wish-analysis" data-status={analysis.status}>
          <div className="aiws-muted">
            分析 · {analysis.status === 'ready' ? '就绪' : '缺少输入'}{analysis.executor === 'mock' ? ' · 模拟执行器' : ''}{analysis.at ? ` · ${new Date(analysis.at).toLocaleString()}` : ''}
          </div>
          {analysis.blockers.length > 0 && (
            <div className="aiws-error" data-testid="aiws-wish-blockers">
              {analysis.blockers.map((b, i) => (
                <div key={i} data-testid="aiws-wish-blocker">
                  {b.message}
                  {(b.candidates ?? []).map((c) => <button key={c} type="button" className="aiws-link" disabled={!canEdit} onClick={() => addInput(c)} title="添加为输入，然后重新分析">{title(c)}</button>)}
                </div>
              ))}
            </div>
          )}
          <details open={!compact}>
            <summary className="aiws-muted">任务说明（context 提示词）</summary>
            <div className="aiws-wish-context">{analysis.context_prompt}</div>
          </details>
          {contract.length > 0 && (
            <table className="aiws-rt-table aiws-wish-contract" data-testid="aiws-wish-contract">
              <thead><tr><th>结果</th><th>类型</th><th>方式</th><th>视图</th></tr></thead>
              <tbody>{contract.map((r) => (
                <tr key={r.name}><td><code>{r.name}</code> {r.title}</td><td>{r.type}{r.key ? `（键 ${r.key.join('+')}）` : ''}{r.target ? `→ ${r.target}` : ''}</td><td>{r.approach === 'program' ? '程序' : '书写'}</td><td>{(r.views ?? []).map((v) => v.renderer).join('、') || '默认'}</td></tr>
              ))}</tbody>
            </table>
          )}
          {analysis.checks.length > 0 && (
            <ul className="aiws-wish-checklist" data-testid="aiws-wish-checkdefs">{analysis.checks.map((c) => <li key={c.id}>{c.kind === 'program' ? '程序检查' : '人工确认'}：{c.text}</li>)}</ul>
          )}
          {analysis.warnings.length > 0 && <div className="aiws-warning">{analysis.warnings.join('；')}</div>}
        </div>
      )}
      <div className="aiws-inline-form">
        <label>输出方式
          <select data-testid="aiws-wish-mode" value={payload.output_mode ?? 'overwrite'} disabled={!canEdit} onChange={(event) => { void setKey('output_mode', event.target.value === 'new' ? 'new' : 'overwrite', `输出方式 → ${event.target.value === 'new' ? '新建' : '覆盖'}`) }}>
            <option value="overwrite">覆盖到固定名称（经版本历史回滚）</option>
            <option value="new">每次新建一组</option>
          </select>
        </label>
        <button type="button" data-testid="aiws-wish-analyze" disabled={!canEdit || running || Boolean(busy)} onClick={() => { void analyze() }}>分析</button>
        <button type="button" data-testid="aiws-wish-execute" disabled={!canEdit || running || Boolean(busy) || !ready} title={!analysis ? '先分析' : needsAnalysis ? '分析之后需求或输入变了：先重新分析' : analysis.status !== 'ready' ? '分析结论是缺少输入' : '编写并运行程序，生成候选'} onClick={() => { void execute() }}>执行</button>
        {payload.program && payload.executor === 'xllm' && <button type="button" data-testid="aiws-wish-rerun" disabled={!canEdit || running || Boolean(busy)} title="用当前数据只重跑程序，不调用模型" onClick={() => { void rerun() }}>只重跑程序</button>}
        {programFailed && <button type="button" data-testid="aiws-wish-repair" disabled={!canEdit || running || Boolean(busy)} onClick={() => { void repair() }}>让 AI 修程序</button>}
      </div>
      {run && isActive(run) && <Progress run={run} onCancel={() => { void cancel() }} />}
      {(message || flowError) && (
        <div className={message?.kind === 'info' ? 'aiws-warning' : 'aiws-error'} role={message?.kind === 'info' ? 'status' : 'alert'} data-testid="aiws-wish-message">{message?.text ?? flowError}</div>
      )}
      {run && !isActive(run) && run.state === 'failed' && !message && !flowError && <div className="aiws-error" data-testid="aiws-wish-message">运行失败：{run.error?.detail ?? run.state}</div>}
      {editEntry && editEntry.state !== 'committed' && <div className={`aiws-state aiws-state-${editEntry.state}`} data-testid="aiws-wish-apply-state">{EDIT_STATE_LABEL[editEntry.state]}{editEntry.detail ? `：${editEntry.detail}` : ''}</div>}
      {run && isWaiting(run) && <WishCandidate run={run} canApply={canEdit} onMessage={(kind, text) => setMessage({ kind, text })} />}
      {lastRun && (
        <div className="aiws-wish-section" data-testid="aiws-wish-last-run">
          <div className="aiws-muted">上次应用 {lastRun.mode === 'program' ? '（只重跑程序）' : ''}{lastRun.at ? ` · ${new Date(lastRun.at).toLocaleString()}` : ''}{lastRun.simulated ? ' · 模拟' : ''}{lastRun.checks ? ` · 检查 ${lastRun.checks.passed} 通过${lastRun.checks.failed ? `，${lastRun.checks.failed} 未通过` : ''}` : ''}</div>
          <div>
            {(freshness?.results?.length ? freshness.results : Object.entries(lastRun.result_bindings?.results ?? {}).map(([name, b]) => ({ name, entity_id: b.entity_id, status: 'unknown' as const, approach: b.approach }))).map((r) => (
              <button key={r.name} type="button" className="aiws-link" data-testid="aiws-wish-produced" data-status={r.status} onClick={() => { if (r.entity_id) ui.openEntity(r.entity_id) }}>
                {lastRun.result_bindings?.results[r.name]?.title ?? r.name}<span className="aiws-muted">（{STATUS_LABEL[r.status] ?? r.status}{r.approach === 'direct' ? '·书写' : ''}）</span>
              </button>
            ))}
          </div>
          {freshness?.direct_stale && <div className="aiws-warning" data-testid="aiws-wish-direct-stale">程序结果已更新，文字部分需要重新生成（点“执行”）。</div>}
          {(lastRun.missing ?? []).length > 0 && (
            <div className="aiws-warning" data-testid="aiws-wish-missing">上次存在、本次未生成：{lastRun.missing?.map((id) => title(id)).join('、')}（未自动删除，请自行处理）</div>
          )}
          {freshness && freshness.status !== 'none' && freshness.status !== 'current' && !freshness.direct_stale && (
            <div className="aiws-warning" data-testid="aiws-wish-stale">{freshness.config_changed ? '许愿格的配置（需求、知识、修改意见、输入或程序）在生成之后改变了：需要刷新' : freshness.status === 'stale' ? `此结果生成后，${(freshness.changed_inputs ?? []).map((l) => l.name ?? l.label ?? l.entity_id).join(' / ') || '输入'} 已改变：需要刷新` : freshness.status === 'upstream_stale' ? '上游需要刷新' : freshness.status === 'unavailable' ? '引用不可用' : '无法确认'}</div>
          )}
          {payload.program && <ProgramView wishId={wishId} canEdit={canEdit && payload.executor === 'xllm'} />}
        </div>
      )}
    </div>
  )
}
