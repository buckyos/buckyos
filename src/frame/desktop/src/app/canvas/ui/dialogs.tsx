/* ── Dialogs: confirm, import, feedback, settings, snapshots ── */

import { AlertTriangle, CheckCircle2, Download, FileSpreadsheet, Trash2, Upload } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { fileKind, listSheets, parseCsv, readSheet } from '../data/parse-file'
import { looksLikeHeader, tableContentFromMatrix } from '../domain/factories'
import { nowIso, newId } from '../domain/ids'
import type { CanvasSnapshot, TableBlockContent } from '../domain/types'
import { MAX_IMPORT_BYTES, MAX_TABLE_COLS, MAX_TABLE_ROWS } from '../domain/types'
import { allEvents, eventCounts, trackEvent } from '../events'
import { downloadText } from '../storage/export'
import { saveFeedback, type CanvasStorageAdapter } from '../storage/indexeddb'
import { useCanvasEditor, useStoreState } from '../store/hooks'
import type { ConfirmAction } from './actions'
import { Badge, Btn, Field, Input, Modal, Select, TextArea } from './primitives'
import { formatTime } from './meta'

/* ── confirm ── */
export interface ConfirmState {
  title: string
  body: string
  actions: ConfirmAction[]
}

export function ConfirmDialog({ state, onClose }: { state: ConfirmState | null; onClose: () => void }) {
  return (
    <Modal
      open={Boolean(state)}
      title={state?.title ?? ''}
      onClose={onClose}
      width={460}
      footer={
        <>
          <Btn variant="ghost" onClick={onClose}>
            {state?.actions.length ? '取消' : '关闭'}
          </Btn>
          {state?.actions.map((a) => (
            <Btn
              key={a.label}
              variant={a.tone === 'danger' ? 'danger' : a.tone === 'subtle' ? 'subtle' : 'primary'}
              onClick={() => {
                onClose()
                a.onClick()
              }}
            >
              {a.label}
            </Btn>
          ))}
        </>
      }
    >
      <p className="whitespace-pre-line text-sm leading-6">{state?.body}</p>
    </Modal>
  )
}

/* ── import ── */
type Matrix = Array<Array<string | number | boolean | null>>

interface ImportState {
  phase: 'pick' | 'parsing' | 'sheets' | 'preview'
  file?: File
  buffer?: ArrayBuffer
  sheets?: Array<{ name: string; path: string }>
  worksheet?: string
  matrix?: Matrix
  hasHeader?: boolean
  warnings: string[]
  error?: string
  truncated?: { originalRows: number }
  keepFirst?: boolean
}

export function ImportDialog(props: { open: boolean; onClose: () => void; initialFile?: File | null; onImport: (content: TableBlockContent, title: string) => void }) {
  if (!props.open) return null
  return <ImportDialogInner {...props} />
}

function ImportDialogInner({ open, onClose, initialFile, onImport }: { open: boolean; onClose: () => void; initialFile?: File | null; onImport: (content: TableBlockContent, title: string) => void }) {
  const [st, setSt] = useState<ImportState>({ phase: 'pick', warnings: [] })
  const inputRef = useRef<HTMLInputElement>(null)
  const [dragOver, setDragOver] = useState(false)
  const startedWith = useRef<File | null>(null)

  async function pickFile(file: File) {
    const kind = fileKind(file.name)
    if (kind !== 'csv' && kind !== 'xlsx') {
      setSt({ phase: 'pick', warnings: [], error: '当前仅支持 CSV 和 XLSX 文件。' })
      return
    }
    if (file.size > MAX_IMPORT_BYTES) {
      setSt({ phase: 'pick', warnings: [], error: `文件超过 ${MAX_IMPORT_BYTES / 1024 / 1024}MB 上限（${(file.size / 1024 / 1024).toFixed(1)}MB）。请先在 Excel 中拆分。` })
      return
    }
    setSt({ phase: 'parsing', file, warnings: [] })
    try {
      const buffer = await file.arrayBuffer()
      if (kind === 'csv') {
        const parsed = await parseCsv(buffer)
        applyMatrix({ file, buffer, warnings: parsed.encodingWarning ? [parsed.encodingWarning] : [] }, parsed.matrix)
      } else {
        const wb = await listSheets(buffer)
        if (wb.sheets.length === 1) await pickSheet({ file, buffer, sheets: wb.sheets, warnings: [] }, wb.sheets[0])
        else setSt({ phase: 'sheets', file, buffer, sheets: wb.sheets, warnings: [] })
      }
    } catch (e) {
      setSt({ phase: 'pick', warnings: [], error: `无法读取文件：${e instanceof Error ? e.message : String(e)}。请重新选择文件。` })
    }
  }

  async function pickSheet(base: Partial<ImportState> & { warnings: string[] }, sheet: { name: string; path: string }) {
    setSt({ ...base, phase: 'parsing', worksheet: sheet.name, warnings: base.warnings })
    try {
      const res = await readSheet(base.buffer!, sheet.path, MAX_TABLE_ROWS + 1)
      const warnings = [...base.warnings]
      if (res.formulaCellsWithoutCache > 0) warnings.push(`${res.formulaCellsWithoutCache} 个公式单元格没有缓存值，已显示为空（原型不计算公式）`)
      applyMatrix({ ...base, worksheet: sheet.name, warnings }, res.matrix)
    } catch (e) {
      setSt({ phase: 'sheets', file: base.file, buffer: base.buffer, sheets: base.sheets, warnings: [], error: `读取工作表失败：${e instanceof Error ? e.message : String(e)}` })
    }
  }

  function applyMatrix(base: Partial<ImportState> & { warnings: string[] }, matrixIn: Matrix) {
    let matrix = matrixIn
    const warnings = [...base.warnings]
    const width = Math.max(0, ...matrix.map((r) => r.length))
    if (width > MAX_TABLE_COLS) {
      matrix = matrix.map((r) => r.slice(0, MAX_TABLE_COLS))
      warnings.push(`列数 ${width} 超过 ${MAX_TABLE_COLS}，仅保留前 ${MAX_TABLE_COLS} 列`)
    }
    if (matrix.length === 0) {
      setSt({ phase: 'pick', warnings: [], error: '文件中没有可导入的数据。' })
      return
    }
    const strMatrix = matrix.slice(0, 50).map((r) => r.map((v) => (v == null ? '' : String(v))))
    const truncated = matrix.length - 1 > MAX_TABLE_ROWS ? { originalRows: matrix.length - 1 } : undefined
    setSt({ ...base, phase: 'preview', matrix, hasHeader: looksLikeHeader(strMatrix), warnings, truncated, keepFirst: true })
  }

  useEffect(() => {
    if (initialFile && startedWith.current !== initialFile) {
      startedWith.current = initialFile
      void pickFile(initialFile)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [initialFile])

  function doImport() {
    if (!st.matrix || !st.file) return
    let matrix = st.matrix
    let truncatedInfo: { originalRows: number; keptRows: number } | undefined
    if (st.truncated) {
      if (!st.keepFirst) return
      matrix = matrix.slice(0, MAX_TABLE_ROWS + (st.hasHeader ? 1 : 0))
      truncatedInfo = { originalRows: st.truncated.originalRows, keptRows: MAX_TABLE_ROWS }
    }
    const kind = fileKind(st.file.name) === 'xlsx' ? 'xlsx' : 'csv'
    const content = tableContentFromMatrix(matrix, { hasHeader: st.hasHeader ?? true, source: { kind, filename: st.file.name, worksheet: st.worksheet, importedAt: nowIso(), truncated: truncatedInfo } })
    trackEvent('file_imported', { kind, rows: content.rows.length, cols: content.columns.length })
    onImport(content, st.worksheet ? `${st.file.name.replace(/\.[^.]+$/, '')} / ${st.worksheet}` : st.file.name.replace(/\.[^.]+$/, ''))
    onClose()
  }

  const preview = st.matrix?.slice(0, 12) ?? []
  const rowCount = st.matrix ? st.matrix.length - (st.hasHeader ? 1 : 0) : 0
  const colCount = st.matrix ? Math.max(0, ...st.matrix.map((r) => r.length)) : 0

  return (
    <Modal open={open} title="导入 Excel / CSV" onClose={onClose} width={720} footer={st.phase === 'preview' ? (<><Btn variant="ghost" onClick={() => setSt({ phase: 'pick', warnings: [] })}>重新选择</Btn><Btn variant="primary" icon={<Upload />} onClick={doImport} disabled={Boolean(st.truncated) && !st.keepFirst}>导入到画布</Btn></>) : undefined}>
      {st.phase === 'pick' ? (
        <div>
          <div
            className={`flex flex-col items-center justify-center gap-2 rounded-xl border-2 border-dashed px-6 py-10 text-center ${dragOver ? 'border-[color:var(--cp-accent)] bg-[color:color-mix(in_srgb,var(--cp-accent)_8%,transparent)]' : 'border-[color:var(--cp-border-opaque)]'}`}
            onDragOver={(e) => { e.preventDefault(); setDragOver(true) }}
            onDragLeave={() => setDragOver(false)}
            onDrop={(e) => { e.preventDefault(); setDragOver(false); const f = e.dataTransfer.files[0]; if (f) void pickFile(f) }}
          >
            <FileSpreadsheet className="size-8 text-[color:var(--cp-muted)]" />
            <p className="text-sm font-semibold">拖入 .xlsx / .csv 文件，或</p>
            <Btn variant="primary" onClick={() => inputRef.current?.click()}>选择文件</Btn>
            <input ref={inputRef} type="file" accept=".csv,.tsv,.txt,.xlsx,.xlsm" className="hidden" onChange={(e) => { const f = e.target.files?.[0]; if (f) void pickFile(f); e.target.value = '' }} />
            <p className="text-[11px] text-[color:var(--cp-muted)]">单文件 ≤ 20MB；每表最多 {MAX_TABLE_ROWS.toLocaleString()} 行 × {MAX_TABLE_COLS} 列。XLSX 只读取单元格当前值，不执行公式与宏。</p>
          </div>
          {st.error ? <div className="mt-3 flex items-start gap-2 rounded-md bg-[color:color-mix(in_srgb,var(--cp-danger)_10%,transparent)] p-2 text-xs text-[color:var(--cp-danger)]"><AlertTriangle className="mt-[1px] size-[14px] shrink-0" />{st.error}</div> : null}
          <p className="mt-3 text-[11px] leading-5 text-[color:var(--cp-muted)]">提示：也可以在 Excel 中复制一个区域，回到画布后直接 Ctrl/⌘+V 粘贴成表格。</p>
        </div>
      ) : null}
      {st.phase === 'parsing' ? (
        <div className="py-8 text-center text-sm">
          <div className="aic-progress is-indeterminate mx-auto mb-3 w-[240px]"><i /></div>
          正在后台解析 {st.file?.name}…
        </div>
      ) : null}
      {st.phase === 'sheets' ? (
        <div>
          <p className="mb-2 text-sm">工作簿 <b>{st.file?.name}</b> 包含 {st.sheets?.length} 个工作表，请选择要导入的一个：</p>
          <ul className="space-y-1">
            {st.sheets?.map((s) => (
              <li key={s.path}>
                <button type="button" className="flex w-full items-center gap-2 rounded-md border border-[color:var(--cp-border)] px-3 py-2 text-left text-sm hover:bg-[color:color-mix(in_srgb,var(--cp-accent)_8%,transparent)]" onClick={() => void pickSheet({ file: st.file, buffer: st.buffer, sheets: st.sheets, warnings: [] }, s)}>
                  <FileSpreadsheet className="size-[14px] text-[color:var(--cp-muted)]" /> {s.name}
                </button>
              </li>
            ))}
          </ul>
          {st.error ? <p className="mt-2 text-xs text-[color:var(--cp-danger)]">{st.error}</p> : null}
        </div>
      ) : null}
      {st.phase === 'preview' && st.matrix ? (
        <div className="space-y-3">
          <div className="flex flex-wrap items-center gap-2 text-xs">
            <Badge tone="accent">{st.file?.name}{st.worksheet ? ` / ${st.worksheet}` : ''}</Badge>
            <span>{rowCount.toLocaleString()} 行 × {colCount} 列</span>
            <label className="ml-auto flex items-center gap-1"><input type="checkbox" checked={st.hasHeader ?? true} onChange={(e) => setSt({ ...st, hasHeader: e.target.checked })} /> 第一行是表头</label>
          </div>
          {st.warnings.map((w) => <div key={w} className="flex items-start gap-2 rounded-md bg-[color:color-mix(in_srgb,var(--cp-warning)_16%,transparent)] p-2 text-xs"><AlertTriangle className="mt-[1px] size-[14px] shrink-0" />{w}</div>)}
          {st.truncated ? (
            <div className="rounded-md border border-[color:var(--cp-warning)] p-2 text-xs">
              <p className="font-semibold">数据超过 {MAX_TABLE_ROWS.toLocaleString()} 行（共 {st.truncated.originalRows.toLocaleString()} 行）</p>
              <label className="mt-1 flex items-center gap-1"><input type="radio" checked={st.keepFirst === true} onChange={() => setSt({ ...st, keepFirst: true })} /> 只导入前 {MAX_TABLE_ROWS.toLocaleString()} 行</label>
              <label className="mt-1 flex items-center gap-1"><input type="radio" checked={st.keepFirst === false} onChange={() => setSt({ ...st, keepFirst: false })} /> 取消导入</label>
            </div>
          ) : null}
          <div className="aic-scroll max-h-[300px] overflow-auto rounded-md border border-[color:var(--cp-border)]">
            <table className="min-w-full border-collapse text-[11px]">
              <tbody>
                {preview.map((r, i) => (
                  <tr key={i} className={i === 0 && st.hasHeader ? 'bg-[color:var(--cp-surface-2-opaque)] font-semibold' : ''}>
                    <td className="border-b border-r border-[color:var(--cp-border)] px-2 py-1 text-[color:var(--cp-muted)]">{i + 1}</td>
                    {Array.from({ length: Math.min(colCount, 20) }, (_, c) => <td key={c} className="max-w-[160px] truncate border-b border-r border-[color:var(--cp-border)] px-2 py-1">{r[c] == null ? '' : String(r[c])}</td>)}
                  </tr>
                ))}
              </tbody>
            </table>
            {st.matrix.length > 12 ? <div className="px-2 py-1 text-[11px] text-[color:var(--cp-muted)]">仅预览前 12 行</div> : null}
          </div>
        </div>
      ) : null}
    </Modal>
  )
}

/* ── feedback ── */
const QUESTIONS: Array<{ id: string; text: string; kind: 'text' | 'scale' }> = [
  { id: 'q1', text: '用你自己的话，这个产品是做什么的？', kind: 'text' },
  { id: 'q2', text: '你最想用它解决哪一项真实工作？', kind: 'text' },
  { id: 'q3', text: '"许愿格"是否容易理解？', kind: 'scale' },
  { id: 'q4', text: '你更愿意把它当成 Excel、Notion、PPT、白板还是其他？为什么？', kind: 'text' },
  { id: 'q5', text: '哪一部分让你最不放心？', kind: 'text' },
  { id: 'q6', text: '哪些自动更新可以让 AI 直接做，哪些必须先确认？', kind: 'text' },
  { id: 'q7', text: '体验后，你是否愿意把某项长期工作放在这张画布上？', kind: 'text' },
]

export function FeedbackDialog(props: { open: boolean; onClose: () => void; canvasId?: string }) {
  if (!props.open) return null
  return <FeedbackDialogInner {...props} />
}

function FeedbackDialogInner({ open, onClose, canvasId }: { open: boolean; onClose: () => void; canvasId?: string }) {
  const [answers, setAnswers] = useState<Record<string, string>>({})
  const [saved, setSaved] = useState<{ id: string; createdAt: string } | null>(null)
  const submit = async () => {
    const record = { id: newId('fb'), createdAt: nowIso(), canvasId, answers, events: eventCounts() }
    try {
      await saveFeedback(record)
    } catch {
      /* IndexedDB unavailable – still allow download */
    }
    trackEvent('feedback_submitted')
    setSaved({ id: record.id, createdAt: record.createdAt })
  }
  const download = () => {
    const payload = { id: saved?.id, createdAt: saved?.createdAt, answers, eventCounts: eventCounts(), events: allEvents().map((e) => ({ name: e.name, at: e.at })) }
    downloadText(`aicanvas-feedback-${(saved?.createdAt ?? nowIso()).slice(0, 10)}.json`, JSON.stringify(payload, null, 2))
  }
  return (
    <Modal open={open} title="首轮体验反馈" onClose={onClose} width={620} footer={saved ? (<><Btn variant="subtle" icon={<Download />} onClick={download}>下载匿名反馈 JSON</Btn><Btn variant="primary" onClick={onClose}>完成</Btn></>) : (<><Btn variant="ghost" onClick={onClose}>稍后再说</Btn><Btn variant="primary" onClick={submit} disabled={Object.values(answers).filter((v) => v.trim()).length < 2}>提交（保存在本机）</Btn></>)}>
      {saved ? (
        <div className="flex flex-col items-center gap-2 py-6 text-center">
          <CheckCircle2 className="size-8 text-[color:var(--cp-success)]" />
          <p className="text-sm font-semibold">谢谢！反馈已保存在本机。</p>
          <p className="text-xs text-[color:var(--cp-muted)]">导出的 JSON 只包含你填写的答案与行为事件计数，不包含画布内容、Prompt 全文或导入的数据。</p>
        </div>
      ) : (
        <div className="space-y-4">
          {QUESTIONS.map((q, i) => (
            <Field key={q.id} label={`${i + 1}. ${q.text}`}>
              {q.kind === 'scale' ? (
                <div className="flex gap-1">
                  {[1, 2, 3, 4, 5].map((n) => (
                    <Btn key={n} variant="subtle" active={answers[q.id] === String(n)} onClick={() => setAnswers({ ...answers, [q.id]: String(n) })}>{n}</Btn>
                  ))}
                  <span className="self-center text-[11px] text-[color:var(--cp-muted)]">1 = 很难理解，5 = 一看就懂</span>
                </div>
              ) : (
                <TextArea rows={2} value={answers[q.id] ?? ''} onChange={(e) => setAnswers({ ...answers, [q.id]: e.target.value })} />
              )}
            </Field>
          ))}
        </div>
      )}
    </Modal>
  )
}

/* ── settings ── */
export function SettingsDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const { settings } = useStoreState()
  const { store, runner } = useCanvasEditor()
  const [health, setHealth] = useState<string | null>(null)
  const check = async () => {
    setHealth('检查中…')
    const h = await runner.http.health()
    setHealth(h.available ? `✓ ${h.message ?? '可用'}` : `✕ ${h.message ?? '不可用'}`)
  }
  return (
    <Modal open={open} title="设置" onClose={onClose} width={520} footer={<Btn variant="primary" onClick={onClose}>完成</Btn>}>
      <div className="space-y-4">
        <Field label="Agent 运行模式">
          <Select value={settings.adapter} onChange={(e) => store.updateSettings({ adapter: e.target.value as 'mock' | 'http' })}>
            <option value="mock">Mock Agent（离线，确定性输出）</option>
            <option value="http">HTTP Agent（BuckyOS Agent 服务）</option>
          </Select>
        </Field>
        {settings.adapter === 'http' ? (
          <>
            <Field label="服务地址" hint="示例 http://127.0.0.1:8080 。接口：POST /api/agent/jobs、GET /jobs/:id/events (SSE)、GET /jobs/:id/result。密钥不保存在文档中。">
              <div className="flex gap-1">
                <Input value={settings.httpBaseUrl} placeholder="http://127.0.0.1:8080" onChange={(e) => store.updateSettings({ httpBaseUrl: e.target.value })} />
                <Btn variant="subtle" onClick={check}>检查</Btn>
              </div>
              {health ? <p className="mt-1 text-[11px]">{health}</p> : null}
            </Field>
            <p className="rounded-md bg-[color:color-mix(in_srgb,var(--cp-warning)_16%,transparent)] p-2 text-[11px] leading-5">使用真实 Agent 时，只有你在许愿格中显式选择的数据来源会被发送到该服务；画布其他内容、评论与历史不会发送。</p>
          </>
        ) : (
          <Field label="Mock 调试模式" hint="用于演示失败、无效补丁与取消。也可在 Prompt 中加 #fail / #invalid / #slow / #timeout。">
            <Select value={settings.mockDebugMode} onChange={(e) => store.updateSettings({ mockDebugMode: e.target.value as typeof settings.mockDebugMode })}>
              <option value="normal">正常</option>
              <option value="slow">慢速（便于演示取消）</option>
              <option value="fail">固定失败</option>
              <option value="invalid_patch">返回无效补丁（校验拒绝）</option>
              <option value="timeout">永不返回（触发超时）</option>
            </Select>
          </Field>
        )}
        <Field label="运行超时（秒）">
          <Input type="number" value={Math.round(settings.timeoutMs / 1000)} onChange={(e) => store.updateSettings({ timeoutMs: Math.max(5, Number(e.target.value)) * 1000 })} />
        </Field>
        <label className="flex items-center gap-2 text-xs">
          <input type="checkbox" checked={settings.reducedMotion} onChange={(e) => store.updateSettings({ reducedMotion: e.target.checked })} /> 减少动画（讲述播放时直接跳转）
        </label>
      </div>
    </Modal>
  )
}

/* ── snapshots ── */
export function SnapshotsDialog(props: { open: boolean; onClose: () => void; storage: CanvasStorageAdapter }) {
  if (!props.open) return null
  return <SnapshotsDialogInner {...props} />
}

function SnapshotsDialogInner({ open, onClose, storage }: { open: boolean; onClose: () => void; storage: CanvasStorageAdapter }) {
  const { doc } = useStoreState()
  const { store } = useCanvasEditor()
  const [list, setList] = useState<CanvasSnapshot[]>([])
  const [name, setName] = useState(() => `快照 ${new Date().toLocaleString('zh-CN')}`)
  const [preview, setPreview] = useState<CanvasSnapshot | null>(null)
  const refresh = () => storage.listSnapshots(doc.id).then(setList).catch(() => setList([]))
  useEffect(() => {
    void storage.listSnapshots(doc.id).then(setList).catch(() => setList([]))
  }, [storage, doc.id])
  return (
    <Modal open={open} title="命名快照" onClose={onClose} width={560} footer={<Btn variant="primary" onClick={onClose}>关闭</Btn>}>
      <div className="flex gap-1">
        <Input value={name} onChange={(e) => setName(e.target.value)} />
        <Btn variant="primary" onClick={async () => { await storage.createSnapshot(store.doc, name.trim() || '未命名快照'); store.toast('快照已创建', 'success'); void refresh() }}>创建</Btn>
      </div>
      <ul className="mt-3 space-y-1">
        {list.length === 0 ? <li className="text-xs text-[color:var(--cp-muted)]">还没有快照。快照会保存当前文档的完整状态，恢复操作本身可以撤销。</li> : null}
        {list.map((s) => (
          <li key={s.id} className="rounded-md border border-[color:var(--cp-border)] p-2 text-xs">
            <div className="flex items-center gap-2">
              <span className="font-semibold">{s.name}</span>
              <span className="text-[color:var(--cp-muted)]">{formatTime(s.createdAt)} · 修订 {s.revision}</span>
              <span className="ml-auto flex gap-1">
                <Btn variant="subtle" className="!py-[2px]" onClick={() => setPreview(preview?.id === s.id ? null : s)}>预览</Btn>
                <Btn variant="primary" className="!py-[2px]" onClick={() => { store.dispatch({ type: 'RESTORE_SNAPSHOT', doc: s.doc }); store.toast('已恢复快照（可撤销）', 'success'); onClose() }}>恢复</Btn>
                <Btn variant="danger" className="!py-[2px]" icon={<Trash2 />} onClick={async () => { await storage.deleteSnapshot(s.id); void refresh() }} />
              </span>
            </div>
            {preview?.id === s.id ? (
              <div className="mt-2 grid grid-cols-3 gap-2 text-[11px] text-[color:var(--cp-muted)]">
                <span>标题：{s.doc.title}</span><span>Sheet：{s.doc.sheets.length}</span><span>块：{Object.keys(s.doc.blocks).length}</span>
                <span>许愿格：{Object.values(s.doc.blocks).filter((b) => b.type === 'wish').length}</span><span>讲述路径：{s.doc.presentationPaths.length}</span><span>绑定：{s.doc.bindings.length}</span>
              </div>
            ) : null}
          </li>
        ))}
      </ul>
    </Modal>
  )
}
