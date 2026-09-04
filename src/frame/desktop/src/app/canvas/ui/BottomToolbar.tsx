/* ── Bottom toolbar: add blocks, import, zoom ── */

import { Frame, Gauge, Image as ImageIcon, Maximize2, Minus, Plus, Sparkles, Table2, Type, Upload } from 'lucide-react'
import { activeSheet } from '../domain/selectors'
import type { PlacementTool } from '../store/canvas-store'
import { useCanvasEditor, useStoreState } from '../store/hooks'
import { useEditorActions } from './actions'
import { Btn, IconBtn } from './primitives'

const TOOLS: Array<{ id: PlacementTool; label: string; icon: React.ReactNode; hint: string }> = [
  { id: 'text', label: '文本', icon: <Type />, hint: '添加文本块' },
  { id: 'table', label: '表格', icon: <Table2 />, hint: '添加 10×5 空表格' },
  { id: 'image', label: '图片', icon: <ImageIcon />, hint: '插入图片（也可直接拖入或粘贴图片）' },
  { id: 'wish', label: '许愿格', icon: <Sparkles />, hint: '添加许愿格 (P)' },
  { id: 'metric', label: '指标', icon: <Gauge />, hint: '添加指标卡' },
  { id: 'frame', label: '框架', icon: <Frame />, hint: '添加区域框架' },
]

export function BottomToolbar() {
  const { doc, ui } = useStoreState()
  const { store } = useCanvasEditor()
  const actions = useEditorActions()
  const zoom = activeSheet(doc).camera.zoom
  return (
    <footer className="flex h-11 flex-none items-center gap-1 border-t border-[color:var(--cp-border)] bg-[color:var(--cp-surface-opaque)] px-2">
      <span className="mr-1 text-[11px] text-[color:var(--cp-muted)]">添加块</span>
      {TOOLS.map((t) => (
        <Btn key={t.id} variant="ghost" icon={t.icon} active={ui.tool === t.id} title={`${t.hint}（点击后在画布上单击放置，或直接双击按钮放到视口中心）`} onClick={() => store.setUi({ tool: ui.tool === t.id ? null : t.id })} onDoubleClick={() => actions.createBlockAt(t.id)}>
          {t.label}
        </Btn>
      ))}
      <span className="mx-1 h-5 w-px bg-[color:var(--cp-border)]" />
      <Btn variant="ghost" icon={<Upload />} onClick={() => actions.openImport()}>
        导入 Excel / CSV
      </Btn>
      {ui.tool ? <span className="ml-2 text-[11px] text-[color:var(--cp-accent)]">在画布上单击放置{TOOLS.find((t) => t.id === ui.tool)?.label}，Esc 取消</span> : null}
      <span className="flex-1" />
      <IconBtn icon={<Minus />} label="缩小" onClick={() => actions.zoomBy(1 / 1.2)} />
      <button type="button" className="w-[52px] rounded px-1 py-1 text-center text-[11px] tabular-nums hover:bg-[color:color-mix(in_srgb,var(--cp-text)_6%,transparent)]" onClick={() => actions.zoomTo(1)} title="回到 100%">
        {Math.round(zoom * 100)}%
      </button>
      <IconBtn icon={<Plus />} label="放大" onClick={() => actions.zoomBy(1.2)} />
      <IconBtn icon={<Maximize2 />} label="适应全部内容 (F)" onClick={() => actions.fitAll()} />
    </footer>
  )
}
