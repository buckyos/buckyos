/* ── Left sidebar: Sheets / Data / Presentation path tabs (PRD §8.3) ── */

import clsx from 'clsx'
import { ArrowDown, ArrowUp, Camera, Copy, FileSpreadsheet, Layers, Pencil, Play, Plus, Presentation, Sparkles, Trash2, Upload } from 'lucide-react'
import { useState } from 'react'
import { newId } from '../domain/ids'
import { tableBlocks } from '../domain/selectors'
import { useCanvasEditor, useStoreState } from '../store/hooks'
import { useEditorActions } from './actions'
import { Btn, EmptyState, IconBtn, Input, SectionTitle, Select } from './primitives'
import { trackEvent } from '../events'

export function LeftSidebar() {
  const { ui } = useStoreState()
  const { store } = useCanvasEditor()
  const tabs = [
    { id: 'sheets' as const, label: 'Sheet', icon: <Layers /> },
    { id: 'data' as const, label: '数据', icon: <FileSpreadsheet /> },
    { id: 'presentation' as const, label: '讲述路径', icon: <Presentation /> },
  ]
  return (
    <aside className="flex h-full w-[236px] flex-none flex-col border-r border-[color:var(--cp-border)] bg-[color:var(--cp-surface-opaque)]">
      <div className="flex border-b border-[color:var(--cp-border)]">
        {tabs.map((t) => (
          <button key={t.id} type="button" onClick={() => store.setUi({ sidebarTab: t.id })} className={clsx('flex flex-1 items-center justify-center gap-1 py-2 text-[11px] font-medium [&>svg]:size-[13px]', ui.sidebarTab === t.id ? 'border-b-2 border-[color:var(--cp-accent)] text-[color:var(--cp-accent)]' : 'text-[color:var(--cp-muted)] hover:text-[color:var(--cp-text)]')}>
            {t.icon}
            {t.label}
          </button>
        ))}
      </div>
      <div className="aic-scroll min-h-0 flex-1 overflow-y-auto p-3">
        {ui.sidebarTab === 'sheets' ? <SheetsTab /> : ui.sidebarTab === 'data' ? <DataTab /> : <PresentationTab />}
      </div>
    </aside>
  )
}

function SheetsTab() {
  const { doc } = useStoreState()
  const { store } = useCanvasEditor()
  const actions = useEditorActions()
  const [renaming, setRenaming] = useState<{ id: string; value: string } | null>(null)
  return (
    <div>
      <SectionTitle aside={<IconBtn icon={<Plus />} label="新建 Sheet" size={22} onClick={() => store.dispatch({ type: 'ADD_SHEET' })} />}>工作页</SectionTitle>
      <ul className="space-y-1">
        {doc.sheets.map((s, i) => {
          const active = s.id === doc.activeSheetId
          return (
            <li key={s.id} className={clsx('group flex items-center gap-1 rounded-md px-2 py-1.5 text-xs', active ? 'bg-[color:color-mix(in_srgb,var(--cp-accent)_14%,transparent)] font-semibold' : 'hover:bg-[color:color-mix(in_srgb,var(--cp-text)_6%,transparent)]')}>
              {renaming?.id === s.id ? (
                <Input
                  autoFocus
                  value={renaming.value}
                  onChange={(e) => setRenaming({ id: s.id, value: e.target.value })}
                  onBlur={() => {
                    if (renaming.value.trim()) store.dispatch({ type: 'RENAME_SHEET', id: s.id, name: renaming.value.trim() })
                    setRenaming(null)
                  }}
                  onKeyDown={(e) => {
                    e.stopPropagation()
                    if (e.key === 'Enter') (e.currentTarget as HTMLInputElement).blur()
                    if (e.key === 'Escape') setRenaming(null)
                  }}
                  className="!py-0.5"
                />
              ) : (
                <button type="button" className="min-w-0 flex-1 truncate text-left" onClick={() => store.dispatch({ type: 'SET_ACTIVE_SHEET', id: s.id })} onDoubleClick={() => setRenaming({ id: s.id, value: s.name })}>
                  {s.name} <span className="font-normal text-[color:var(--cp-muted)]">({s.blockIds.length})</span>
                </button>
              )}
              <span className="hidden items-center group-hover:flex">
                <IconBtn icon={<Pencil />} label="重命名" size={20} onClick={() => setRenaming({ id: s.id, value: s.name })} />
                <IconBtn icon={<ArrowUp />} label="上移" size={20} disabled={i === 0} onClick={() => store.dispatch({ type: 'MOVE_SHEET', id: s.id, direction: -1 })} />
                <IconBtn icon={<ArrowDown />} label="下移" size={20} disabled={i === doc.sheets.length - 1} onClick={() => store.dispatch({ type: 'MOVE_SHEET', id: s.id, direction: 1 })} />
                <IconBtn icon={<Copy />} label="复制" size={20} onClick={() => store.dispatch({ type: 'DUPLICATE_SHEET', id: s.id })} />
                <IconBtn
                  icon={<Trash2 />}
                  label="删除"
                  size={20}
                  disabled={doc.sheets.length <= 1}
                  onClick={() =>
                    actions.confirm({
                      title: `删除 ${s.name}？`,
                      body: `该 Sheet 上的 ${s.blockIds.length} 个块会一起删除（可撤销）。`,
                      actions: [{ label: '删除', tone: 'danger', onClick: () => store.dispatch({ type: 'DELETE_SHEET', id: s.id }) }],
                    })
                  }
                />
              </span>
            </li>
          )
        })}
      </ul>
      <p className="mt-3 text-[11px] leading-5 text-[color:var(--cp-muted)]">每个 Sheet 是一张可以自由平移缩放的无限画布，切换 Sheet 会记住各自的镜头位置。</p>
    </div>
  )
}

function DataTab() {
  const { doc } = useStoreState()
  const actions = useEditorActions()
  const tables = tableBlocks(doc)
  return (
    <div>
      <SectionTitle aside={<IconBtn icon={<Upload />} label="导入 CSV / XLSX" size={22} onClick={() => actions.openImport()} />}>数据表</SectionTitle>
      {tables.length === 0 ? (
        <EmptyState title="还没有数据" body="导入 Excel / CSV，或直接从 Excel 复制区域粘贴到画布。" action={<Btn variant="primary" icon={<Upload />} onClick={() => actions.openImport()}>导入文件</Btn>} />
      ) : (
        <ul className="space-y-1.5">
          {tables.map((t) => {
            const sheet = doc.sheets.find((s) => s.id === t.sheetId)
            const src = t.content.source
            return (
              <li key={t.id} className="rounded-md border border-[color:var(--cp-border)] p-2 text-xs">
                <button type="button" className="flex w-full items-center gap-1.5 text-left font-semibold hover:underline" onClick={() => actions.focusBlock(t.id)}>
                  <FileSpreadsheet className="size-[13px] text-[color:var(--cp-muted)]" />
                  <span className="truncate">{t.title ?? '表格'}</span>
                  {t.generated && !t.generated.detached ? <Sparkles className="size-[11px] text-[color:var(--aic-ai)]" /> : null}
                </button>
                <div className="mt-1 text-[11px] text-[color:var(--cp-muted)]">
                  {t.content.rows.length} 行 × {t.content.columns.length} 列 · {sheet?.name}
                  {src?.filename ? <div className="truncate">{src.filename}{src.worksheet ? ` / ${src.worksheet}` : ''}</div> : null}
                  {t.dataRevision > 0 ? <div>已修改 {t.dataRevision} 次</div> : null}
                </div>
                <div className="mt-1.5">
                  <Btn variant="subtle" icon={<Sparkles />} className="!py-[3px]" onClick={() => actions.createWishFromTable(t.id)}>
                    基于此数据创建许愿格
                  </Btn>
                </div>
              </li>
            )
          })}
        </ul>
      )}
      <p className="mt-3 text-[11px] leading-5 text-[color:var(--cp-muted)]">在表格中拖选区域后，右侧面板可以只以选区作为许愿格的数据来源。</p>
    </div>
  )
}

function PresentationTab() {
  const { doc, ui } = useStoreState()
  const { store } = useCanvasEditor()
  const actions = useEditorActions()
  const path = doc.presentationPaths.find((p) => p.id === ui.selectedPathId) ?? doc.presentationPaths[0]
  const [renaming, setRenaming] = useState(false)
  const [name, setName] = useState('')

  const createPath = () => {
    const id = newId('path')
    store.dispatch({ type: 'PRESENTATION_CREATE_PATH', name: `讲述路径 ${doc.presentationPaths.length + 1}`, id })
    store.setUi({ selectedPathId: id, selectedStepId: null })
    trackEvent('presentation_created')
  }
  const selectedBlocks = ui.selection.filter((id) => doc.blocks[id])

  return (
    <div>
      <SectionTitle aside={<IconBtn icon={<Plus />} label="新建讲述路径" size={22} onClick={createPath} />}>讲述路径</SectionTitle>
      {doc.presentationPaths.length === 0 ? (
        <EmptyState title="还没有讲述路径" body="把画布上的区域按顺序串起来，就能像 PPT 一样播放。" action={<Btn variant="primary" icon={<Plus />} onClick={createPath}>新建路径</Btn>} />
      ) : path ? (
        <>
          <div className="flex items-center gap-1">
            {renaming ? (
              <Input
                autoFocus
                value={name}
                onChange={(e) => setName(e.target.value)}
                onBlur={() => {
                  if (name.trim()) store.dispatch({ type: 'PRESENTATION_RENAME_PATH', pathId: path.id, name: name.trim() })
                  setRenaming(false)
                }}
                onKeyDown={(e) => {
                  e.stopPropagation()
                  if (e.key === 'Enter') (e.currentTarget as HTMLInputElement).blur()
                }}
              />
            ) : (
              <Select value={path.id} onChange={(e) => store.setUi({ selectedPathId: e.target.value, selectedStepId: null })}>
                {doc.presentationPaths.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.name}（{p.steps.length} 步）
                  </option>
                ))}
              </Select>
            )}
            <IconBtn icon={<Pencil />} label="重命名" size={24} onClick={() => { setName(path.name); setRenaming(true) }} />
            <IconBtn icon={<Trash2 />} label="删除路径" size={24} onClick={() => actions.confirm({ title: `删除「${path.name}」？`, body: '路径中的步骤会被移除，画布内容不受影响。', actions: [{ label: '删除', tone: 'danger', onClick: () => { store.dispatch({ type: 'PRESENTATION_DELETE_PATH', pathId: path.id }); store.setUi({ selectedPathId: null }) } }] })} />
          </div>
          <div className="mt-2 flex flex-wrap gap-1">
            <Btn variant="subtle" icon={<Camera />} className="!py-[3px]" onClick={() => actions.addStepFromViewport(path.id)}>
              从当前视口添加
            </Btn>
            <Btn variant="subtle" icon={<Plus />} className="!py-[3px]" disabled={!selectedBlocks.length} onClick={() => actions.addStepFromBlocks(path.id, selectedBlocks)} title={selectedBlocks.length ? '' : '先在画布上选中块或框架'}>
              选中块加入
            </Btn>
            <Btn variant="primary" icon={<Play />} className="!py-[3px]" disabled={!path.steps.length} onClick={() => actions.startPresentation(path.id)}>
              播放
            </Btn>
          </div>
          <ol className="mt-3 space-y-1">
            {path.steps.map((st, i) => {
              const active = ui.selectedStepId === st.id
              return (
                <li key={st.id} className={clsx('group flex items-center gap-1 rounded-md px-2 py-1.5 text-xs', active ? 'bg-[color:color-mix(in_srgb,var(--cp-accent)_14%,transparent)]' : 'hover:bg-[color:color-mix(in_srgb,var(--cp-text)_6%,transparent)]')}>
                  <span className="w-5 text-[color:var(--cp-muted)]">{i + 1}</span>
                  <button type="button" className="min-w-0 flex-1 truncate text-left" onClick={() => { store.setUi({ selectedStepId: st.id, selection: [] }); actions.goToStep(i) }}>
                    {st.title || `步骤 ${i + 1}`}
                    {st.targetBlockIds.length ? <span className="ml-1 text-[color:var(--cp-muted)]">· {st.targetBlockIds.length} 块</span> : null}
                  </button>
                  <span className="hidden items-center group-hover:flex">
                    <IconBtn icon={<ArrowUp />} label="上移" size={20} disabled={i === 0} onClick={() => store.dispatch({ type: 'PRESENTATION_MOVE_STEP', pathId: path.id, stepId: st.id, direction: -1 })} />
                    <IconBtn icon={<ArrowDown />} label="下移" size={20} disabled={i === path.steps.length - 1} onClick={() => store.dispatch({ type: 'PRESENTATION_MOVE_STEP', pathId: path.id, stepId: st.id, direction: 1 })} />
                    <IconBtn icon={<Trash2 />} label="删除步骤" size={20} onClick={() => store.dispatch({ type: 'PRESENTATION_REMOVE_STEP', pathId: path.id, stepId: st.id })} />
                  </span>
                </li>
              )
            })}
          </ol>
          {path.steps.length === 0 ? <p className="mt-2 text-[11px] leading-5 text-[color:var(--cp-muted)]">先把镜头移到想讲的位置，或者框选几个块，然后点"添加"。</p> : null}
        </>
      ) : null}
    </div>
  )
}
