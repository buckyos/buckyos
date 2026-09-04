/* ── Right property panel: block / wish / generated / chart / metric / step properties (PRD §8.4) ── */

import { Copy, Download, ExternalLink, ImagePlus, Lock, Play, RefreshCw, Sparkles, Trash2, Unlink, Unlock, Camera } from 'lucide-react'
import { toCsv } from '../data/csv'
import { formatBytes } from '../data/image'
import { imageBlockHeight } from '../domain/factories'
import { downloadText } from '../storage/export'
import { activeSheet, generatedStatus, refBlockId, tableToMatrix } from '../domain/selectors'
import type { CanvasBlock, ChartBlockContent, ImageBlockContent, MetricBlockContent } from '../domain/types'
import { useCanvasEditor, useStoreState } from '../store/hooks'
import { useEditorActions } from './actions'
import { STATUS_META, TYPE_LABEL, WISH_STATE_LABEL, formatTime } from './meta'
import { chartRows } from './chart-data'
import { Badge, Btn, Field, IconBtn, Input, Kbd, SectionTitle, Select, TextArea } from './primitives'

export function RightPanel() {
  const { doc, ui } = useStoreState()
  const blocks = ui.selection.map((id) => doc.blocks[id]).filter(Boolean) as CanvasBlock[]
  return (
    <aside className="aic-scroll flex h-full w-[300px] flex-none flex-col overflow-y-auto border-l border-[color:var(--cp-border)] bg-[color:var(--cp-surface-opaque)] p-3">
      {blocks.length === 0 ? (
        ui.sidebarTab === 'presentation' && ui.selectedStepId ? <StepPanel /> : <SheetPanel />
      ) : blocks.length === 1 ? (
        <BlockPanel block={blocks[0]} />
      ) : (
        <MultiPanel blocks={blocks} />
      )}
    </aside>
  )
}

function SheetPanel() {
  const { doc } = useStoreState()
  const sheet = activeSheet(doc)
  const shortcuts: Array<[string, string]> = [
    ['P', '创建许愿格'],
    ['F', '适应全部内容'],
    ['Ctrl/⌘ + Enter', '运行当前许愿格'],
    ['Ctrl/⌘ + Z / Shift+Z', '撤销 / 重做'],
    ['Ctrl/⌘ + C / V', '复制 / 粘贴块（也可粘贴 Excel 区域）'],
    ['Delete', '删除选中块'],
    ['右键 / 中键拖动', '平移画布（空白处右键：插入菜单）'],
    ['Space + 拖动', '平移画布'],
    ['Ctrl/⌘ + 滚轮', '缩放'],
    ['Esc', '取消选择 / 退出编辑'],
  ]
  return (
    <div className="space-y-4">
      <div>
        <SectionTitle>当前 Sheet</SectionTitle>
        <p className="text-sm font-semibold">{sheet.name}</p>
        <p className="text-[11px] text-[color:var(--cp-muted)]">{sheet.blockIds.length} 个块 · 缩放 {Math.round(sheet.camera.zoom * 100)}%</p>
      </div>
      <div>
        <SectionTitle>快捷键</SectionTitle>
        <ul className="space-y-1.5 text-[11px]">
          {shortcuts.map(([k, v]) => (
            <li key={k} className="flex items-start justify-between gap-2">
              <span className="text-[color:var(--cp-muted)]">{v}</span>
              <Kbd>{k}</Kbd>
            </li>
          ))}
        </ul>
      </div>
      <div className="rounded-md bg-[color:var(--cp-surface-2-opaque)] p-2 text-[11px] leading-5 text-[color:var(--cp-muted)]">
        选中一个块后，这里会显示它的属性；选中 AI 生成的结果，可以看到数据来源、最后运行时间与刷新入口。
      </div>
    </div>
  )
}

function CommonSection({ block }: { block: CanvasBlock }) {
  const { store } = useCanvasEditor()
  const actions = useEditorActions()
  const num = (k: 'x' | 'y' | 'width' | 'height') => (
    <label key={k} className="block">
      <span className="text-[10px] uppercase text-[color:var(--cp-muted)]">{{ x: 'X', y: 'Y', width: '宽', height: '高' }[k]}</span>
      <Input type="number" value={Math.round(block.rect[k])} disabled={block.locked} onChange={(e) => store.dispatch({ type: 'UPDATE_BLOCK', id: block.id, patch: { rect: { ...block.rect, [k]: Number(e.target.value) } } })} className="!py-1" />
    </label>
  )
  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2">
        <Badge tone={block.type === 'wish' ? 'ai' : 'neutral'}>{TYPE_LABEL[block.type]}</Badge>
        <span className="ml-auto flex">
          <IconBtn icon={block.locked ? <Unlock /> : <Lock />} label={block.locked ? '解除锁定' : '锁定位置'} onClick={() => store.dispatch({ type: 'UPDATE_BLOCK', id: block.id, patch: { locked: !block.locked } })} />
          <IconBtn icon={<Copy />} label="创建副本" onClick={() => actions.duplicateBlocks([block.id])} />
          <IconBtn icon={<Trash2 />} label="删除" onClick={() => actions.deleteBlocks([block.id])} />
        </span>
      </div>
      <Field label="标题">
        <Input value={block.title ?? ''} onChange={(e) => store.dispatch({ type: 'UPDATE_BLOCK', id: block.id, patch: { title: e.target.value }, userEdit: true })} />
      </Field>
      <div className="grid grid-cols-4 gap-1.5">{(['x', 'y', 'width', 'height'] as const).map(num)}</div>
    </div>
  )
}

function GeneratedSection({ block }: { block: CanvasBlock }) {
  const { doc } = useStoreState()
  const actions = useEditorActions()
  const meta = block.generated
  if (!meta || meta.detached) return null
  const status = generatedStatus(doc, block)
  const sm = status && status !== 'never_run' ? STATUS_META[status] : null
  const wish = doc.blocks[meta.wishBlockId]
  const group = block.type === 'group' ? block : Object.values(doc.blocks).find((g) => g.type === 'group' && g.content.childBlockIds.includes(block.id))
  return (
    <div className="space-y-2 rounded-md border border-[color:color-mix(in_srgb,var(--aic-ai)_40%,transparent)] bg-[color:var(--aic-ai-soft)] p-2.5">
      <div className="flex items-center gap-1.5 text-xs font-semibold text-[color:var(--aic-ai)]">
        <Sparkles className="size-[13px]" /> AI 生成
        {sm ? <Badge tone={sm.tone} className="ml-auto">{sm.glyph} {sm.label}</Badge> : null}
      </div>
      <dl className="grid grid-cols-[64px_1fr] gap-x-2 gap-y-1 text-[11px]">
        <dt className="text-[color:var(--cp-muted)]">来源</dt>
        <dd>
          {wish ? (
            <button type="button" className="inline-flex items-center gap-1 hover:underline" onClick={() => actions.focusBlock(wish.id)}>
              {wish.title ?? '许愿格'} <ExternalLink className="size-[11px]" />
            </button>
          ) : (
            '许愿格已删除'
          )}
        </dd>
        <dt className="text-[color:var(--cp-muted)]">数据</dt>
        <dd>
          {meta.sourceRevisions.length ? meta.sourceRevisions.map((s) => {
            const b = doc.blocks[refBlockId(s.refKey)]
            return (
              <div key={s.refKey} className="flex items-center gap-1">
                {b ? <button type="button" className="truncate hover:underline" onClick={() => actions.focusBlock(b.id)}>{b.title ?? b.type}</button> : <span className="text-[color:var(--cp-danger)]">来源已删除</span>}
                {b && b.dataRevision !== s.revision ? <span className="text-[color:var(--cp-warning)]">（已变化）</span> : null}
              </div>
            )
          }) : '无'}
        </dd>
        <dt className="text-[color:var(--cp-muted)]">最后运行</dt>
        <dd>{formatTime(meta.generatedAt)} · {meta.agentAdapter === 'mock' ? 'Mock' : 'HTTP'}</dd>
        {meta.userModified ? (<><dt className="text-[color:var(--cp-muted)]">手工修改</dt><dd>是，刷新前会询问</dd></>) : null}
      </dl>
      {meta.assumptions?.length ? (
        <div className="text-[11px]"><span className="text-[color:var(--cp-muted)]">假设：</span>{meta.assumptions.join('；')}</div>
      ) : null}
      {meta.warnings?.length ? (
        <div className="text-[11px] text-[color:color-mix(in_srgb,var(--cp-warning)_60%,var(--cp-text))]"><span>警告：</span>{meta.warnings.join('；')}</div>
      ) : null}
      <div className="flex flex-wrap gap-1">
        <Btn variant="primary" icon={<RefreshCw />} className="!py-[3px]" disabled={!wish} onClick={() => wish && actions.runWish(wish.id)}>
          重新运行
        </Btn>
        {group ? (
          <Btn variant="subtle" icon={<Unlink />} className="!py-[3px]" onClick={() => actions.detachGroup(group.id)}>
            解除 AI 管理
          </Btn>
        ) : null}
      </div>
    </div>
  )
}

function BlockPanel({ block }: { block: CanvasBlock }) {
  return (
    <div className="space-y-4">
      <CommonSection block={block} />
      <GeneratedSection block={block} />
      {block.type === 'table' ? <TableSection block={block} /> : null}
      {block.type === 'wish' ? <WishSection block={block} /> : null}
      {block.type === 'chart' ? <ChartSection block={block} /> : null}
      {block.type === 'metric' ? <MetricSection block={block} /> : null}
      {block.type === 'frame' ? <FrameSection block={block} /> : null}
      {block.type === 'image' ? <ImageSection block={block} /> : null}
      {block.type === 'video' ? <VideoSection block={block} /> : null}
      {block.type === 'text' ? <p className="text-[11px] text-[color:var(--cp-muted)]">双击文本块进入编辑，支持 Markdown 标题、列表、加粗与链接。</p> : null}
    </div>
  )
}

function TableSection({ block }: { block: Extract<CanvasBlock, { type: 'table' }> }) {
  const { ui } = useStoreState()
  const { store } = useCanvasEditor()
  const actions = useEditorActions()
  const c = block.content
  const sel = ui.tableSelection?.blockId === block.id ? ui.tableSelection.range : null
  const types = c.columns.map((x) => x.inferredType ?? 'string')
  const counts = types.reduce<Record<string, number>>((a, t) => ({ ...a, [t]: (a[t] ?? 0) + 1 }), {})
  return (
    <div className="space-y-3">
      <SectionTitle>数据</SectionTitle>
      <dl className="grid grid-cols-[64px_1fr] gap-x-2 gap-y-1 text-[11px]">
        <dt className="text-[color:var(--cp-muted)]">规模</dt>
        <dd>{c.rows.length} 行 × {c.columns.length} 列</dd>
        <dt className="text-[color:var(--cp-muted)]">列类型</dt>
        <dd>{Object.entries(counts).map(([t, n]) => `${{ string: '文本', number: '数字', date: '日期', boolean: '布尔', null: '空' }[t] ?? t} ${n}`).join(' · ')}</dd>
        <dt className="text-[color:var(--cp-muted)]">来源</dt>
        <dd>{c.source?.filename ? `${c.source.filename}${c.source.worksheet ? ` / ${c.source.worksheet}` : ''}` : c.source?.kind === 'paste' ? '粘贴' : '手动创建'}{c.source?.importedAt ? ` · ${formatTime(c.source.importedAt)}` : ''}</dd>
        <dt className="text-[color:var(--cp-muted)]">修订</dt>
        <dd>dataRevision {block.dataRevision}</dd>
        {c.source?.truncated ? (<><dt className="text-[color:var(--cp-muted)]">截断</dt><dd className="text-[color:var(--cp-warning)]">原 {c.source.truncated.originalRows} 行，保留 {c.source.truncated.keptRows} 行</dd></>) : null}
      </dl>
      <div className="flex flex-wrap gap-1">
        <Btn variant="primary" icon={<Sparkles />} className="!py-[3px]" onClick={() => actions.createWishFromTable(block.id)}>基于此数据创建许愿格</Btn>
        {sel ? (
          <Btn variant="subtle" icon={<Sparkles />} className="!py-[3px]" onClick={() => actions.createWishFromTable(block.id, sel)}>以选区创建许愿格</Btn>
        ) : null}
        <Btn variant="subtle" className="!py-[3px]" onClick={() => store.dispatch({ type: 'TABLE_STRUCTURE', id: block.id, action: { kind: 'addRow' } })}>添加行</Btn>
        <Btn variant="subtle" className="!py-[3px]" onClick={() => store.dispatch({ type: 'TABLE_STRUCTURE', id: block.id, action: { kind: 'addColumn' } })}>添加列</Btn>
        <Btn variant="subtle" icon={<Download />} className="!py-[3px]" onClick={() => downloadText(`${block.title ?? 'table'}.csv`, `\uFEFF${toCsv(tableToMatrix(c))}`, 'text/csv')}>导出 CSV</Btn>
      </div>
      <p className="text-[11px] leading-5 text-[color:var(--cp-muted)]">右键单元格可"转为 AI 单元格"，用一句话让 AI 计算这一行；双击表头重命名列。</p>
    </div>
  )
}

function WishSection({ block }: { block: Extract<CanvasBlock, { type: 'wish' }> }) {
  const { store } = useCanvasEditor()
  const actions = useEditorActions()
  const c = block.content
  const update = (patch: Partial<typeof c>) => store.dispatch({ type: 'UPDATE_BLOCK', id: block.id, patch: { content: { ...c, ...patch } } })
  return (
    <div className="space-y-3">
      <SectionTitle>运行方式</SectionTitle>
      <Field label="输出偏好">
        <Select value={c.outputPreference} onChange={(e) => update({ outputPreference: e.target.value as typeof c.outputPreference })}>
          <option value="auto">自动决定</option>
          <option value="table">生成表格</option>
          <option value="visual">生成图表与指标</option>
          <option value="brief">生成汇报摘要</option>
        </Select>
      </Field>
      <Field label="数据来源变化后" hint="原型中的自动运行只在页面打开期间生效；含手工修改的结果不会被自动覆盖。">
        <Select value={c.refreshPolicy.mode} onChange={(e) => update({ refreshPolicy: { mode: e.target.value as typeof c.refreshPolicy.mode } })}>
          <option value="manual">仅手动刷新</option>
          <option value="notify_on_change">提醒"需要刷新"（默认）</option>
          <option value="on_change">自动重新运行</option>
        </Select>
      </Field>
      <Btn variant="primary" icon={<Play />} onClick={() => actions.runWish(block.id)} disabled={!c.prompt.trim()}>运行 (Ctrl/⌘+Enter)</Btn>
      <div>
        <SectionTitle>运行历史（最近 10 次）</SectionTitle>
        {c.runHistory.length === 0 ? <p className="text-[11px] text-[color:var(--cp-muted)]">尚未运行</p> : (
          <ul className="space-y-1.5">
            {c.runHistory.map((h) => (
              <li key={h.runId} className="rounded border border-[color:var(--cp-border)] p-1.5 text-[11px]">
                <div className="flex items-center gap-1">
                  <Badge tone={h.status === 'succeeded' ? 'success' : h.status === 'failed' ? 'danger' : 'neutral'}>{WISH_STATE_LABEL[h.status]}</Badge>
                  <span className="text-[color:var(--cp-muted)]">{formatTime(h.startedAt)} · {h.adapter}</span>
                  {h.groupId ? <IconBtn icon={<ExternalLink />} label="定位结果" size={18} className="ml-auto" onClick={() => actions.focusBlock(h.groupId!)} /> : null}
                </div>
                <div className="mt-0.5 truncate text-[color:var(--cp-muted)]" title={h.promptExcerpt}>{h.promptExcerpt}</div>
                {h.error ? <div className="text-[color:var(--cp-danger)]">{h.error}</div> : null}
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  )
}

function ChartSection({ block }: { block: Extract<CanvasBlock, { type: 'chart' }> }) {
  const { doc } = useStoreState()
  const { store } = useCanvasEditor()
  const c = block.content
  const rows = chartRows(doc, c)
  const fields = rows.length ? Object.keys(rows[0]) : []
  const update = (patch: Partial<ChartBlockContent>) => store.dispatch({ type: 'UPDATE_BLOCK', id: block.id, patch: { content: { ...c, ...patch } }, userEdit: true })
  const ys = c.yFields ?? []
  return (
    <div className="space-y-3">
      <SectionTitle>图表</SectionTitle>
      <Field label="类型">
        <Select value={c.chartType} onChange={(e) => update({ chartType: e.target.value as ChartBlockContent['chartType'] })}>
          <option value="bar">柱状图</option>
          <option value="horizontalBar">横向条形图</option>
          <option value="line">折线图</option>
          <option value="pie">饼图</option>
        </Select>
      </Field>
      <Field label="分类字段 (X)">
        <Select value={c.xField ?? ''} onChange={(e) => update({ xField: e.target.value })}>
          {fields.map((f) => <option key={f} value={f}>{f}</option>)}
        </Select>
      </Field>
      <Field label="数值字段 (Y)">
        <div className="space-y-1">
          {fields.filter((f) => f !== c.xField).map((f) => (
            <label key={f} className="flex items-center gap-2 text-xs">
              <input type="checkbox" checked={ys.includes(f)} onChange={(e) => update({ yFields: e.target.checked ? [...ys, f] : ys.filter((y) => y !== f) })} />
              {f}
            </label>
          ))}
        </div>
      </Field>
      <div className="grid grid-cols-2 gap-2">
        <Field label="聚合">
          <Select value={c.aggregation ?? 'sum'} onChange={(e) => update({ aggregation: e.target.value as ChartBlockContent['aggregation'] })}>
            <option value="sum">求和</option><option value="avg">平均</option><option value="count">计数</option><option value="min">最小</option><option value="max">最大</option>
          </Select>
        </Field>
        <Field label="数值格式">
          <Select value={c.numberFormat ?? 'plain'} onChange={(e) => update({ numberFormat: e.target.value as ChartBlockContent['numberFormat'] })}>
            <option value="plain">普通</option><option value="currency">货币</option><option value="percent">百分比</option>
          </Select>
        </Field>
      </div>
      <Field label="说明">
        <Input value={c.caption ?? ''} onChange={(e) => update({ caption: e.target.value })} />
      </Field>
      <p className="text-[11px] text-[color:var(--cp-muted)]">数据来源：{c.data.kind === 'inline' ? `内嵌 ${c.data.rows.length} 行` : `表格 ${doc.blocks[c.data.blockId]?.title ?? '（已删除）'}`}。修改图表设置不需要重新运行 Agent。</p>
    </div>
  )
}

function MetricSection({ block }: { block: Extract<CanvasBlock, { type: 'metric' }> }) {
  const { store } = useCanvasEditor()
  const c = block.content
  const update = (patch: Partial<MetricBlockContent>) => store.dispatch({ type: 'UPDATE_BLOCK', id: block.id, patch: { content: { ...c, ...patch } }, userEdit: true })
  return (
    <div className="space-y-3">
      <SectionTitle>指标</SectionTitle>
      <Field label="名称"><Input value={c.label} onChange={(e) => update({ label: e.target.value })} /></Field>
      <Field label="数值"><Input value={String(c.value)} onChange={(e) => update({ value: Number.isFinite(Number(e.target.value)) && e.target.value.trim() !== '' ? Number(e.target.value) : e.target.value })} /></Field>
      <div className="grid grid-cols-2 gap-2">
        <Field label="格式">
          <Select value={c.format ?? 'plain'} onChange={(e) => update({ format: e.target.value as MetricBlockContent['format'] })}>
            <option value="plain">普通</option><option value="currency">货币</option><option value="percent">百分比</option>
          </Select>
        </Field>
        <Field label="色调">
          <Select value={c.tone ?? 'neutral'} onChange={(e) => update({ tone: e.target.value as MetricBlockContent['tone'] })}>
            <option value="neutral">中性</option><option value="positive">正向</option><option value="negative">负向</option><option value="warning">警示</option>
          </Select>
        </Field>
      </div>
      <Field label="备注"><Input value={c.note ?? ''} onChange={(e) => update({ note: e.target.value })} /></Field>
    </div>
  )
}

function FrameSection({ block }: { block: Extract<CanvasBlock, { type: 'frame' }> }) {
  const { store } = useCanvasEditor()
  const c = block.content
  return (
    <div className="space-y-3">
      <SectionTitle>框架</SectionTitle>
      <label className="flex items-center gap-2 text-xs">
        <input type="checkbox" checked={c.moveChildren} onChange={(e) => store.dispatch({ type: 'UPDATE_BLOCK', id: block.id, patch: { content: { ...c, moveChildren: e.target.checked } } })} />
        移动框架时连同内部块一起移动
      </label>
      <Field label="颜色">
        <div className="flex gap-1">
          {['var(--cp-accent)', 'var(--cp-success)', 'var(--cp-warning)', 'var(--cp-danger)', 'var(--cp-accent-soft)'].map((col) => (
            <button key={col} type="button" aria-label="颜色" className="h-6 w-6 rounded-full border border-[color:var(--cp-border-opaque)]" style={{ background: col, outline: c.color === col ? '2px solid var(--cp-text)' : undefined }} onClick={() => store.dispatch({ type: 'UPDATE_BLOCK', id: block.id, patch: { content: { ...c, color: col } } })} />
          ))}
        </div>
      </Field>
      <p className="text-[11px] text-[color:var(--cp-muted)]">框架用于视觉分区，也可以作为讲述路径的步骤目标。拖动框架标题移动它。</p>
    </div>
  )
}

function ImageSection({ block }: { block: Extract<CanvasBlock, { type: 'image' }> }) {
  const { store } = useCanvasEditor()
  const actions = useEditorActions()
  const c = block.content
  const update = (patch: Partial<ImageBlockContent>) => store.dispatch({ type: 'UPDATE_BLOCK', id: block.id, patch: { content: { ...c, ...patch } }, userEdit: true })
  const src = c.source
  return (
    <div className="space-y-3">
      <SectionTitle>图片</SectionTitle>
      <div className="flex flex-wrap gap-1">
        <Btn variant="primary" icon={<ImagePlus />} className="!py-[3px]" disabled={block.locked} onClick={() => actions.pickImageFor(block.id)}>{c.src ? '替换图片…' : '选择图片…'}</Btn>
        {c.src && c.naturalWidth && c.naturalHeight ? (
          <Btn variant="subtle" className="!py-[3px]" disabled={block.locked} onClick={() => store.dispatch({ type: 'UPDATE_BLOCK', id: block.id, patch: { rect: { ...block.rect, height: imageBlockHeight(block.rect.width, c.naturalWidth, c.naturalHeight) } } })}>按原图比例</Btn>
        ) : null}
      </div>
      <Field label="显示方式">
        <Select value={c.fit} onChange={(e) => update({ fit: e.target.value as ImageBlockContent['fit'] })}>
          <option value="contain">完整显示（留白）</option>
          <option value="cover">填满裁切</option>
        </Select>
      </Field>
      <Field label="说明文字（显示在图片下方）"><Input value={c.caption ?? ''} onChange={(e) => update({ caption: e.target.value })} /></Field>
      <Field label="替代文本 / 给 AI 的描述" hint="作为许愿格数据来源时，会连同图片一起发送给 Agent。"><TextArea rows={3} value={c.alt ?? ''} onChange={(e) => update({ alt: e.target.value })} /></Field>
      <dl className="grid grid-cols-[64px_1fr] gap-x-2 gap-y-1 text-[11px]">
        <dt className="text-[color:var(--cp-muted)]">尺寸</dt>
        <dd>{c.naturalWidth && c.naturalHeight ? `${c.naturalWidth} × ${c.naturalHeight}` : '—'}{src?.bytes ? ` · ${formatBytes(src.bytes)}` : ''}</dd>
        <dt className="text-[color:var(--cp-muted)]">来源</dt>
        <dd className="break-words">{src?.kind === 'generated' ? `AI 生成${src.prompt ? `：${src.prompt}` : ''}` : src?.filename ?? (c.src ? '外部地址' : '尚未选择')}</dd>
      </dl>
    </div>
  )
}

function VideoSection({ block }: { block: Extract<CanvasBlock, { type: 'video' }> }) {
  const { store } = useCanvasEditor()
  const c = block.content
  const total = c.durationMs ?? (c.frames ?? []).reduce((n, f) => n + f.durationMs, 0)
  return (
    <div className="space-y-3">
      <SectionTitle>视频</SectionTitle>
      <Field label="说明文字"><Input value={c.caption ?? ''} onChange={(e) => store.dispatch({ type: 'UPDATE_BLOCK', id: block.id, patch: { content: { ...c, caption: e.target.value } }, userEdit: true })} /></Field>
      <dl className="grid grid-cols-[64px_1fr] gap-x-2 gap-y-1 text-[11px]">
        <dt className="text-[color:var(--cp-muted)]">形式</dt>
        <dd>{c.src ? '视频文件' : c.frames?.length ? `逐帧预览 · ${c.frames.length} 个镜头` : '空'}</dd>
        <dt className="text-[color:var(--cp-muted)]">时长</dt>
        <dd>{total ? `${(total / 1000).toFixed(1)} 秒` : '—'}</dd>
        <dt className="text-[color:var(--cp-muted)]">来源</dt>
        <dd className="break-words">{c.source?.kind === 'generated' ? `AI 生成${c.source.prompt ? `：${c.source.prompt}` : ''}` : c.source?.filename ?? '—'}</dd>
      </dl>
      {c.src ? <Btn variant="subtle" icon={<Download />} className="!py-[3px]" onClick={() => downloadText(`${block.title ?? 'video'}.txt`, c.src!, 'text/plain')}>导出地址</Btn> : null}
      <p className="text-[11px] leading-5 text-[color:var(--cp-muted)]">Mock Agent 生成的是逐帧预览；接入真实视频模型后，这里会是可播放、可下载的视频文件。</p>
    </div>
  )
}

function MultiPanel({ blocks }: { blocks: CanvasBlock[] }) {
  const { doc, ui } = useStoreState()
  const actions = useEditorActions()
  const path = doc.presentationPaths.find((p) => p.id === ui.selectedPathId) ?? doc.presentationPaths[0]
  return (
    <div className="space-y-3">
      <SectionTitle>已选中 {blocks.length} 个块</SectionTitle>
      <div className="flex flex-wrap gap-1">
        <Btn variant="subtle" icon={<Copy />} onClick={() => actions.duplicateBlocks(blocks.map((b) => b.id))}>创建副本</Btn>
        <Btn variant="danger" icon={<Trash2 />} onClick={() => actions.deleteBlocks(blocks.map((b) => b.id))}>删除</Btn>
        {path ? <Btn variant="primary" icon={<Camera />} onClick={() => actions.addStepFromBlocks(path.id, blocks.map((b) => b.id))}>加入讲述路径「{path.name}」</Btn> : null}
      </div>
      <ul className="space-y-1 text-xs">
        {blocks.map((b) => (
          <li key={b.id} className="flex items-center gap-2"><Badge>{TYPE_LABEL[b.type]}</Badge><span className="truncate">{b.title ?? b.type}</span></li>
        ))}
      </ul>
    </div>
  )
}

function StepPanel() {
  const { doc, ui } = useStoreState()
  const { store } = useCanvasEditor()
  const actions = useEditorActions()
  const path = doc.presentationPaths.find((p) => p.id === ui.selectedPathId) ?? doc.presentationPaths[0]
  const step = path?.steps.find((s) => s.id === ui.selectedStepId)
  if (!path || !step) return <SheetPanel />
  const idx = path.steps.indexOf(step)
  const update = (patch: Partial<typeof step>) => store.dispatch({ type: 'PRESENTATION_UPDATE_STEP', pathId: path.id, stepId: step.id, patch })
  return (
    <div className="space-y-3">
      <SectionTitle>讲述步骤 {idx + 1} / {path.steps.length}</SectionTitle>
      <Field label="标题"><Input value={step.title ?? ''} onChange={(e) => update({ title: e.target.value })} /></Field>
      <Field label="讲述说明"><TextArea rows={4} value={step.note ?? ''} onChange={(e) => update({ note: e.target.value })} placeholder="播放时显示在屏幕下方的提示" /></Field>
      <Field label="切换时长 (毫秒)"><Input type="number" value={step.transitionMs} onChange={(e) => update({ transitionMs: Math.max(0, Number(e.target.value)) })} /></Field>
      <div className="text-[11px] text-[color:var(--cp-muted)]">镜头：({Math.round(step.camera.x)}, {Math.round(step.camera.y)}) · {Math.round(step.camera.zoom * 100)}% · 目标块 {step.targetBlockIds.length}</div>
      <div className="flex flex-wrap gap-1">
        <Btn variant="subtle" icon={<Camera />} onClick={() => update({ camera: activeSheet(store.doc).camera })}>用当前视口更新镜头</Btn>
        <Btn variant="primary" icon={<Play />} onClick={() => actions.startPresentation(path.id, idx)}>从此步播放</Btn>
      </div>
    </div>
  )
}
