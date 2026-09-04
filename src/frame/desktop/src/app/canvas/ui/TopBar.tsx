/* ── Top bar: back, title, save status, undo/redo, run mode, play, feedback, more ── */

import { ArrowLeft, Bot, Camera, Download, Info, MessageSquareHeart, MoreHorizontal, Play, Redo2, Settings2, Undo2 } from 'lucide-react'
import { useState } from 'react'
import { useCanvasEditor, useStoreState } from '../store/hooks'
import { useEditorActions } from './actions'
import { Badge, Btn, IconBtn, MenuButton } from './primitives'

export function TopBar({ onBack }: { onBack: () => void }) {
  const { doc, saveStatus, saveError, past, future, settings } = useStoreState()
  const { store } = useCanvasEditor()
  const actions = useEditorActions()
  const [title, setTitle] = useState<string | null>(null)
  const path = doc.presentationPaths.find((p) => p.steps.length) ?? doc.presentationPaths[0]

  const save = saveStatus === 'saving' ? { label: '正在保存…', tone: 'neutral' as const } : saveStatus === 'error' ? { label: `保存失败：${saveError ?? ''}`, tone: 'danger' as const } : saveStatus === 'saved' ? { label: '已保存到本机', tone: 'success' as const } : { label: '本地文档', tone: 'neutral' as const }

  return (
    <header className="flex h-11 flex-none items-center gap-2 border-b border-[color:var(--cp-border)] bg-[color:var(--cp-surface-opaque)] px-2">
      <IconBtn icon={<ArrowLeft />} label="返回画布列表" onClick={onBack} />
      {title !== null ? (
        <input
          autoFocus
          className="aic-input !w-[260px] !py-1 font-display text-sm font-semibold"
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          onBlur={() => {
            if (title.trim() && title.trim() !== doc.title) store.dispatch({ type: 'SET_TITLE', title: title.trim() })
            setTitle(null)
          }}
          onKeyDown={(e) => {
            e.stopPropagation()
            if (e.key === 'Enter') (e.currentTarget as HTMLInputElement).blur()
            if (e.key === 'Escape') setTitle(null)
          }}
        />
      ) : (
        <button type="button" className="max-w-[320px] truncate rounded px-1.5 py-1 font-display text-sm font-semibold hover:bg-[color:color-mix(in_srgb,var(--cp-text)_6%,transparent)]" onClick={() => setTitle(doc.title)} title="点击重命名">
          {doc.title}
        </button>
      )}
      <Badge tone={save.tone} title={saveStatus === 'error' ? '请通过"更多 → 导出"备份' : undefined}>{save.label}</Badge>
      <span className="mx-1 h-5 w-px bg-[color:var(--cp-border)]" />
      <IconBtn icon={<Undo2 />} label="撤销 (Ctrl/⌘+Z)" disabled={!past.length} onClick={() => store.undo()} />
      <IconBtn icon={<Redo2 />} label="重做 (Ctrl/⌘+Shift+Z)" disabled={!future.length} onClick={() => store.redo()} />
      <span className="flex-1" />
      <Btn variant="ghost" icon={<Bot />} onClick={() => actions.openSettings()} title="切换 Mock / 真实 Agent 服务">
        {settings.adapter === 'http' ? 'HTTP Agent' : 'Mock Agent（离线）'}
        {settings.adapter === 'mock' && settings.mockDebugMode !== 'normal' ? <Badge tone="warning">调试:{settings.mockDebugMode}</Badge> : null}
      </Btn>
      <Btn variant="ghost" icon={<Play />} disabled={!path?.steps.length} onClick={() => path && actions.startPresentation(path.id)} title={path?.steps.length ? `播放「${path.name}」` : '先在左侧"讲述路径"中添加步骤'}>
        播放
      </Btn>
      <Btn variant="ghost" icon={<MessageSquareHeart />} onClick={() => actions.openFeedback()}>
        反馈
      </Btn>
      <MenuButton
        icon={<MoreHorizontal />}
        items={[
          { label: '导出 .aicanvas.json', icon: <Download />, onClick: () => actions.exportJson() },
          { label: '命名快照…', icon: <Camera />, onClick: () => actions.openSnapshots() },
          { label: '设置（Agent / 调试）', icon: <Settings2 />, onClick: () => actions.openSettings() },
          { label: '', divider: true, onClick: () => undefined },
          { label: '关于原型', icon: <Info />, onClick: () => actions.confirm({ title: 'BuckyOS AI Canvas 原型 v0.1', body: '这是一个可离线演示的 P0 原型：画布、表格、许愿格、Mock Agent、依赖刷新、讲述路径、本地保存与导入导出均可用。多人协作、在线分享、定时后台任务与自定义交互块属于后续版本，未在本原型中提供。', actions: [] }) },
        ]}
      />
    </header>
  )
}
