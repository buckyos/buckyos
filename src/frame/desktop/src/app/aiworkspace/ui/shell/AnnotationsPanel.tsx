/* Annotations in scope (design §3.7, phase two §8.4): the ones anchored to what is shown or placed
 * under a parent (a Surface's content folder, the selected data). New annotations are allowed with
 * the `comment` capability in edit and view mode alike (D13). */

import { useState } from 'react'
import type { CapturedAnchor } from '../../anchors/registry'
import { useStore, useWorkspaceUi } from '../../state/hooks'
import { annotationOp } from '../creators'
import { anchorStatus, annotationLabel, revealAnnotation } from '../annotationInfo'

export function AnnotationsPanel({ parentId, draft, onDraftDone }: { parentId: string | null; draft?: CapturedAnchor | null; onDraftDone?: () => void }) {
  const store = useStore()
  const { annotations, annotate, activeAnnotation, setActiveAnnotation, draft: uiDraft, clearDraft } = useWorkspaceUi() as ReturnType<typeof useWorkspaceUi> & { draft?: CapturedAnchor | null; clearDraft?: () => void }
  const [body, setBody] = useState('')
  const pending = draft ?? uiDraft ?? null
  const done = onDraftDone ?? clearDraft ?? (() => undefined)
  const target = parentId ?? 'data'
  return (
    <div className="aiws-annotations" data-testid="aiws-annotations">
      <div className="aiws-panel-title">批注 <span className="aiws-muted">{annotations.length}</span></div>
      {pending && (
        <form className="aiws-annotation-draft" onSubmit={(event) => {
          event.preventDefault()
          if (body.trim() === '') return
          void store.submit({ editId: `annotation:new`, label: `批注 ${pending.label}`, mine: body, hasMine: true, operations: [annotationOp(store.core, store.outline.childrenOf(target), target, pending, body.trim())] })
            .then((outcome) => { if (outcome.status === 'accepted' || outcome.status === 'saved_locally') { setBody(''); done() } })
        }}>
          <div data-testid="aiws-annotation-draft-target">批注对象：{pending.label}</div>
          {pending.context?.quote && pending.range && <div className="aiws-annotation-quote">{pending.context.quote.exact}</div>}
          <textarea aria-label="批注内容" autoFocus rows={2} value={body} onChange={(event) => setBody(event.target.value)} maxLength={4000} />
          <button type="submit" data-testid="aiws-annotation-save">保存批注</button>
          <button type="button" onClick={() => { setBody(''); done() }}>取消</button>
        </form>
      )}
      {annotations.length === 0 && !pending && (
        <div className="aiws-muted">{annotate ? '还没有批注。在表格单元格上点 ✎，在富文本里选中文字后点“批注”，或在画布上选中 Block 后点“批注”。' : '还没有批注。'}</div>
      )}
      {annotations.map((mark) => {
        const capabilities = mark.envelope.capabilities
        const quote = mark.payload.context?.quote?.exact
        const free = !mark.payload.target
        return (
          <div
            key={mark.entityId}
            className={`aiws-annotation${mark.entityId === activeAnnotation ? ' is-active' : ''}`}
            data-testid="aiws-annotation"
            data-annotation-id={mark.entityId}
            data-anchor-state={mark.anchor.state}
            data-anchor-level={mark.anchor.level}
            style={{ background: typeof mark.payload.style?.color === 'string' ? mark.payload.style.color : undefined }}
            onClick={() => { setActiveAnnotation(mark.entityId); revealAnnotation(mark.entityId) }}
          >
            <div className="aiws-annotation-body">{mark.payload.body}</div>
            {quote && mark.payload.range && mark.anchor.level !== 'range' && <div className="aiws-annotation-quote" title="批注时选中的原文">{quote}</div>}
            <div className="aiws-annotation-meta">
              <span>{free ? '自由便签' : annotationLabel(mark)}</span>
              {!free && <span data-testid="aiws-anchor-state">{anchorStatus(mark.anchor)}</span>}
              {mark.payload.author && <span>{mark.payload.author}</span>}
              {(capabilities.includes('comment') || capabilities.includes('manage')) && (
                <button type="button" className="aiws-link" onClick={(event) => {
                  event.stopPropagation()
                  void store.submit({ editId: `entity:${mark.entityId}`, label: '删除批注', operations: [{ op: 'entity.delete', entity_id: mark.entityId, expect: { rev: mark.envelope.life_rev } }] })
                }}>删除</button>
              )}
            </div>
          </div>
        )
      })}
    </div>
  )
}
