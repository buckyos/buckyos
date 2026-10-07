/* The candidate of a run (许愿格 §8.7, §8.8, §11): results as they will look, the checks and what
 * needs the user's confirmation, structure changes, manual-edit choices, results that will be
 * missing, inputs appended during execution — and feedback for another round. Nothing is written
 * until 应用; the plan applied is exactly the one previewed. */

import { useState } from 'react'
import { describeError } from '../../api/session'
import type { WishChoices, WishRunView } from '../../api/types'
import { useStore } from '../../state/hooks'
import { ResultPreview } from './WishPreviews'
import { applyMessage } from './WishService'

const ACTION_LABEL: Record<string, string> = { create: '新建', update: '更新', unchanged: '未变化', keep_manual: '保留人工修改', new_copy: '另存一份' }
const CHECK_LABEL: Record<string, string> = { passed: '通过', failed: '未通过', not_run: '未执行', review: '请确认' }
const TYPE_LABEL: Record<string, string> = { table: '表格', table_columns: '派生列', record: '记录', richtext: '文本', image: '图片', asset: '文件', html: 'HTML Block', video: '逐帧预览' }

export function WishCandidate({ run, canApply, onMessage }: { run: WishRunView; canApply: boolean; onMessage: (kind: 'info' | 'error', text: string) => void }) {
  const store = useStore()
  const [choices, setChoices] = useState<WishChoices>({})
  const [feedback, setFeedback] = useState('')
  const [open, setOpen] = useState<Record<string, boolean>>({})
  const [acceptFailed, setAcceptFailed] = useState(false)
  const [runSeen, setRunSeen] = useState(run.run_id)
  if (runSeen !== run.run_id) { setRunSeen(run.run_id); setChoices({}); setFeedback(''); setAcceptFailed(false) }
  const busy = store.wish.busy.get(run.wish_id)
  const cand = run.candidate ?? {}
  const preview = run.preview
  const summary = preview?.summary
  const choose = (next: WishChoices) => {
    setChoices(next)
    void store.wish.preview(run, next).catch((error: unknown) => onMessage('error', describeError(error)))
  }
  const apply = async () => {
    if (!preview) return
    try {
      const next = await store.wish.apply(run, preview.plan_digest)
      if (next.applied?.status === 'accepted') onMessage('info', run.stage === 'analyze' ? '分析已写回。' : applyMessage(next))
      else if (next.applied?.status === 'conflict') onMessage('error', `应用冲突：运行之后输入、许愿格或结果已变化（${next.applied.commit.status === 'conflict' ? next.applied.commit.code : ''}）。旧结果保持不变；请重新预览或重新执行。`)
      else onMessage('error', `应用未被接受（${next.applied?.commit && 'code' in next.applied.commit ? next.applied.commit.code : next.state}）：旧结果保持不变。`)
    } catch (error) { onMessage('error', `应用未被接受：${describeError(error)}`) }
  }
  const sendFeedback = async () => {
    if (!feedback.trim()) return
    try {
      await store.wish.execute(run.wish_id, { feedback: feedback.trim(), parentRunId: run.run_id, cellId: (run.params.location as { cell_id?: string } | undefined)?.cell_id })
      setFeedback('')
    } catch (error) { onMessage('error', describeError(error)) }
  }
  const results = cand.results ?? []
  const planned = new Map((summary?.results ?? []).map((r) => [r.name, r]))
  const checks = cand.checks ?? []
  const failed = checks.filter((c) => c.status === 'failed')
  const manual = summary?.manual ?? []
  const needsChoice = manual.some((m) => !choices.results?.[m.name])
  const destructive = Boolean(summary?.destructive)
  const blocking = (summary?.problems ?? []).filter((p) => !(destructive && p.includes('结构变化')))
  const title = run.stage === 'analyze' ? '分析结果（待写回）' : run.stage === 'rerun_program' ? '只重跑程序的候选' : run.stage === 'repair_program' ? '修复后的候选' : '候选结果'
  return (
    <div className="aiws-wish-candidate" data-testid="aiws-wish-candidate" data-run-id={run.run_id} data-state={run.state} data-ready={preview?.ready ? 'true' : 'false'}>
      <div className="aiws-wish-head">
        <b>{title}</b>
        <span className="aiws-chip aiws-chip-derived">{run.simulated ? '模拟' : cand.mode === 'program' ? '程序（未调用模型）' : '生成'}</span>
        {run.feedback && <span className="aiws-chip" title={run.feedback}>按反馈修改</span>}
        {cand.summary && <span className="aiws-muted">{cand.summary}</span>}
      </div>
      {run.stage === 'analyze' && cand.analysis && (
        <div className="aiws-wish-context">{cand.analysis.context_prompt}</div>
      )}
      {results.length > 0 && (
        <ul className="aiws-wish-results">
          {results.map((r) => {
            const p = planned.get(r.name)
            const isOpen = open[r.name] ?? results.length <= 3
            return (
              <li key={r.name} data-testid="aiws-wish-result" data-name={r.name} data-action={p?.action ?? ''}>
                <div className="aiws-wish-result-head">
                  <button type="button" className="aiws-link" onClick={() => setOpen((o) => ({ ...o, [r.name]: !isOpen }))}>{isOpen ? '▾' : '▸'} {r.title ?? r.name}</button>
                  <span className="aiws-muted">{TYPE_LABEL[r.type] ?? r.type}</span>
                  <span className="aiws-chip" title={r.approach === 'program' ? '由程序产生，数字可复算' : '由模型直接书写'}>{r.approach === 'program' ? '程序' : '书写'}</span>
                  {p && <span className="aiws-chip aiws-chip-derived">{ACTION_LABEL[p.action] ?? p.action}{p.reordered ? ' · 行序更新' : ''}</span>}
                  {cand.model_judgment?.includes(r.name) && <span className="aiws-chip aiws-chip-warn" title="使用了逐项模型判断（llm.map），请抽查">含模型判断</span>}
                  {r.approach === 'program' && (cand.external_data?.length ?? 0) > 0 && <span className="aiws-chip aiws-chip-warn" title={cand.external_data?.join('\n')}>含外部数据</span>}
                </div>
                {isOpen && <ResultPreview result={r} />}
              </li>
            )
          })}
        </ul>
      )}
      {checks.length > 0 && (
        <div className="aiws-wish-checks" data-testid="aiws-wish-checks">
          <div className="aiws-muted">验收检查</div>
          {checks.map((c) => (
            <div key={c.id} className={`aiws-wish-check is-${c.status}`} data-testid="aiws-wish-check" data-check={c.id} data-status={c.status}>
              <span className="aiws-chip">{CHECK_LABEL[c.status] ?? c.status}</span> {c.text}
              {c.detail !== undefined && c.detail !== null && <span className="aiws-muted"> · {typeof c.detail === 'string' ? c.detail : JSON.stringify(c.detail)}</span>}
              {c.self_assessment !== undefined && <span className="aiws-muted"> · 自评：{typeof c.self_assessment === 'string' ? c.self_assessment : JSON.stringify(c.self_assessment)}</span>}
            </div>
          ))}
        </div>
      )}
      {failed.length > 0 && (
        <label className="aiws-error" role="alert" data-testid="aiws-wish-failed-checks">
          <input type="checkbox" data-testid="aiws-wish-accept-failed" checked={acceptFailed} onChange={(e) => setAcceptFailed(e.target.checked)} /> 有 {failed.length} 项检查未通过：我已确认结果仍然可用
        </label>
      )}
      {(cand.uncited_numbers?.length ?? 0) > 0 && (
        <div className="aiws-warning" data-testid="aiws-wish-uncited">文字中有数字无法在程序输出中找到，请核对：{cand.uncited_numbers?.map((u) => `${u.result}：${u.numbers.join('、')}`).join('；')}</div>
      )}
      {(cand.assumptions?.length ?? 0) > 0 && <div className="aiws-muted">假设：{cand.assumptions?.join('；')}</div>}
      {(cand.warnings?.length ?? 0) > 0 && <div className="aiws-warning" data-testid="aiws-wish-warnings">{cand.warnings?.join('；')}</div>}
      {(summary?.appended_inputs?.length ?? 0) > 0 && (
        <div className="aiws-muted" data-testid="aiws-wish-appended">执行时追加的输入（应用后加入许愿格）：{summary?.appended_inputs?.map((i) => i.label ?? store.outline.get(i.entity_id)?.title ?? i.entity_id).join('、')}</div>
      )}
      {(summary?.structure?.length ?? 0) > 0 && (
        <div className="aiws-wish-section" data-testid="aiws-wish-structure">
          <div className="aiws-muted">结构变化</div>
          {summary?.structure?.map((s) => (
            <div key={s.name}>{s.name}：{s.changes.map((c) => `${{ add_field: '新增字段', delete_field: '删除字段', change_type: '改类型', delete_rows: '删除行', insert_rows: '新增行', update_values: '更新值', clear_values: '清空值', key_changed: '键改变', recreate_field: '重建字段', widen_scale: '增加小数位' }[c.kind] ?? c.kind}${c.field ? ` ${c.field}` : ''}${c.count ? ` ${c.count}` : ''}${c.from ? `（${c.from} → ${c.to}）` : c.kind === 'widen_scale' ? `（${c.to} 位）` : ''}`).join('，')}{s.rows_before !== undefined ? `（${s.rows_before} → ${s.rows_after} 行）` : ''}</div>
          ))}
          {destructive && (
            <label className="aiws-warning"><input type="checkbox" data-testid="aiws-wish-confirm-structure" checked={Boolean(choices.confirm_structure)} onChange={(e) => choose({ ...choices, confirm_structure: e.target.checked })} /> 我已看过：会删除行或字段</label>
          )}
        </div>
      )}
      {manual.length > 0 && (
        <div className="aiws-warning" data-testid="aiws-wish-manual">
          以下结果在生成后被人工修改过，请逐项选择：
          {manual.map((m) => (
            <div key={m.name} className="aiws-inline-form" data-testid={`aiws-wish-manual-${m.name}`}>
              <span>{m.name}{m.cells ? `（${m.cells} 个单元格）` : ''}{m.field_deleted ? `（列「${m.field_deleted}」已被删除）` : ''}</span>
              {(['keep', 'replace', 'new'] as const).map((choice) => (
                <label key={choice}><input type="radio" name={`manual-${run.run_id}-${m.name}`} checked={choices.results?.[m.name] === choice} onChange={() => choose({ ...choices, results: { ...(choices.results ?? {}), [m.name]: choice } })} /> {choice === 'keep' ? '保留人工修改' : choice === 'replace' ? '显式替换' : '新建一份'}</label>
              ))}
            </div>
          ))}
        </div>
      )}
      {(summary?.missing?.length ?? 0) > 0 && <div className="aiws-warning">上次存在、本次未生成：{summary?.missing?.map((m) => m.name).join('、')}（不会被删除）</div>}
      {summary?.program && <div className="aiws-muted" data-testid="aiws-wish-program-change">应用后保存新的程序（{summary.program.from ? '修改' : '首次'}）</div>}
      {(summary?.refinements?.length ?? 0) > 0 && <div className="aiws-muted">应用后记入修改意见：{summary?.refinements?.map((r) => r.text).join('；')}</div>}
      {blocking.length > 0 && !needsChoice && <div className="aiws-error" data-testid="aiws-wish-problems">{blocking.join('；')}</div>}
      {run.state === 'conflict' && <div className="aiws-error">上次应用冲突：可以重新预览后再应用，或重新执行。</div>}
      {run.stage !== 'analyze' && !run.simulated && canApply && (
        <div className="aiws-wish-feedback">
          <textarea data-testid="aiws-wish-feedback" rows={2} placeholder="对这组结果提修改意见（如“把华东拆成上海和其他”“图换成折线图”）：在已有程序上修改，应用时记入许愿格" value={feedback} onChange={(e) => setFeedback(e.target.value)} />
          <button type="button" data-testid="aiws-wish-feedback-send" disabled={Boolean(busy) || !feedback.trim()} onClick={() => { void sendFeedback() }}>按反馈修改</button>
        </div>
      )}
      <div className="aiws-inline-form">
        <button type="button" data-testid="aiws-wish-apply" disabled={Boolean(busy) || !canApply || !preview?.ready || needsChoice || (failed.length > 0 && !acceptFailed)} title={needsChoice ? '先对人工修改过的结果作出选择' : !preview?.ready ? '预览有未解决的问题' : failed.length > 0 && !acceptFailed ? '先确认未通过的检查' : '一次提交，可整体撤销'} onClick={() => { void apply() }}>{run.stage === 'analyze' ? '写回分析' : '应用'}</button>
        <button type="button" data-testid="aiws-wish-discard" disabled={Boolean(busy)} onClick={() => store.wish.discard(run.wish_id)}>放弃</button>
      </div>
    </div>
  )
}
