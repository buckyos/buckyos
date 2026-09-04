/* ── Wish block: prompt + sources + output preference + run (PRD §11.7) ── */

import clsx from 'clsx'
import { AlertTriangle, ChevronDown, ChevronRight, Database, Play, Plus, Square, X } from 'lucide-react'
import { useMemo, useState } from 'react'
import { contextPreviewLines, buildContext } from '../../agent/context'
import { refKey, sheetBlocks, tableRangeLabel, wishStatus, wouldCreateCycle } from '../../domain/selectors'
import type { CanvasBlockOf, ContextRef } from '../../domain/types'
import { useCanvasEditor, useStoreState } from '../../store/hooks'
import { useEditorActions } from '../actions'
import { Badge, Btn, IconBtn, Menu, Select } from '../primitives'
import { RUNNING_STATES, STATUS_META, WISH_STATE_LABEL, formatTime } from '../meta'

export function WishBlockView({ block }: { block: CanvasBlockOf<'wish'> }) {
  const { doc, runs, ui, settings } = useStoreState()
  const { store } = useCanvasEditor()
  const actions = useEditorActions()
  const c = block.content
  const run = runs[block.id]
  const running = RUNNING_STATES.includes(c.state)
  const [sourceMenu, setSourceMenu] = useState<{ x: number; y: number } | null>(null)
  const [showDetails, setShowDetails] = useState(false)
  const status = wishStatus(doc, block)

  const candidates = useMemo(() => {
    const own = new Set(c.generatedGroupIds)
    return sheetBlocks(doc, block.sheetId).filter(
      (b) => b.id !== block.id && b.type !== 'wish' && b.type !== 'frame' && !own.has(b.id) && !(b.type === 'group' && b.generated?.wishBlockId === block.id),
    )
  }, [doc, block.id, block.sheetId, c.generatedGroupIds])

  const update = (patch: Partial<typeof c>) => store.dispatch({ type: 'UPDATE_BLOCK', id: block.id, patch: { content: { ...c, ...patch } } })

  const addRef = (ref: ContextRef) => {
    if (c.contextRefs.some((r) => refKey(r) === refKey(ref))) return
    if (wouldCreateCycle(doc, block.id, ref.blockId)) {
      store.toast('不能添加：该块依赖本许愿格的结果，会形成循环依赖', 'error')
      return
    }
    update({ contextRefs: [...c.contextRefs, ref] })
  }
  const removeRef = (key: string) => update({ contextRefs: c.contextRefs.filter((r) => refKey(r) !== key) })

  const selectedOthers = ui.selection.filter((id) => id !== block.id && doc.blocks[id] && doc.blocks[id].type !== 'wish' && doc.blocks[id].type !== 'frame')
  const contextLines = useMemo(() => contextPreviewLines(buildContext(doc, block).items), [doc, block])
  const generatedCount = c.generatedGroupIds.reduce((n, g) => {
    const b = doc.blocks[g]
    return n + (b?.type === 'group' ? b.content.childBlockIds.length : b ? 1 : 0)
  }, 0)

  return (
    <div className="flex h-full flex-col gap-2 p-2.5" data-no-drag>
      <textarea
        className="aic-input aic-scroll min-h-[64px] flex-1 resize-none text-[13px] leading-relaxed"
        placeholder="在这里用一句话写下你想要的结果，例如：按区域汇总销售额并找出下滑最明显的产品…"
        value={c.prompt}
        disabled={running}
        onChange={(e) => update({ prompt: e.target.value })}
        onKeyDown={(e) => {
          if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') {
            e.preventDefault()
            actions.runWish(block.id)
          }
        }}
      />

      <div className="flex flex-wrap items-center gap-1">
        <span className="inline-flex items-center gap-1 text-[11px] text-[color:var(--cp-muted)]">
          <Database className="size-[12px]" /> 数据来源
        </span>
        {c.contextRefs.map((ref) => {
          const src = doc.blocks[ref.blockId]
          const key = refKey(ref)
          const label = !src ? '已删除的来源' : src.type === 'table' ? tableRangeLabel(src, ref) : src.title ?? src.type
          return (
            <span key={key} className={clsx('inline-flex max-w-[200px] items-center gap-1 rounded-full border px-2 py-[1px] text-[11px]', src ? 'border-[color:var(--cp-border-opaque)] bg-[color:var(--cp-surface-2-opaque)]' : 'border-[color:var(--cp-danger)] text-[color:var(--cp-danger)]')} title={label}>
              <button type="button" className="truncate hover:underline" onClick={() => src && actions.focusBlock(src.id)}>
                {label}
              </button>
              {!running ? (
                <button type="button" aria-label="移除来源" onClick={() => removeRef(key)} className="rounded-full hover:bg-[color:color-mix(in_srgb,var(--cp-text)_10%,transparent)]">
                  <X className="size-[11px]" />
                </button>
              ) : null}
            </span>
          )
        })}
        <div className="relative">
          <Btn variant="ghost" icon={<Plus />} disabled={running} onClick={(e) => setSourceMenu({ x: 0, y: (e.currentTarget as HTMLElement).offsetHeight + 2 })} className="!px-1.5 !py-[2px]">
            添加
          </Btn>
          {sourceMenu ? (
            <Menu
              at={sourceMenu}
              onClose={() => setSourceMenu(null)}
              items={[
                ...(selectedOthers.length
                  ? [{ label: `使用当前选中的 ${selectedOthers.length} 个块`, onClick: () => selectedOthers.forEach((id) => addRef({ kind: 'block', blockId: id, revision: doc.blocks[id].dataRevision })) }, { label: '', divider: true, onClick: () => undefined }]
                  : []),
                ...(candidates.length
                  ? candidates.map((b) => ({
                      label: `${b.title ?? b.type}${b.type === 'table' ? `（${b.content.rows.length} 行）` : b.generated ? '（AI 结果）' : ''}`,
                      disabled: c.contextRefs.some((r) => r.kind === 'block' && r.blockId === b.id),
                      onClick: () => addRef({ kind: 'block', blockId: b.id, revision: b.dataRevision }),
                    }))
                  : [{ label: '当前 Sheet 没有可引用的表格或文本', disabled: true, onClick: () => undefined }]),
                { label: '', divider: true, onClick: () => undefined },
                { label: '提示：在表格中选中区域后，右侧面板可"以选区创建许愿格"', disabled: true, onClick: () => undefined },
              ]}
            />
          ) : null}
        </div>
      </div>

      <div className="flex items-center gap-2">
        <Select value={c.outputPreference} disabled={running} onChange={(e) => update({ outputPreference: e.target.value as typeof c.outputPreference })} className="!w-auto !py-1">
          <option value="auto">自动决定输出</option>
          <option value="table">生成表格</option>
          <option value="visual">生成图表与指标</option>
          <option value="brief">生成汇报摘要</option>
        </Select>
        <span className="ml-auto" />
        {running ? (
          <Btn variant="danger" icon={<Square />} onClick={() => actions.cancelWish(block.id)}>
            取消
          </Btn>
        ) : (
          <Btn variant="primary" icon={<Play />} onClick={() => actions.runWish(block.id)} className={clsx(ui.highlightRun && 'aic-highlight')} title="Ctrl/Cmd + Enter" disabled={!c.prompt.trim()}>
            {c.generatedGroupIds.length ? '重新运行' : '运行'}
          </Btn>
        )}
      </div>

      <div className="rounded-md border border-[color:var(--cp-border)] bg-[color:var(--cp-surface-2-opaque)] px-2 py-1.5 text-[11px]">
        <div className="flex items-center gap-2">
          <Badge tone={c.state === 'failed' ? 'danger' : running ? 'accent' : c.state === 'succeeded' ? 'success' : 'neutral'}>{WISH_STATE_LABEL[c.state]}</Badge>
          {running && run ? <span className="truncate text-[color:var(--cp-muted)]">{run.message}</span> : null}
          {!running && c.generatedGroupIds.length ? (
            <>
              <span className="text-[color:var(--cp-muted)]">已生成 {generatedCount} 个结果</span>
              {status !== 'never_run' ? <Badge tone={STATUS_META[status].tone}>{STATUS_META[status].glyph} {STATUS_META[status].label}</Badge> : null}
            </>
          ) : null}
          <span className="ml-auto whitespace-nowrap text-[color:var(--cp-muted)]">{c.runHistory[0] ? `最后运行 ${formatTime(c.runHistory[0].startedAt)}` : ''}</span>
        </div>
        {running ? (
          <div className={clsx('aic-progress mt-1.5', run?.percent === undefined && 'is-indeterminate')}>
            <i style={{ width: run?.percent !== undefined ? `${run.percent}%` : undefined }} />
          </div>
        ) : null}
        {c.state === 'failed' && c.lastError ? (
          <div className="mt-1.5 flex items-start gap-1.5 text-[color:var(--cp-danger)]">
            <AlertTriangle className="mt-[1px] size-[12px] shrink-0" />
            <div className="min-w-0 flex-1">
              <div>{c.lastError}</div>
              {run?.errorDetails?.length ? <ul className="mt-1 list-disc pl-4 text-[color:var(--cp-muted)]">{run.errorDetails.slice(0, 5).map((d, i) => <li key={i}>{d}</li>)}</ul> : null}
              <div className="mt-1 flex gap-1">
                <Btn variant="subtle" className="!py-[2px]" onClick={() => actions.runWish(block.id)}>
                  重试
                </Btn>
                {settings.adapter === 'http' ? (
                  <Btn variant="subtle" className="!py-[2px]" onClick={() => actions.runWish(block.id, { adapterId: 'mock' })}>
                    改用 Mock 模式重试
                  </Btn>
                ) : null}
                {run?.errorDetails?.length ? (
                  <Btn variant="ghost" className="!py-[2px]" onClick={() => navigator.clipboard?.writeText([c.lastError, ...(run.errorDetails ?? [])].join('\n'))}>
                    复制校验摘要
                  </Btn>
                ) : null}
              </div>
            </div>
          </div>
        ) : null}
        <button type="button" className="mt-1 flex items-center gap-1 text-[10px] text-[color:var(--cp-muted)] hover:text-[color:var(--cp-text)]" onClick={() => setShowDetails((v) => !v)}>
          {showDetails ? <ChevronDown className="size-[11px]" /> : <ChevronRight className="size-[11px]" />} 技术详情与将发送的上下文
        </button>
        {showDetails ? (
          <div className="aic-scroll mt-1 max-h-[120px] overflow-auto rounded bg-[color:var(--cp-surface-opaque)] p-1.5 font-mono text-[10px] leading-4 text-[color:var(--cp-muted)]">
            <div>Agent：{settings.adapter === 'http' ? `HTTP ${settings.httpBaseUrl || '(未配置)'}` : 'Mock（离线）'}</div>
            <div>将发送：{contextLines.length ? contextLines.join('；') : '无数据来源（仅发送目标）'}</div>
            <div>刷新策略：{c.refreshPolicy.mode === 'manual' ? '手动' : c.refreshPolicy.mode === 'on_change' ? '来源变化后自动运行' : '来源变化后提醒'}</div>
            {run?.log.map((l, i) => (
              <div key={i} className={clsx(l.level === 'error' && 'text-[color:var(--cp-danger)]', l.level === 'warning' && 'text-[color:var(--cp-warning)]')}>
                {new Date(l.at).toLocaleTimeString('zh-CN')} {l.text}
              </div>
            ))}
          </div>
        ) : null}
      </div>
      {ui.highlightBlockId === block.id ? <IconBtn icon={<span />} label="" className="hidden" /> : null}
    </div>
  )
}
