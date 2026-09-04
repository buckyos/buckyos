import { useEffect, useRef, useState } from 'react'
import type { CanvasBlockOf } from '../../domain/types'
import { useCanvasEditor } from '../../store/hooks'
import { Markdown } from '../markdown'
import { trackEvent } from '../../events'

export function TextBlockView({ block, editing }: { block: CanvasBlockOf<'text'>; editing: boolean }) {
  const { store } = useCanvasEditor()
  const [draft, setDraft] = useState(block.content.text)
  const [prevEditing, setPrevEditing] = useState(editing)
  const ref = useRef<HTMLTextAreaElement>(null)
  if (editing !== prevEditing) {
    setPrevEditing(editing)
    if (editing) setDraft(block.content.text)
  }

  useEffect(() => {
    if (editing) requestAnimationFrame(() => ref.current?.focus())
  }, [editing])

  const commit = () => {
    if (draft !== block.content.text) {
      store.dispatch({ type: 'UPDATE_BLOCK', id: block.id, patch: { content: { ...block.content, text: draft } }, userEdit: true })
      if (block.generated) trackEvent('result_edited', { type: 'text' })
    }
    store.setUi({ editingBlockId: null })
  }

  if (editing) {
    return (
      <textarea
        ref={ref}
        data-no-drag
        className="aic-scroll h-full w-full resize-none bg-transparent p-3 text-[13px] leading-relaxed outline-none"
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === 'Escape') {
            e.stopPropagation()
            commit()
          }
        }}
        placeholder="支持 Markdown：# 标题、- 列表、**加粗**、[链接](url)"
      />
    )
  }
  return (
    <div
      className="aic-scroll h-full w-full overflow-auto p-3 text-[13px]"
      onDoubleClick={(e) => {
        e.stopPropagation()
        if (!block.locked) store.setUi({ editingBlockId: block.id, selection: [block.id] })
      }}
    >
      {block.content.text.trim() ? <Markdown text={block.content.text} /> : <span className="text-[color:var(--cp-muted)]">双击输入文字</span>}
    </div>
  )
}
