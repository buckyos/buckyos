/* Side panels of the workspace view: outline ("data" tree), annotations, Mock run, edits that need attention. */

import { useState } from 'react'
import { describeError } from '../api/session'
import type { CommitResult, EntityEnvelope, Operation, Reference, RunView, ServiceError, Touched } from '../api/types'
import { orderKeyBetween } from '../api/wasm'
import { EDIT_STATE_LABEL } from '../state/edits'
import { useEdits, useStore, useWorkspaceUi } from '../state/hooks'
import { annotationOp, describeTarget, descendants, entityLabel, sortedChildren } from './creators'

const TYPE_LABEL: Record<string, string> = {
  'buckyos.container': '容器', 'buckyos.record': '记录', 'buckyos.richtext': '富文本', 'buckyos.table-source': '表',
  'buckyos.cell': '单元', 'buckyos.asset-ref': '资产', 'buckyos.annotation': '批注',
}

// ---- outline

export function OutlinePanel({ selected, onSelect }: { selected: string | null; onSelect: (entityId: string | null) => void }) {
  const { entities, byId } = useWorkspaceUi()
  const root = entities.find((entity) => entity.kind === 'root')
  const current = selected ? byId.get(selected) : undefined
  return (
    <div className="aiws-outline" data-testid="aiws-outline">
      <div className="aiws-panel-title">数据大纲 <span className="aiws-muted">{entities.filter((entity) => !entity.deleted).length} 个对象</span></div>
      <ul role="tree">{root && <OutlineNode entity={root} selected={selected} onSelect={onSelect} />}</ul>
      {current && <EntityActions key={current.entity_id} entity={current} />}
    </div>
  )
}

function OutlineNode({ entity, selected, onSelect }: { entity: EntityEnvelope; selected: string | null; onSelect: (entityId: string | null) => void }) {
  const { entities } = useWorkspaceUi()
  const children = sortedChildren(entities, entity.entity_id)
  return (
    <li role="treeitem" aria-selected={selected === entity.entity_id} data-testid="aiws-outline-item" data-entity-id={entity.entity_id} data-type={entity.type_id}>
      <button type="button" className={`aiws-outline-row${selected === entity.entity_id ? ' is-selected' : ''}`} onClick={() => onSelect(selected === entity.entity_id ? null : entity.entity_id)}>
        <span className="aiws-outline-type">{entity.kind === 'root' ? '根' : entity.kind === 'page' ? '页' : entity.kind === 'group' ? '组' : TYPE_LABEL[entity.type_id] ?? entity.type_id}</span>
        <span className="aiws-outline-name">{entityLabel(entity)}</span>
        {entity.write_policy === 'lock_required' && <span title={entity.lock_holder ? `由 ${entity.lock_holder.principal} 持有写锁` : '启用了写锁'}>🔒</span>}
        {entity.degraded && <span className="aiws-chip aiws-chip-warn" title={entity.degraded}>降级</span>}
      </button>
      {children.length > 0 && <ul role="group">{children.map((child) => <OutlineNode key={child.entity_id} entity={child} selected={selected} onSelect={onSelect} />)}</ul>}
    </li>
  )
}

function EntityActions({ entity }: { entity: EntityEnvelope }) {
  const store = useStore()
  const { entities } = useWorkspaceUi()
  const [name, setName] = useState(entity.name ?? '')
  const isRoot = entity.kind === 'root'
  const can = (capability: string) => entity.capabilities.includes(capability as never)
  const siblings = entity.parent_id ? sortedChildren(entities, entity.parent_id) : []
  const index = siblings.findIndex((item) => item.entity_id === entity.entity_id)
  const containers = entities.filter((item) => !item.deleted && item.type_id === 'buckyos.container' && (item.kind === 'page' || item.kind === 'group')
    && item.entity_id !== entity.entity_id && !descendants(entities, entity.entity_id).includes(item.entity_id))
  const submit = (label: string, operations: Operation[]) => store.submit({ editId: `entity:${entity.entity_id}`, label, operations })

  const place = (direction: -1 | 1) => {
    // Reordering is an auto-merge operation: no `expect`, the later arrival wins (design §3.1).
    const before = direction === -1 ? siblings[index - 2]?.order_key : siblings[index + 1]?.order_key
    const after = direction === -1 ? siblings[index - 1]?.order_key : siblings[index + 2]?.order_key
    let key: string
    try { key = orderKeyBetween(store.core, before, after) } catch (error) { store.notify('error', `无法生成顺序键：${describeError(error)}`); return }
    void submit(`调整顺序 ${entityLabel(entity)}`, [{ op: 'tree.place', entity_id: entity.entity_id, order_key: key }])
  }

  return (
    <div className="aiws-entity-actions" data-testid="aiws-entity-actions">
      <div className="aiws-muted">{entity.entity_id} · {TYPE_LABEL[entity.type_id] ?? entity.type_id} · 内容版本 {entity.content_rev}</div>
      {!isRoot && can('structure') && (
        <>
          <form className="aiws-inline-form" onSubmit={(event) => {
            event.preventDefault()
            void submit(`改名 ${entity.entity_id}`, [{ op: 'entity.rename', entity_id: entity.entity_id, name: name.trim() === '' ? null : name.trim(), expect: { rev: entity.meta_rev } }])
          }}>
            <input aria-label="查找名" placeholder="查找名（可留空）" value={name} onChange={(event) => setName(event.target.value)} />
            <button type="submit" data-testid="aiws-rename">改名</button>
          </form>
          <div className="aiws-inline-form">
            <button type="button" data-testid="aiws-move-up" disabled={index <= 0} onClick={() => place(-1)}>上移</button>
            <button type="button" data-testid="aiws-move-down" disabled={index < 0 || index >= siblings.length - 1} onClick={() => place(1)}>下移</button>
            {entity.kind !== 'page' && (
              <select aria-label="移动到" value="" onChange={(event) => {
                const parent = event.target.value
                if (!parent) return
                const last = sortedChildren(entities, parent).at(-1)?.order_key
                let key: string
                try { key = orderKeyBetween(store.core, last, null) } catch (error) { store.notify('error', describeError(error)); return }
                void submit(`移动 ${entityLabel(entity)}`, [{ op: 'tree.move', entity_id: entity.entity_id, new_parent_id: parent, order_key: key }])
              }}>
                <option value="">移动到…</option>
                {containers.filter((item) => item.entity_id !== entity.parent_id).map((item) => <option key={item.entity_id} value={item.entity_id}>{entityLabel(item)}</option>)}
              </select>
            )}
          </div>
        </>
      )}
      {!isRoot && can('delete') && (
        <button type="button" data-testid="aiws-delete-entity" onClick={() => {
          // The whole subtree is listed explicitly: nobody deletes something they have not seen (design §3.1).
          const subtree = descendants(entities, entity.entity_id)
          void submit(`删除 ${entityLabel(entity)}`, [{ op: 'entity.delete', entity_id: entity.entity_id, ...(subtree.length > 0 ? { subtree: { delete: subtree } } : {}), expect: { rev: entity.life_rev } }])
        }}>删除{descendants(entities, entity.entity_id).length > 0 ? `（连同 ${descendants(entities, entity.entity_id).length} 个子对象）` : ''}</button>
      )}
      {!isRoot && can('manage') && (
        <div className="aiws-inline-form" data-testid="aiws-lock-admin">
          <span>写入策略：{entity.write_policy === 'lock_required' ? '需要写锁' : '开放'}</span>
          <button type="button" data-testid="aiws-toggle-policy" onClick={() => {
            const policy = entity.write_policy === 'lock_required' ? 'open' : 'lock_required'
            void submit(`写入策略 → ${policy}`, [{ op: 'entity.set_write_policy', entity_id: entity.entity_id, policy, expect: { rev: entity.meta_rev } }])
          }}>{entity.write_policy === 'lock_required' ? '改为开放' : '启用写锁'}</button>
          {entity.lock_holder && (
            <button type="button" data-testid="aiws-break-lock" onClick={() => {
              store.session.lockBreak(entity.entity_id).then(
                () => { store.notify('info', `已强制解除 ${entity.lock_holder?.principal ?? ''} 对 ${entityLabel(entity)} 的写锁。`); store.versions.bump(['outline']) },
                (error: unknown) => store.notify('error', `解除写锁失败：${describeError(error)}`),
              )
            }}>强制解除 {entity.lock_holder.principal} 的锁</button>
          )}
        </div>
      )}
    </div>
  )
}

// ---- annotations (design §3.7)

export function AnnotationsPanel({ pageId, draft, onDraftDone }: { pageId: string | null; draft: { target: Reference; label: string } | null; onDraftDone: () => void }) {
  const store = useStore()
  const { annotations, entities, byId } = useWorkspaceUi()
  const [body, setBody] = useState('')
  return (
    <div className="aiws-annotations" data-testid="aiws-annotations">
      <div className="aiws-panel-title">批注</div>
      {draft && (
        <form className="aiws-annotation-draft" onSubmit={(event) => {
          event.preventDefault()
          if (!pageId || body.trim() === '') return
          void store.submit({ editId: `annotation:new`, label: `批注 ${draft.label}`, mine: body, hasMine: true, operations: [annotationOp(store.core, entities, pageId, draft.target, body.trim())] })
            .then((outcome) => { if (outcome.status === 'accepted') { setBody(''); onDraftDone() } })
        }}>
          <div>批注对象：{describeTarget(draft.target)}</div>
          <textarea aria-label="批注内容" autoFocus rows={2} value={body} onChange={(event) => setBody(event.target.value)} maxLength={4000} />
          <button type="submit" data-testid="aiws-annotation-save">保存批注</button>
          <button type="button" onClick={() => { setBody(''); onDraftDone() }}>取消</button>
        </form>
      )}
      {annotations.length === 0 && !draft && <div className="aiws-muted">还没有批注。在表格单元格上点 ✎，或在富文本里点“批注当前块”。</div>}
      {annotations.map((mark) => {
        const envelope = byId.get(mark.entityId)
        return (
          <div key={mark.entityId} className="aiws-annotation" data-testid="aiws-annotation" data-anchor-state={mark.anchorState} style={{ background: typeof mark.payload.style?.color === 'string' ? mark.payload.style.color : undefined }}>
            <div className="aiws-annotation-body">{mark.payload.body}</div>
            <div className="aiws-annotation-meta">
              <span>{describeTarget(mark.payload.target)}</span>
              <span data-testid="aiws-anchor-state">{mark.anchorState === 'resolved' ? '锚点有效' : mark.anchorState === 'target_deleted' ? '目标已删除' : mark.anchorState}</span>
              {mark.payload.author && <span>{mark.payload.author}</span>}
              {envelope && (envelope.capabilities.includes('comment') || envelope.capabilities.includes('manage')) && (
                <button type="button" className="aiws-link" onClick={() => {
                  void store.submit({ editId: `entity:${mark.entityId}`, label: '删除批注', operations: [{ op: 'entity.delete', entity_id: mark.entityId, expect: { rev: envelope.life_rev } }] })
                }}>删除</button>
              )}
            </div>
          </div>
        )
      })}
    </div>
  )
}

// ---- Mock run (design §7)

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

  const act = async (work: () => Promise<void>) => {
    setBusy(true)
    setMessage(null)
    try { await work() } catch (error) { setMessage(describeError(error)) } finally { setBusy(false) }
  }
  const start = () => act(async () => { setRun(await store.session.procStart(MOCK_PROGRAM, { today })) })
  const apply = () => act(async () => {
    if (!run) return
    const next = await store.session.procApply(run.run_id)
    setRun(next)
    const result = next.result
    if (result && result.status === 'accepted') {
      // One application is one commit: one entry on the undo stack (design §2.7).
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
      <div className="aiws-panel-title">受控加工 <span className="aiws-chip aiws-chip-derived" data-testid="aiws-mock-simulated">模拟结果</span></div>
      <div className="aiws-muted">程序 {MOCK_PROGRAM} 是确定性的模拟程序，不是 AI：它按“今天”计算未完成任务的风险并生成任务摘要。它的全部输出都是模拟结果。</div>
      <div className="aiws-inline-form">
        <label>今天 <input type="date" aria-label="今天" data-testid="aiws-mock-today" value={today} onChange={(event) => setToday(event.target.value)} /></label>
        <button type="button" data-testid="aiws-mock-start" disabled={busy} onClick={() => { void start() }}>生成候选</button>
      </div>
      {message && <div className="aiws-warning" role="status" data-testid="aiws-mock-message">{message}</div>}
      {run && (
        <div className="aiws-mock-run" data-testid="aiws-mock-run" data-state={run.state}>
          <div>运行 {run.run_id} · 状态 <b>{run.state}</b> · <span className="aiws-chip aiws-chip-derived">模拟结果</span></div>
          {prepare && prepare.status === 'ok' && (
            <div data-testid="aiws-mock-prepare">
              候选预检通过，将影响 {prepare.touched.length} 处：
              <ul>{prepare.touched.slice(0, 20).map((item, index) => <li key={index}>{touchedText(item)}</li>)}</ul>
            </div>
          )}
          {prepare && prepare.status !== 'ok' && (
            <div className="aiws-error" role="alert" data-testid="aiws-mock-prepare">
              候选预检未通过（{prepare.code}）：{prepare.status === 'conflict' ? `${prepare.conflicts.length} 处冲突` : prepare.errors?.[0]?.detail ?? ''}
            </div>
          )}
          {runError && <div className="aiws-warning" role="status" data-testid="aiws-mock-failed">运行没有产生候选（{runError.code}）：{runError.detail ?? ''}。文档没有任何变化。</div>}
          {run.warnings.length > 0 && (
            <div className="aiws-warning" data-testid="aiws-mock-warnings">
              警告（{run.warnings.length}）：<ul>{run.warnings.map((warning, index) => <li key={index}>{typeof warning === 'string' ? warning : JSON.stringify(warning)}</li>)}</ul>
            </div>
          )}
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

// ---- save states (design §6.5)

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
