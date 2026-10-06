/* Shell panels shared by both top-level modes: notices (with actions), the save-state list and the
 * phase-one controlled-processing panel kept as a kernel regression tool. */

import { useState, useSyncExternalStore } from 'react'
import { describeError } from '../../api/session'
import type { CommitResult, RunView, ServiceError, Touched } from '../../api/types'
import { EDIT_STATE_LABEL } from '../../state/edits'
import { useEdits, useStore } from '../../state/hooks'

export function Notices() {
  const store = useStore()
  const notices = useSyncExternalStore(store.subscribeNotices, store.noticeSnapshot)
  if (notices.length === 0) return null
  return (
    <div className="aiws-notices">
      {notices.map((notice) => (
        <div key={notice.id} className={notice.kind === 'error' ? 'aiws-error' : 'aiws-warning'} role={notice.kind === 'error' ? 'alert' : 'status'} data-testid="aiws-notice">
          {notice.text}
          {notice.action && <button type="button" className="aiws-link" data-testid="aiws-notice-action" onClick={() => { notice.action?.run(); store.dismissNotice(notice.id) }}>{notice.action.label}</button>}
          <button type="button" className="aiws-link" onClick={() => store.dismissNotice(notice.id)}>关闭</button>
        </div>
      ))}
    </div>
  )
}

/** Save states (design §6.5): entries that need attention, unsaved inputs, local-only saves. */
export function EditsPanel() {
  const store = useStore()
  const edits = useEdits()
  const list = [...edits.values()].sort((a, b) => b.at - a.at)
  const attention = list.filter((entry) => entry.state === 'needs_attention')
  const unsaved = list.filter((entry) => entry.state === 'unsaved')
  const local = list.filter((entry) => entry.state === 'saved_locally')
  if (attention.length === 0 && unsaved.length === 0 && local.length === 0) return null
  const replica = store.session.offline
  return (
    <div className="aiws-edits" data-testid="aiws-edits">
      <div className="aiws-panel-title">修改状态</div>
      {[...attention, ...unsaved, ...local].map((entry) => (
        <div key={entry.id} className={`aiws-edit aiws-edit-${entry.state}`} data-testid="aiws-edit-entry" data-state={entry.state} data-code={entry.code} data-edit-id={entry.id}>
          <span className={`aiws-state aiws-state-${entry.state}`}>{EDIT_STATE_LABEL[entry.state]}</span>
          <b>{entry.label}</b>
          {entry.detail && <div data-testid="aiws-edit-detail">{entry.detail}</div>}
          {entry.hasMine && <div>我的输入：<code data-testid="aiws-edit-mine">{typeof entry.mine === 'string' ? entry.mine : JSON.stringify(entry.mine)}</code></div>}
          {entry.state === 'needs_attention' && (
            <>
              {replica && entry.pendingKey && entry.code !== 'REVISION_CONFLICT' && entry.code !== 'TARGET_DELETED' && store.session.status().kind !== 'stopped' && (
                <button type="button" className="aiws-link" data-testid="aiws-edit-requeue" title="原样重新排队发送（同一个幂等键）"
                  onClick={() => { void replica.requeuePending(entry.pendingKey ?? '').catch((error: unknown) => store.notify('error', describeError(error))) }}>重新发送</button>
              )}
              <button type="button" className="aiws-link" data-testid="aiws-edit-dismiss" onClick={() => { void store.dismissEdit(entry.id) }}>{entry.pendingKey ? '放弃这条修改' : '知道了，移除此项'}</button>
            </>
          )}
          {entry.state === 'unsaved' && entry.storageFailed && (
            <button type="button" className="aiws-link" data-testid="aiws-edit-export" onClick={() => { void store.exportLocal() }}>导出未保存的输入</button>
          )}
        </div>
      ))}
    </div>
  )
}

// ---- controlled processing (design §7): the phase-one Mock kept as a kernel regression tool

const MOCK_PROGRAM = 'mock.task-summary@1'

function touchedText(item: Touched): string {
  const selector = item.selector
  const where = !selector ? '' : selector.kind === 'table_cell' ? ` / ${selector.record_id} / ${selector.field_id}` : selector.kind === 'richtext_block' ? ` / 块 ${selector.block_id}` : ` / ${selector.kind}`
  return `${item.entity_id}${where}（${item.change}）`
}

export function MockRunPanel() {
  const store = useStore()
  const [today, setToday] = useState(() => new Date().toISOString().slice(0, 10))
  const [run, setRun] = useState<RunView | null>(null)
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState<string | null>(null)
  const surfaces = store.outline.childrenOf('surfaces')
  const act = async (work: () => Promise<void>) => {
    setBusy(true)
    setMessage(null)
    try { await work() } catch (error) { setMessage(describeError(error)) } finally { setBusy(false) }
  }
  const start = () => act(async () => { setRun(await store.session.procStart(MOCK_PROGRAM, { today, surface_id: surfaces[0]?.entity_id ?? 'surface-main' })) })
  const apply = () => act(async () => {
    if (!run) return
    const next = await store.session.procApply(run.run_id)
    setRun(next)
    const result = next.result
    if (result && result.status === 'accepted') {
      store.undo.pushCommit(result.commit_id, '应用模拟加工结果')
      await store.session.whenApplied(result.seq)
    }
  })
  const cancel = () => act(async () => {
    if (!run) return
    const result = await store.session.procCancel(run.run_id)
    setMessage(result.already_applied ? `运行已应用（提交 ${result.commit_id ?? ''}），无法取消。` : '运行已取消，候选结果不会被应用。')
    setRun(await store.session.procGet(run.run_id))
  })
  const prepare = run?.prepare
  const waiting = run?.state === 'waiting_confirmation'
  const outcome = run?.result as { error?: ServiceError; status?: string } | null | undefined
  const runError = outcome?.error ?? null
  const applied = outcome?.status ? (outcome as CommitResult) : null
  return (
    <div className="aiws-mock" data-testid="aiws-mock">
      <div className="aiws-panel-title">受控加工（内核回归用例） <span className="aiws-chip aiws-chip-derived" data-testid="aiws-mock-simulated">模拟结果</span></div>
      <div className="aiws-muted">程序 {MOCK_PROGRAM} 是第一期的确定性模拟程序，保留为内核回归用例；许愿格请在画布或数据树中使用。</div>
      <div className="aiws-inline-form">
        <label>今天 <input type="date" aria-label="今天" data-testid="aiws-mock-today" value={today} onChange={(event) => setToday(event.target.value)} /></label>
        <button type="button" data-testid="aiws-mock-start" disabled={busy} onClick={() => { void start() }}>生成候选</button>
      </div>
      {message && <div className="aiws-warning" role="status" data-testid="aiws-mock-message">{message}</div>}
      {run && (
        <div className="aiws-mock-run" data-testid="aiws-mock-run" data-state={run.state}>
          <div>运行 {run.run_id} · 状态 <b>{run.state}</b></div>
          {prepare && prepare.status === 'ok' && <div data-testid="aiws-mock-prepare">候选预检通过，将影响 {prepare.touched.length} 处：<ul>{prepare.touched.slice(0, 20).map((item, index) => <li key={index}>{touchedText(item)}</li>)}</ul></div>}
          {prepare && prepare.status !== 'ok' && <div className="aiws-error" role="alert" data-testid="aiws-mock-prepare">候选预检未通过（{prepare.code}）：{prepare.status === 'conflict' ? `${prepare.conflicts.length} 处冲突` : prepare.errors?.[0]?.detail ?? ''}</div>}
          {runError && <div className="aiws-warning" role="status" data-testid="aiws-mock-failed">运行没有产生候选（{runError.code}）：{runError.detail ?? ''}。文档没有任何变化。</div>}
          {run.warnings.length > 0 && <div className="aiws-warning" data-testid="aiws-mock-warnings">警告（{run.warnings.length}）：<ul>{run.warnings.map((warning, index) => <li key={index}>{typeof warning === 'string' ? warning : JSON.stringify(warning)}</li>)}</ul></div>}
          {applied && applied.status !== 'accepted' && <div className="aiws-error" role="alert" data-testid="aiws-mock-result">应用未被接受：{applied.code}，文档没有任何变化。</div>}
          {applied?.status === 'accepted' && <div data-testid="aiws-mock-result">已应用为提交 {applied.commit_id}（模拟结果，可用撤销整体回退）。</div>}
          {waiting && (
            <div className="aiws-inline-form">
              <button type="button" data-testid="aiws-mock-apply" disabled={busy} onClick={() => { void apply() }}>应用模拟结果</button>
              <button type="button" data-testid="aiws-mock-cancel" disabled={busy} onClick={() => { void cancel() }}>取消运行</button>
            </div>
          )}
        </div>
      )}
    </div>
  )
}
