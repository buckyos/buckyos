/* The wish flow as the user sees it (phase two §7.2, §8.3): prompt and executor, inputs with their
 * state, the analysis (context prompt, re-analysis needed?), explicit 分析 / 执行 / 应用 / 取消, the
 * candidate with its warnings and pre-check, the manual-modification choices, the last run's
 * freshness and the results that were not generated this time. Shown by the data-source detail and
 * by the wish Block after activation. */

import { useCallback, useState, useSyncExternalStore } from 'react'
import { describeError, type ReadOk } from '../../api/session'
import type { Json, WishPayloadRead } from '../../api/types'
import { EDIT_STATE_LABEL } from '../../state/edits'
import { useEdit, useEntity, useFreshness, useLoad, useOutlineVersion, useStore, useVersion, useWorkspaceUi } from '../../state/hooks'
import { FreshnessBadge } from '../sources/FreshnessBadge'
import type { Candidate, ManualChoice } from './WishService'

export function WishPanel({ wishId, readOnly, compact }: { wishId: string; readOnly: boolean; compact?: boolean }) {
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
  const [message, setMessage] = useState<{ kind: 'info' | 'error'; text: string } | null>(null)
  const [choices, setChoices] = useState<Record<string, ManualChoice>>({})
  const [manual, setManual] = useState<{ name: string; entityId: string }[]>([])
  const editEntry = useEdit(`wish:${wishId}:apply`)
  const candidate = store.wish.candidate(wishId)
  const busy = store.wish.busy.get(wishId)
  // a new candidate resets the manual-modification answers (derived during render)
  const [runSeen, setRunSeen] = useState<string | undefined>(candidate?.runId)
  if (runSeen !== candidate?.runId) { setRunSeen(candidate?.runId); setChoices({}); setManual([]) }
  if (read.error && !read.data) return <div className="aiws-error" role="alert">无法读取许愿格：{read.error}</div>
  if (!read.data || !entity) return <div className="aiws-muted">载入许愿格…</div>
  const payload = read.data.content.payload
  const keyRevs = read.data.content.key_revs
  const canEdit = !readOnly && entity.capabilities.includes('update')
  const needsAnalysis = !payload.analysis || payload.analysis.prompt !== payload.prompt
  const inputs = payload.inputs ?? []
  const act = async (work: () => Promise<void>) => {
    setMessage(null)
    try { await work() } catch (error) { setMessage({ kind: 'error', text: describeError(error) }) }
  }
  const savePrompt = () => act(async () => {
    if (prompt === null || prompt === payload.prompt) { setPrompt(null); return }
    const outcome = await store.submit({ editId: `wish:${wishId}:prompt`, label: '修改提示词', mine: prompt, hasMine: true, operations: [{ op: 'entity.set_keys', entity_id: wishId, keys: [{ key: 'prompt', value: prompt, expect: { rev: keyRevs.prompt ?? 0 } }] }] })
    if (outcome.status === 'accepted' || outcome.status === 'saved_locally') setPrompt(null)
  })
  const setMode = (mode: 'overwrite' | 'new') => act(async () => {
    await store.submit({ editId: `wish:${wishId}:mode`, label: `输出方式 → ${mode === 'overwrite' ? '覆盖' : '新建'}`, operations: [{ op: 'entity.set_keys', entity_id: wishId, keys: [{ key: 'output_mode', value: mode, expect: { rev: keyRevs.output_mode ?? 0 } }] }] })
  })
  const removeInput = (index: number) => act(async () => {
    const next = inputs.filter((_, i) => i !== index)
    await store.submit({ editId: `wish:${wishId}:inputs`, label: '移除输入', operations: [{ op: 'entity.set_keys', entity_id: wishId, keys: [{ key: 'inputs', value: next as unknown as Json, expect: { rev: keyRevs.inputs ?? 0 } }] }] })
  })
  const addInput = (entityId: string) => act(async () => {
    if (!entityId || inputs.some((input) => input.entity_id === entityId)) return
    const target = store.outline.get(entityId)
    const next = [...inputs, { entity_id: entityId, label: target?.title ?? target?.name ?? entityId, version: { mode: 'follow' as const } }]
    await store.submit({ editId: `wish:${wishId}:inputs`, label: '添加输入', operations: [{ op: 'entity.set_keys', entity_id: wishId, keys: [{ key: 'inputs', value: next as unknown as Json, expect: { rev: keyRevs.inputs ?? 0 } }] }] })
  })
  const toggleFixed = (index: number) => act(async () => {
    const next = inputs.map((input, i) => (i === index ? { ...input, version: { mode: input.version?.mode === 'fixed' ? 'follow' as const : 'fixed' as const } } : input))
    await store.submit({ editId: `wish:${wishId}:inputs`, label: '输入版本策略', operations: [{ op: 'entity.set_keys', entity_id: wishId, keys: [{ key: 'inputs', value: next as unknown as Json, expect: { rev: keyRevs.inputs ?? 0 } }] }] })
  })
  const analyze = () => act(() => store.wish.analyze(wishId))
  const execute = () => act(async () => { await store.wish.execute(wishId) })
  const apply = (extra: Record<string, ManualChoice> = {}) => act(async () => {
    if (!candidate) return
    const merged = { ...choices, ...extra }
    const { outcome, plan } = await store.wish.apply(candidate, merged)
    if (!outcome) { setManual(plan.manual); setChoices(merged); return }
    setManual([])
    if (outcome.status === 'accepted') setMessage({ kind: 'info', text: `已应用：新建 ${plan.created.length} 项，更新 ${plan.updated.length} 项${plan.skipped.length ? `，保留 ${plan.skipped.length} 项人工修改` : ''}${plan.missing.length ? `；上次存在、本次未生成 ${plan.missing.length} 项` : ''}。可整体撤销。` })
    else if (outcome.status === 'saved_locally') setMessage({ kind: 'info', text: '已保存到本设备，等待发送到后台。' })
    else if (outcome.status === 'conflict') setMessage({ kind: 'error', text: `应用冲突：执行期间输入或许愿格已变化（${outcome.code}）。旧结果保持不变；请重新执行。` })
    else setMessage({ kind: 'error', text: `应用未被接受（${'code' in outcome ? outcome.code : outcome.status}）：旧结果保持不变。${outcome.status === 'rejected' ? outcome.errors?.[0]?.detail ?? '' : ''}` })
  })
  const candidates = store.outline.all().filter((e) => !e.deleted && e.entity_id !== wishId && e.type_id !== 'buckyos.cell' && e.type_id !== 'buckyos.wish' && e.type_id !== 'buckyos.block-def' && (e.type_id !== 'buckyos.container' || e.kind === 'folder') && store.outline.ancestors(e.entity_id).includes('data') && e.entity_id !== 'canvas-content')
  const lastRun = payload.last_run
  return (
    <div className={`aiws-wish${compact ? ' is-compact' : ''}`} data-testid={`aiws-wish-${wishId}`} data-busy={busy ?? ''} data-needs-analysis={needsAnalysis ? 'true' : 'false'}>
      <div className="aiws-wish-head">
        <span className="aiws-chip aiws-chip-derived" data-testid="aiws-wish-executor">{payload.executor === 'mock' ? '模拟执行器' : payload.executor}</span>
        <FreshnessBadge entityId={wishId} />
        {busy && <span className="aiws-muted" data-testid="aiws-wish-busy">{busy === 'analyzing' ? '分析中…' : busy === 'executing' ? '执行中…' : '应用中…'}</span>}
      </div>
      <label className="aiws-wish-prompt">
        <span className="aiws-muted">原始提示词</span>
        <textarea data-testid="aiws-wish-prompt" rows={compact ? 2 : 3} value={prompt ?? payload.prompt} disabled={!canEdit} onChange={(event) => setPrompt(event.target.value)} onBlur={() => { void savePrompt() }} />
      </label>
      <div className="aiws-wish-section">
        <div className="aiws-muted">输入引用 {needsAnalysis && <span className="aiws-chip aiws-chip-warn" data-testid="aiws-wish-needs-analysis">需要重新分析</span>}</div>
        {inputs.length === 0 && <div className="aiws-muted">尚无输入：点“分析”由执行器从提示词和可见数据中选取，或手动添加。</div>}
        <ul className="aiws-wish-inputs">
          {inputs.map((input, index) => {
            const target = store.outline.get(input.entity_id)
            const problem = freshness?.input_problems?.find((p) => p.entity_id === input.entity_id)
            return (
              <li key={`${input.entity_id}:${index}`} data-testid="aiws-wish-input" data-problem={problem?.reason ?? ''}>
                <button type="button" className="aiws-link" onClick={() => ui.openEntity(input.entity_id)}>{input.label ?? target?.title ?? target?.name ?? input.entity_id}</button>
                <span className="aiws-muted"> {target?.type_id.replace('buckyos.', '') ?? '?'}</span>
                <button type="button" className={`aiws-chip${input.version?.mode === 'fixed' ? ' aiws-chip-warn' : ''}`} title="跟随当前数据 / 固定版本" disabled={!canEdit} onClick={() => { void toggleFixed(index) }}>{input.version?.mode === 'fixed' ? '固定版本' : '跟随当前'}</button>
                {problem && <span className="aiws-error" data-testid="aiws-wish-input-problem">{problem.reason === 'missing' ? '不存在' : '不可读'}</span>}
                {canEdit && <button type="button" className="aiws-icon" aria-label="移除输入" onClick={() => { void removeInput(index) }}>×</button>}
              </li>
            )
          })}
        </ul>
        {canEdit && (
          <select aria-label="添加输入" value="" data-testid="aiws-wish-add-input" onChange={(event) => { void addInput(event.target.value) }}>
            <option value="">添加输入…</option>
            {candidates.map((e) => <option key={e.entity_id} value={e.entity_id}>{e.title ?? e.name ?? e.entity_id}（{e.type_id.replace('buckyos.', '')}）</option>)}
          </select>
        )}
      </div>
      {payload.analysis && (
        <div className="aiws-wish-section" data-testid="aiws-wish-analysis">
          <div className="aiws-muted">context 提示词{payload.analysis.at ? ` · ${new Date(payload.analysis.at).toLocaleString()}` : ''}</div>
          <div className="aiws-wish-context">{payload.analysis.context_prompt}</div>
          {(payload.analysis.warnings ?? []).length > 0 && <div className="aiws-warning">{payload.analysis.warnings?.join('；')}</div>}
        </div>
      )}
      <div className="aiws-inline-form">
        <label>输出方式
          <select data-testid="aiws-wish-mode" value={payload.output_mode ?? 'overwrite'} disabled={!canEdit} onChange={(event) => { void setMode(event.target.value === 'new' ? 'new' : 'overwrite') }}>
            <option value="overwrite">覆盖到固定名称（经版本历史回滚）</option>
            <option value="new">每次新建一组</option>
          </select>
        </label>
        <button type="button" data-testid="aiws-wish-analyze" disabled={!canEdit || Boolean(busy)} onClick={() => { void analyze() }}>分析</button>
        <button type="button" data-testid="aiws-wish-execute" disabled={!canEdit || Boolean(busy) || needsAnalysis || inputs.length === 0} title={needsAnalysis ? '先分析' : inputs.length === 0 ? '没有输入' : '按 context 提示词和读集合执行，生成候选'} onClick={() => { void execute() }}>执行</button>
      </div>
      {message && <div className={message.kind === 'error' ? 'aiws-error' : 'aiws-warning'} role={message.kind === 'error' ? 'alert' : 'status'} data-testid="aiws-wish-message">{message.text}</div>}
      {editEntry && editEntry.state !== 'committed' && <div className={`aiws-state aiws-state-${editEntry.state}`} data-testid="aiws-wish-apply-state">{EDIT_STATE_LABEL[editEntry.state]}{editEntry.detail ? `：${editEntry.detail}` : ''}</div>}
      {candidate && <CandidateView candidate={candidate} manual={manual} choices={choices} onChoice={(name, choice) => setChoices((prev) => ({ ...prev, [name]: choice }))} onApply={() => { void apply() }} onDiscard={() => store.wish.discard(wishId)} busy={Boolean(busy)} canApply={canEdit} />}
      {lastRun && (
        <div className="aiws-wish-section" data-testid="aiws-wish-last-run">
          <div className="aiws-muted">上次运行 {lastRun.run_id}{lastRun.at ? ` · ${new Date(lastRun.at).toLocaleString()}` : ''}{lastRun.simulated ? ' · 模拟' : ''}</div>
          <div>结果：{(lastRun.produced ?? []).map((id) => {
            const e = store.outline.get(id)
            return <button key={id} type="button" className="aiws-link" data-testid="aiws-wish-produced" onClick={() => ui.openEntity(id)}>{e?.title ?? e?.name ?? id}{e?.deleted || !e ? '（已删除）' : ''}</button>
          })}</div>
          {(lastRun.missing ?? []).length > 0 && (
            <div className="aiws-warning" data-testid="aiws-wish-missing">上次存在、本次未生成：{lastRun.missing?.map((id) => store.outline.get(id)?.title ?? store.outline.get(id)?.name ?? id).join('、')}（未自动删除，请自行处理）</div>
          )}
          {freshness && freshness.status !== 'none' && freshness.status !== 'current' && (
            <div className="aiws-warning" data-testid="aiws-wish-stale">{freshness.status === 'stale' ? `此结果生成后，${(freshness.changed_inputs ?? []).map((l) => l.name ?? l.label ?? l.entity_id).join(' / ') || '输入'} 已改变：需要刷新` : freshness.status === 'upstream_stale' ? '上游需要刷新' : freshness.status === 'unavailable' ? '引用不可用' : '无法确认'}</div>
          )}
        </div>
      )}
    </div>
  )
}

function CandidateView({ candidate, manual, choices, onChoice, onApply, onDiscard, busy, canApply }: { candidate: Candidate; manual: { name: string; entityId: string }[]; choices: Record<string, ManualChoice>; onChoice: (name: string, choice: ManualChoice) => void; onApply: () => void; onDiscard: () => void; busy: boolean; canApply: boolean }) {
  const store = useStore()
  const [check, setCheck] = useState<{ ok: boolean; text: string } | null>(null)
  const precheck = async () => {
    try {
      const plan = await store.wish.plan(candidate, choices)
      const result = await store.session.prepare(plan.operations)
      setCheck(result.status === 'ok' ? { ok: true, text: `预检通过：将影响 ${result.touched.length} 处（新建 ${plan.created.length}，更新 ${plan.updated.length}）` } : { ok: false, text: `预检未通过（${result.code}）：${result.status === 'conflict' ? `${result.conflicts.length} 处冲突` : result.errors?.[0]?.detail ?? ''}` })
    } catch (error) { setCheck({ ok: false, text: describeError(error) }) }
  }
  return (
    <div className="aiws-wish-candidate" data-testid="aiws-wish-candidate" data-run-id={candidate.runId}>
      <div><b>候选结果</b> <span className="aiws-chip aiws-chip-derived">{candidate.simulated ? '模拟' : '生成'}</span> <span className="aiws-muted">{candidate.result.summary}</span></div>
      <ul>
        {candidate.result.results.map((r) => <li key={r.name} data-testid="aiws-wish-result">{r.title ?? r.name} <span className="aiws-muted">{r.type}{r.renderer ? ` · ${r.renderer}` : ''}</span></li>)}
      </ul>
      {candidate.result.warnings.length > 0 && <div className="aiws-warning" data-testid="aiws-wish-warnings">{candidate.result.warnings.join('；')}</div>}
      {candidate.result.assumptions.length > 0 && <div className="aiws-muted">假设：{candidate.result.assumptions.join('；')}</div>}
      <div className="aiws-muted">读集合：{candidate.readSet.length} 个版本格（执行开始时固定）</div>
      {manual.length > 0 && (
        <div className="aiws-warning" data-testid="aiws-wish-manual">
          以下结果在生成后被人工修改过，请逐项选择：
          {manual.map((m) => (
            <div key={m.name} className="aiws-inline-form" data-testid={`aiws-wish-manual-${m.name}`}>
              <span>{m.name}</span>
              {(['keep', 'replace', 'new'] as ManualChoice[]).map((choice) => (
                <label key={choice}><input type="radio" name={`manual-${m.name}`} checked={choices[m.name] === choice} onChange={() => onChoice(m.name, choice)} /> {choice === 'keep' ? '保留人工修改' : choice === 'replace' ? '显式替换' : '新建一份'}</label>
              ))}
            </div>
          ))}
        </div>
      )}
      {check && <div className={check.ok ? 'aiws-muted' : 'aiws-error'} data-testid="aiws-wish-precheck">{check.text}</div>}
      <div className="aiws-inline-form">
        <button type="button" data-testid="aiws-wish-precheck-run" disabled={busy} onClick={() => { void precheck() }}>预检</button>
        <button type="button" data-testid="aiws-wish-apply" disabled={busy || !canApply || manual.some((m) => !choices[m.name])} onClick={onApply}>应用</button>
        <button type="button" data-testid="aiws-wish-discard" disabled={busy} onClick={onDiscard}>取消</button>
      </div>
    </div>
  )
}
