/* ── Result group header: AI badge, freshness, re-run / sources / detach (PRD FR-RESULT-002) ── */

import { Copy, ExternalLink, MoreHorizontal, RefreshCw, Sparkles, Trash2, Unlink, PencilLine } from 'lucide-react'
import type { CanvasBlockOf } from '../../domain/types'
import { generatedStatus } from '../../domain/selectors'
import { useCanvasEditor, useStoreState } from '../../store/hooks'
import { useEditorActions } from '../actions'
import { Badge, IconBtn, MenuButton } from '../primitives'
import { STATUS_META, formatTime } from '../meta'

export function GroupHeader({ block }: { block: CanvasBlockOf<'group'> }) {
  const { doc, runs } = useStoreState()
  const { store } = useCanvasEditor()
  const actions = useEditorActions()
  const meta = block.generated
  const status = meta && !meta.detached ? generatedStatus(doc, block) : null
  const wishId = meta?.wishBlockId
  const running = wishId ? Boolean(runs[wishId] && !['succeeded', 'failed', 'cancelled', 'idle'].includes(runs[wishId].stage)) : false
  const sm = status && status !== 'never_run' ? STATUS_META[status] : null

  return (
    <div className="aic-head pointer-events-auto rounded-t-[10px]" style={{ background: 'color-mix(in srgb, var(--aic-ai) 8%, var(--cp-surface-opaque))', borderBottom: 'none' }}>
      <Sparkles className="size-[13px] text-[color:var(--aic-ai)]" />
      <span className="truncate">{block.title ?? '生成结果'}</span>
      {meta && !meta.detached ? (
        <>
          <Badge tone="ai">AI 生成</Badge>
          {sm ? (
            <Badge tone={sm.tone} title={status === 'stale' ? '数据来源已变化，结果可能过期' : status === 'broken' ? '数据来源已删除' : '数据来源未变化'}>
              {sm.glyph} {sm.label}
            </Badge>
          ) : null}
          {meta.userModified ? (
            <Badge tone="neutral" icon={<PencilLine />} title="包含手工修改，刷新前会询问">
              已手工修改
            </Badge>
          ) : null}
          <span className="ml-auto text-[10px] font-normal text-[color:var(--cp-muted)]">{formatTime(meta.generatedAt)}</span>
          <span data-no-drag className="flex items-center">
            <IconBtn icon={<RefreshCw className={running ? 'animate-spin' : ''} />} label="重新运行" size={22} disabled={running || !wishId} onClick={() => wishId && actions.runWish(wishId)} />
            <IconBtn icon={<ExternalLink />} label="查看来源（许愿格）" size={22} onClick={() => wishId && actions.focusBlock(wishId)} />
            <MenuButton
              icon={<MoreHorizontal />}
              items={[
                { label: '查看来源许愿格', icon: <ExternalLink />, onClick: () => wishId && actions.focusBlock(wishId) },
                { label: '复制为新版本', icon: <Copy />, onClick: () => actions.duplicateBlocks([block.id, ...block.content.childBlockIds]) },
                { label: '解除 AI 管理', icon: <Unlink />, onClick: () => actions.detachGroup(block.id) },
                { label: '', onClick: () => undefined, divider: true },
                { label: '删除结果组', icon: <Trash2 />, danger: true, onClick: () => actions.deleteBlocks([block.id]) },
              ]}
            />
          </span>
        </>
      ) : (
        <>
          <Badge tone="neutral">已解除 AI 管理</Badge>
          <span data-no-drag className="ml-auto flex items-center">
            <IconBtn icon={<Trash2 />} label="删除分组（保留成员）" size={22} onClick={() => store.dispatch({ type: 'UPDATE_BLOCK', id: block.id, patch: { content: { ...block.content, childBlockIds: [] } } })} />
          </span>
        </>
      )}
    </div>
  )
}
