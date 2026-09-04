/* ── Table block: virtualized grid, cell editing, range selection, TSV copy/paste, AI cells ── */

import clsx from 'clsx'
import { Sparkles } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from 'react'
import { parseDelimited, toTsv } from '../../data/csv'
import { parseCellValue } from '../../domain/factories'
import { cellDisplay, colLetter } from '../../domain/selectors'
import type { CanvasBlockOf, TableCell } from '../../domain/types'
import { MAX_TABLE_COLS, MAX_TABLE_ROWS } from '../../domain/types'
import { useCanvasEditor, useStoreState } from '../../store/hooks'
import { useEditorActions } from '../actions'
import { trackEvent } from '../../events'
import { Btn, Input, Menu, type MenuItem } from '../primitives'

const ROW_H = 28
const HEAD_H = 28
const ROWNUM_W = 40

interface Range {
  rowStart: number
  rowEnd: number
  colStart: number
  colEnd: number
}

function norm(a: { r: number; c: number }, b: { r: number; c: number }): Range {
  return { rowStart: Math.min(a.r, b.r), rowEnd: Math.max(a.r, b.r), colStart: Math.min(a.c, b.c), colEnd: Math.max(a.c, b.c) }
}

export function TableBlockView({ block, selected }: { block: CanvasBlockOf<'table'>; selected: boolean }) {
  const { store, runner } = useCanvasEditor()
  const { ui } = useStoreState()
  const actions = useEditorActions()
  const c = block.content
  const scrollRef = useRef<HTMLDivElement>(null)
  const [scrollTop, setScrollTop] = useState(0)
  const [viewH, setViewH] = useState(200)
  const [anchor, setAnchor] = useState<{ r: number; c: number } | null>(null)
  const [cursor, setCursor] = useState<{ r: number; c: number } | null>(null)
  const [edit, setEditState] = useState<{ r: number; c: number; value: string } | null>(null)
  const editRef = useRef<{ r: number; c: number; value: string } | null>(null)
  const setEdit = useCallback((v: { r: number; c: number; value: string } | null) => {
    editRef.current = v
    setEditState(v)
  }, [])
  const [headerEdit, setHeaderEdit] = useState<{ c: number; value: string } | null>(null)
  const [menu, setMenu] = useState<{ x: number; y: number; r: number; c: number } | null>(null)
  const [aiPrompt, setAiPrompt] = useState<{ r: number; c: number; value: string } | null>(null)
  const dragging = useRef(false)

  const range = anchor && cursor ? norm(anchor, cursor) : null
  const isSel = (r: number, col: number) => Boolean(range && r >= range.rowStart && r <= range.rowEnd && col >= range.colStart && col <= range.colEnd)

  // publish selection to the store (right panel uses it)
  useEffect(() => {
    if (!selected) return
    store.setUi({ tableSelection: range ? { blockId: block.id, range } : null })
  }, [range?.rowStart, range?.rowEnd, range?.colStart, range?.colEnd, selected, block.id, store]) // eslint-disable-line react-hooks/exhaustive-deps

  const [prevSelected, setPrevSelected] = useState(selected)
  if (prevSelected !== selected) {
    setPrevSelected(selected)
    if (!selected) {
      setAnchor(null)
      setCursor(null)
      setEditState(null)
      setMenu(null)
    }
  }
  useEffect(() => {
    if (!selected) editRef.current = null
  }, [selected])

  useEffect(() => {
    const el = scrollRef.current
    if (!el) return
    const ro = new ResizeObserver(() => setViewH(el.clientHeight))
    ro.observe(el)
    setViewH(el.clientHeight)
    return () => ro.disconnect()
  }, [])

  const colOffsets = useMemo(() => {
    const out: number[] = [ROWNUM_W]
    for (const col of c.columns) out.push(out[out.length - 1] + (col.width ?? 120))
    return out
  }, [c.columns])
  const totalW = colOffsets[colOffsets.length - 1]
  const first = Math.max(0, Math.floor(scrollTop / ROW_H) - 4)
  const last = Math.min(c.rows.length, Math.ceil((scrollTop + viewH) / ROW_H) + 4)

  const writeCells = useCallback(
    (edits: Array<{ r: number; c: number; cell: TableCell }>) => {
      const list = edits.filter((e) => c.rows[e.r] && c.columns[e.c]).map((e) => ({ rowId: c.rows[e.r].id, columnId: c.columns[e.c].id, cell: e.cell }))
      if (list.length) store.dispatch({ type: 'UPDATE_TABLE_CELLS', id: block.id, edits: list })
      if (block.generated) trackEvent('result_edited', { type: 'table' })
    },
    [block.id, block.generated, c.rows, c.columns, store],
  )

  // Enter and blur can both fire for one edit; the ref guarantees a single commit.
  const commitEdit = () => {
    const e = editRef.current
    if (!e) return
    setEdit(null)
    const fresh = store.doc.blocks[block.id]
    if (!fresh || fresh.type !== 'table') return
    const existing = fresh.content.rows[e.r]?.cells[fresh.content.columns[e.c]?.id]
    if (e.value !== cellDisplay(existing)) writeCells([{ r: e.r, c: e.c, cell: parseCellValue(e.value) }])
  }

  const startEdit = (r: number, col: number, initial?: string) => {
    if (block.locked) return
    const cell = c.rows[r]?.cells[c.columns[col]?.id]
    setEdit({ r, c: col, value: initial ?? cellDisplay(cell) })
  }

  const move = (dr: number, dc: number, extend = false) => {
    const base = cursor ?? { r: 0, c: 0 }
    const next = { r: Math.max(0, Math.min(c.rows.length - 1, base.r + dr)), c: Math.max(0, Math.min(c.columns.length - 1, base.c + dc)) }
    setCursor(next)
    if (!extend) setAnchor(next)
    const el = scrollRef.current
    if (el) {
      const top = HEAD_H + next.r * ROW_H
      if (top < el.scrollTop + HEAD_H) el.scrollTop = top - HEAD_H
      else if (top + ROW_H > el.scrollTop + el.clientHeight) el.scrollTop = top + ROW_H - el.clientHeight
    }
  }

  const onKeyDown = (e: ReactKeyboardEvent) => {
    if ((e.ctrlKey || e.metaKey) && ['z', 'y', 'Z'].includes(e.key)) return // let editor handle undo/redo
    if (edit || headerEdit || aiPrompt) return
    if (!cursor) return
    if (e.key === 'ArrowDown') {
      e.preventDefault()
      move(1, 0, e.shiftKey)
    } else if (e.key === 'ArrowUp') {
      e.preventDefault()
      move(-1, 0, e.shiftKey)
    } else if (e.key === 'ArrowLeft') {
      e.preventDefault()
      move(0, -1, e.shiftKey)
    } else if (e.key === 'ArrowRight' || e.key === 'Tab') {
      e.preventDefault()
      move(0, e.shiftKey && e.key === 'Tab' ? -1 : 1, e.shiftKey && e.key !== 'Tab')
    } else if (e.key === 'Enter' || e.key === 'F2') {
      e.preventDefault()
      startEdit(cursor.r, cursor.c)
    } else if (e.key === 'Delete' || e.key === 'Backspace') {
      e.preventDefault()
      if (range) {
        const edits = []
        for (let r = range.rowStart; r <= range.rowEnd; r++) for (let col = range.colStart; col <= range.colEnd; col++) edits.push({ r, c: col, cell: parseCellValue(null) })
        writeCells(edits)
      }
    } else if ((e.ctrlKey || e.metaKey) && e.key === 'c') {
      e.preventDefault()
      copyRange()
    } else if ((e.ctrlKey || e.metaKey) && e.key === 'a') {
      e.preventDefault()
      setAnchor({ r: 0, c: 0 })
      setCursor({ r: c.rows.length - 1, c: c.columns.length - 1 })
    } else if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) {
      startEdit(cursor.r, cursor.c, e.key)
      e.preventDefault()
    } else if (e.key === 'Escape') {
      setAnchor(null)
      setCursor(null)
      return
    }
    e.stopPropagation()
  }

  const copyRange = () => {
    if (!range) return
    const matrix: string[][] = []
    for (let r = range.rowStart; r <= range.rowEnd; r++) {
      const row: string[] = []
      for (let col = range.colStart; col <= range.colEnd; col++) row.push(cellDisplay(c.rows[r]?.cells[c.columns[col]?.id]))
      matrix.push(row)
    }
    void navigator.clipboard?.writeText(toTsv(matrix))
    store.toast(`已复制 ${matrix.length} 行 × ${matrix[0]?.length ?? 0} 列`, 'success')
  }

  const pasteText = (text: string) => {
    if (!cursor || block.locked) return
    const matrix = parseDelimited(text, text.includes('\t') ? '\t' : undefined)
    if (!matrix.length) return
    const needRows = cursor.r + matrix.length - c.rows.length
    const needCols = cursor.c + Math.max(...matrix.map((r) => r.length)) - c.columns.length
    if (c.rows.length + Math.max(0, needRows) > MAX_TABLE_ROWS || c.columns.length + Math.max(0, needCols) > MAX_TABLE_COLS) {
      store.toast('粘贴内容超过表格上限', 'error')
      return
    }
    store.beginTransient()
    for (let i = 0; i < needRows; i++) store.dispatch({ type: 'TABLE_STRUCTURE', id: block.id, action: { kind: 'addRow' } })
    for (let i = 0; i < needCols; i++) store.dispatch({ type: 'TABLE_STRUCTURE', id: block.id, action: { kind: 'addColumn' } })
    const fresh = store.doc.blocks[block.id]
    if (fresh?.type === 'table') {
      const edits = matrix.flatMap((row, ri) => row.map((v, ci) => ({ rowId: fresh.content.rows[cursor.r + ri].id, columnId: fresh.content.columns[cursor.c + ci].id, cell: parseCellValue(v) })))
      store.dispatch({ type: 'UPDATE_TABLE_CELLS', id: block.id, edits })
    }
    store.endTransient()
    setAnchor(cursor)
    setCursor({ r: cursor.r + matrix.length - 1, c: cursor.c + Math.max(...matrix.map((r) => r.length)) - 1 })
  }

  const menuItems = (r: number, col: number): MenuItem[] => {
    const cell = c.rows[r]?.cells[c.columns[col]?.id]
    const key = `${c.rows[r]?.id}:${c.columns[col]?.id}`
    const wish = c.cellWishes?.[key]
    return [
      cell?.kind === 'ai'
        ? { label: '恢复为普通单元格', icon: <Sparkles />, onClick: () => {
            writeCells([{ r, c: col, cell: { kind: 'value', value: cell.value ?? cell.displayValue ?? null, valueType: cell.valueType ?? 'string', displayValue: cell.displayValue } }])
            store.dispatch({ type: 'TABLE_STRUCTURE', id: block.id, action: { kind: 'setCellWish', key, wish: null } })
          } }
        : { label: '转为 AI 单元格…', icon: <Sparkles />, onClick: () => setAiPrompt({ r, c: col, value: wish?.prompt ?? '' }) },
      ...(cell?.kind === 'ai' ? [{ label: '修改 AI 目标并重新计算…', onClick: () => setAiPrompt({ r, c: col, value: wish?.prompt ?? '' }) }] : []),
      { label: '', divider: true, onClick: () => undefined },
      { label: '复制选区', onClick: copyRange, disabled: !range },
      { label: '在下方插入行', onClick: () => store.dispatch({ type: 'TABLE_STRUCTURE', id: block.id, action: { kind: 'addRow', afterRowId: c.rows[r]?.id } }) },
      { label: '在右侧插入列', onClick: () => store.dispatch({ type: 'TABLE_STRUCTURE', id: block.id, action: { kind: 'addColumn', afterColumnId: c.columns[col]?.id } }) },
      { label: '', divider: true, onClick: () => undefined },
      { label: range ? `删除 ${range.rowEnd - range.rowStart + 1} 行` : '删除行', danger: true, onClick: () => store.dispatch({ type: 'TABLE_STRUCTURE', id: block.id, action: { kind: 'deleteRows', rowIds: (range ? c.rows.slice(range.rowStart, range.rowEnd + 1) : [c.rows[r]]).map((x) => x.id) } }) },
      { label: range ? `删除 ${range.colEnd - range.colStart + 1} 列` : '删除列', danger: true, onClick: () => store.dispatch({ type: 'TABLE_STRUCTURE', id: block.id, action: { kind: 'deleteColumns', columnIds: (range ? c.columns.slice(range.colStart, range.colEnd + 1) : [c.columns[col]]).map((x) => x.id) } }) },
    ]
  }

  const runAiCell = () => {
    if (!aiPrompt || !aiPrompt.value.trim()) return
    const row = c.rows[aiPrompt.r]
    const col = c.columns[aiPrompt.c]
    if (!row || !col) return
    const existing = row.cells[col.id]
    const go = () => {
      void runner.runCell(block.id, row.id, col.id, aiPrompt.value.trim())
      setAiPrompt(null)
    }
    if (existing && existing.kind === 'value' && existing.value !== null && existing.value !== '') {
      actions.confirm({
        title: '覆盖单元格内容？',
        body: `该单元格当前值为「${cellDisplay(existing)}」，AI 结果会覆盖它（可撤销）。`,
        actions: [{ label: '覆盖并运行', tone: 'primary', onClick: go }],
      })
      return
    }
    go()
  }

  const startColResize = (colIdx: number, e: React.PointerEvent) => {
    e.stopPropagation()
    e.preventDefault()
    const startX = e.clientX
    const startW = c.columns[colIdx].width ?? 120
    const zoom = (() => {
      const el = scrollRef.current?.closest('.aic-layer') as HTMLElement | null
      const m = el?.style.transform.match(/scale\(([\d.]+)\)/)
      return m ? Number(m[1]) : 1
    })()
    store.beginTransient()
    const onMove = (ev: PointerEvent) => {
      const w = Math.max(48, Math.round(startW + (ev.clientX - startX) / zoom))
      store.dispatch({ type: 'TABLE_STRUCTURE', id: block.id, action: { kind: 'setColumnWidth', columnId: c.columns[colIdx].id, width: w } })
    }
    const onUp = () => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
      store.endTransient()
    }
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
  }

  const src = c.source
  const sourceLabel = src?.kind === 'csv' || src?.kind === 'xlsx' || src?.kind === 'sample' ? `${src.filename ?? ''}${src.worksheet ? ` / ${src.worksheet}` : ''}` : src?.kind === 'paste' ? '粘贴' : '手动创建'

  return (
    <div className="flex h-full flex-col" data-no-drag data-own-menu>
      <div
        ref={scrollRef}
        tabIndex={0}
        className="aic-scroll relative min-h-0 flex-1 overflow-auto outline-none"
        onScroll={(e) => setScrollTop(e.currentTarget.scrollTop)}
        onKeyDown={onKeyDown}
        onPaste={(e) => {
          const text = e.clipboardData.getData('text/plain')
          if (text && cursor) {
            e.preventDefault()
            e.stopPropagation()
            pasteText(text)
          }
        }}
        onCopy={(e) => {
          if (range && !edit) {
            e.preventDefault()
            e.stopPropagation()
            copyRange()
          }
        }}
        onPointerDown={(e) => {
          if (!selected) store.select([block.id])
          e.stopPropagation()
        }}
        onContextMenu={(e) => {
          e.preventDefault()
          e.stopPropagation()
        }}
      >
        <div style={{ width: totalW, height: HEAD_H + c.rows.length * ROW_H, position: 'relative' }}>
          {/* header */}
          <div className="sticky top-0 z-[3] flex" style={{ height: HEAD_H }}>
            <div className="aic-cell is-rownum" style={{ width: ROWNUM_W, flex: 'none' }} />
            {c.columns.map((col, ci) => (
              <div
                key={col.id}
                className={clsx('aic-cell is-header', range && ci >= range.colStart && ci <= range.colEnd && 'is-sel')}
                style={{ width: col.width ?? 120, flex: 'none' }}
                title={`${col.name}${col.inferredType ? ` · ${col.inferredType}` : ''}`}
                onDoubleClick={() => !block.locked && setHeaderEdit({ c: ci, value: col.name })}
                onMouseDown={(e) => {
                  if (e.button !== 0) return
                  setAnchor({ r: 0, c: ci })
                  setCursor({ r: c.rows.length - 1, c: ci })
                }}
                onContextMenu={(e) => setMenu({ x: e.clientX, y: e.clientY, r: 0, c: ci })}
              >
                {headerEdit?.c === ci ? (
                  <input
                    autoFocus
                    className="h-full w-full bg-transparent outline-none"
                    value={headerEdit.value}
                    onChange={(e) => setHeaderEdit({ c: ci, value: e.target.value })}
                    onBlur={() => {
                      if (headerEdit.value.trim() && headerEdit.value !== col.name) store.dispatch({ type: 'TABLE_STRUCTURE', id: block.id, action: { kind: 'renameColumn', columnId: col.id, name: headerEdit.value.trim() } })
                      setHeaderEdit(null)
                    }}
                    onKeyDown={(e) => {
                      e.stopPropagation()
                      if (e.key === 'Enter') (e.currentTarget as HTMLInputElement).blur()
                      if (e.key === 'Escape') setHeaderEdit(null)
                    }}
                  />
                ) : (
                  <>
                    <span className="mr-1 text-[10px] font-normal text-[color:var(--cp-muted)]">{colLetter(ci)}</span>
                    {col.name}
                  </>
                )}
                <div className="aic-col-resize" onPointerDown={(e) => startColResize(ci, e)} />
              </div>
            ))}
          </div>
          {/* rows */}
          {c.rows.slice(first, last).map((row, i) => {
            const r = first + i
            return (
              <div key={row.id} className="absolute left-0 flex" style={{ top: HEAD_H + r * ROW_H, height: ROW_H }}>
                <div
                  className={clsx('aic-cell is-rownum', range && r >= range.rowStart && r <= range.rowEnd && 'is-sel')}
                  style={{ width: ROWNUM_W, flex: 'none' }}
                  onMouseDown={(e) => {
                    if (e.button !== 0) return
                    setAnchor({ r, c: 0 })
                    setCursor({ r, c: c.columns.length - 1 })
                  }}
                  onContextMenu={(e) => setMenu({ x: e.clientX, y: e.clientY, r, c: 0 })}
                >
                  {r + 1}
                </div>
                {c.columns.map((col, ci) => {
                  const cell = row.cells[col.id]
                  const editing = edit && edit.r === r && edit.c === ci
                  const isNum = cell?.kind === 'value' ? cell.valueType === 'number' : typeof cell?.value === 'number'
                  const wishKey = `${row.id}:${col.id}`
                  const cw = c.cellWishes?.[wishKey]
                  return (
                    <div
                      key={col.id}
                      className={clsx('aic-cell', isNum && 'is-num', isSel(r, ci) && 'is-sel', cursor?.r === r && cursor.c === ci && 'is-active', cell?.kind === 'ai' && 'is-ai')}
                      style={{ width: col.width ?? 120, flex: 'none' }}
                      title={cell?.kind === 'ai' ? `AI 单元格：${cw?.prompt ?? ''}` : cell?.kind === 'value' && cell.warning ? cell.warning : undefined}
                      onMouseDown={(e) => {
                        if (e.button !== 0) return
                        if (edit && (edit.r !== r || edit.c !== ci)) commitEdit()
                        dragging.current = true
                        if (e.shiftKey && anchor) setCursor({ r, c: ci })
                        else {
                          setAnchor({ r, c: ci })
                          setCursor({ r, c: ci })
                        }
                        scrollRef.current?.focus()
                      }}
                      onMouseEnter={(e) => {
                        if (dragging.current && e.buttons === 1) setCursor({ r, c: ci })
                      }}
                      onMouseUp={() => (dragging.current = false)}
                      onDoubleClick={() => (cell?.kind === 'ai' ? setAiPrompt({ r, c: ci, value: cw?.prompt ?? '' }) : startEdit(r, ci))}
                      onContextMenu={(e) => {
                        setAnchor((a) => (isSel(r, ci) ? a : { r, c: ci }))
                        setCursor((cu) => (isSel(r, ci) ? cu : { r, c: ci }))
                        setMenu({ x: e.clientX, y: e.clientY, r, c: ci })
                      }}
                    >
                      {editing ? (
                        <input
                          autoFocus
                          className="h-full w-full bg-transparent outline-none"
                          value={edit.value}
                          onChange={(e) => setEdit({ ...edit, value: e.target.value })}
                          onBlur={commitEdit}
                          onKeyDown={(e) => {
                            e.stopPropagation()
                            if (e.key === 'Enter' || e.key === 'Tab') {
                              e.preventDefault()
                              commitEdit()
                              move(e.key === 'Enter' ? 1 : 0, e.key === 'Tab' ? 1 : 0)
                              scrollRef.current?.focus()
                            }
                            if (e.key === 'Escape') {
                              setEdit(null)
                              scrollRef.current?.focus()
                            }
                          }}
                        />
                      ) : (
                        <>
                          {cell?.kind === 'ai' ? <Sparkles className={clsx('mr-1 inline size-[11px] text-[color:var(--aic-ai)]', cw?.state === 'running' && 'animate-pulse')} /> : null}
                          {cell?.kind === 'value' && cell.warning ? <span className="mr-1 text-[color:var(--cp-warning)]">⚠</span> : null}
                          {cw?.state === 'running' ? <span className="text-[color:var(--cp-muted)]">计算中…</span> : cw?.state === 'failed' ? <span className="text-[color:var(--cp-danger)]">失败</span> : cellDisplay(cell)}
                        </>
                      )}
                    </div>
                  )
                })}
              </div>
            )
          })}
        </div>
        {aiPrompt ? (
          <div className="absolute left-2 right-2 top-9 z-[6] rounded-lg border border-[color:var(--aic-ai)] bg-[color:var(--cp-surface-opaque)] p-2 shadow-[var(--cp-panel-shadow)]">
            <div className="mb-1 flex items-center gap-1 text-[11px] font-semibold text-[color:var(--aic-ai)]">
              <Sparkles className="size-[12px]" /> AI 单元格 · 第 {aiPrompt.r + 1} 行 / {c.columns[aiPrompt.c]?.name}
              <span className="ml-auto font-normal text-[color:var(--cp-muted)]">上下文：本行全部列</span>
            </div>
            <Input
              autoFocus
              placeholder="例如：计算本行毛利率 / 用一句话总结本行"
              value={aiPrompt.value}
              onChange={(e) => setAiPrompt({ ...aiPrompt, value: e.target.value })}
              onKeyDown={(e) => {
                e.stopPropagation()
                if (e.key === 'Enter') runAiCell()
                if (e.key === 'Escape') setAiPrompt(null)
              }}
            />
            <div className="mt-1.5 flex justify-end gap-1">
              <Btn variant="ghost" onClick={() => setAiPrompt(null)}>
                取消
              </Btn>
              <Btn variant="primary" onClick={runAiCell} disabled={!aiPrompt.value.trim()}>
                运行
              </Btn>
            </div>
          </div>
        ) : null}
      </div>
      <div className="flex h-[22px] flex-none items-center gap-3 border-t border-[color:var(--cp-border)] px-2 text-[10px] text-[color:var(--cp-muted)]">
        <span>
          {c.rows.length} 行 × {c.columns.length} 列
        </span>
        <span className="truncate">来源：{sourceLabel}</span>
        {src?.truncated ? <span className="text-[color:var(--cp-warning)]">已截断（原 {src.truncated.originalRows} 行）</span> : null}
        {block.dataRevision > 0 ? <span>已修改 {block.dataRevision} 次</span> : null}
        {range ? <span className="ml-auto">{`${colLetter(range.colStart)}${range.rowStart + 1}:${colLetter(range.colEnd)}${range.rowEnd + 1}`}</span> : null}
        {ui.tableSelection?.blockId === block.id && !range ? null : null}
      </div>
      {menu ? <MenuPortal at={menu} items={menuItems(menu.r, menu.c)} onClose={() => setMenu(null)} /> : null}
    </div>
  )
}

/** Context menu in screen coordinates (escapes the zoomed layer). */
function MenuPortal({ at, items, onClose }: { at: { x: number; y: number }; items: MenuItem[]; onClose: () => void }) {
  const [root] = useState(() => {
    const el = document.createElement('div')
    el.style.position = 'fixed'
    el.style.inset = '0'
    el.style.zIndex = '1000'
    el.style.pointerEvents = 'none'
    return el
  })
  useEffect(() => {
    document.body.appendChild(root)
    return () => {
      root.remove()
    }
  }, [root])
  return <MenuInPortal root={root} at={at} items={items} onClose={onClose} />
}

import { createPortal } from 'react-dom'
function MenuInPortal({ root, at, items, onClose }: { root: HTMLElement; at: { x: number; y: number }; items: MenuItem[]; onClose: () => void }) {
  return createPortal(
    <div style={{ position: 'absolute', left: at.x, top: at.y, pointerEvents: 'auto' }}>
      <Menu at={{ x: 0, y: 0 }} items={items} onClose={onClose} />
    </div>,
    root,
  )
}

