/* ── Home: recent canvases, new / from Excel / sample / import JSON, feedback, prototype notes ── */

import { Clapperboard, FilePlus2, FileSpreadsheet, FolderInput, LayoutTemplate, MessageSquareHeart, Sparkles, Trash2 } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { createEmptyDocument } from '../domain/factories'
import type { CanvasDocument } from '../domain/types'
import { trackEvent } from '../events'
import { createAigcWorkflowCanvas } from '../fixtures/aigc-workflow'
import { createSampleCanvas } from '../fixtures/sample-canvas'
import { importDocument } from '../storage/export'
import type { CanvasListItem, CanvasStorageAdapter } from '../storage/indexeddb'
import { FeedbackDialog } from './dialogs'
import { Btn, IconBtn } from './primitives'
import { formatTime } from './meta'

export function HomePage({ storage, onOpen }: { storage: CanvasStorageAdapter; onOpen: (doc: CanvasDocument, opts?: { importFile?: File }) => void }) {
  const [items, setItems] = useState<CanvasListItem[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [feedback, setFeedback] = useState(false)
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null)
  const fileRef = useRef<HTMLInputElement>(null)
  const jsonRef = useRef<HTMLInputElement>(null)

  const refresh = () => storage.list().then(setItems).catch((e) => { setItems([]); setError(`无法读取本地画布列表：${e instanceof Error ? e.message : String(e)}`) })
  useEffect(() => {
    void refresh()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const create = async (doc: CanvasDocument, opts?: { importFile?: File }) => {
    try {
      await storage.save(doc)
    } catch (e) {
      setError(`保存失败：${e instanceof Error ? e.message : String(e)}。画布仍可编辑，请记得导出。`)
    }
    onOpen(doc, opts)
  }
  const open = async (id: string) => {
    const doc = await storage.load(id)
    if (doc) onOpen(doc)
    else setError('画布不存在或已被删除')
  }

  return (
    <div className="aic-root aic-scroll h-full w-full overflow-y-auto" style={{ background: 'var(--cp-bg)' }}>
      <div className="mx-auto max-w-[1040px] px-6 py-6">
        <div className="flex items-start justify-between gap-4">
          <div>
            <p className="text-[11px] font-semibold uppercase tracking-[0.18em] text-[color:var(--cp-accent)]">BuckyOS AI Canvas · 原型 v0.1</p>
            <h1 className="mt-1 font-display text-2xl font-semibold">把数据和想法放在一张画布上，在需要的位置直接写下目标</h1>
            <p className="mt-2 max-w-[640px] text-sm leading-6 text-[color:var(--cp-muted)]">像 Excel 一样组织数据，在任意位置放一个"许愿格"，用一句话说出你要的分析。结果会作为可编辑、可刷新、可讲述的对象留在画布上。所有内容默认保存在本机。</p>
          </div>
          <Btn variant="ghost" icon={<MessageSquareHeart />} onClick={() => setFeedback(true)}>原型说明与反馈</Btn>
        </div>

        <div className="mt-6 grid gap-3 sm:grid-cols-2">
          <StartCard icon={<Sparkles />} title="打开季度经营分析示例" body="预置销售表与许愿格，3 步体验完整流程。" primary onClick={() => { trackEvent('sample_opened', { template: 'sales' }); void create(createSampleCanvas()) }} />
          <StartCard icon={<Clapperboard />} title="打开 AI 短片工作流示例" body="角色设定 → 角色图 → 故事板 → 成片：像 ComfyUI 那样串起来的许愿格，改上游即可刷新下游。" primary onClick={() => { trackEvent('sample_opened', { template: 'aigc' }); void create(createAigcWorkflowCanvas()) }} />
        </div>
        <div className="mt-3 grid gap-3 sm:grid-cols-3">
          <StartCard icon={<FileSpreadsheet />} title="从 Excel / CSV 开始" body="导入你的表格，基于它创建许愿格。" onClick={() => fileRef.current?.click()} />
          <StartCard icon={<FilePlus2 />} title="新建空白画布" body="从一张空的无限画布开始。" onClick={() => { trackEvent('canvas_created'); void create(createEmptyDocument()) }} />
          <StartCard icon={<FolderInput />} title="导入 .aicanvas.json" body="恢复之前导出的画布文件。" onClick={() => jsonRef.current?.click()} />
        </div>
        <input ref={fileRef} type="file" accept=".csv,.tsv,.txt,.xlsx,.xlsm" className="hidden" onChange={(e) => { const f = e.target.files?.[0]; e.target.value = ''; if (!f) return; trackEvent('canvas_created', { from: 'file' }); void create(createEmptyDocument(f.name.replace(/\.[^.]+$/, '')), { importFile: f }) }} />
        <input ref={jsonRef} type="file" accept=".json" className="hidden" onChange={async (e) => { const f = e.target.files?.[0]; e.target.value = ''; if (!f) return; try { const { doc, warnings } = importDocument(await f.text(), { newId: true }); if (warnings.length) setError(`导入完成，有 ${warnings.length} 条提示：${warnings.slice(0, 3).join('；')}`); await create(doc) } catch (err) { setError(err instanceof Error ? err.message : String(err)) } }} />

        {error ? <div className="mt-4 rounded-md border border-[color:var(--cp-danger)] bg-[color:color-mix(in_srgb,var(--cp-danger)_8%,transparent)] px-3 py-2 text-xs text-[color:var(--cp-danger)]">{error}</div> : null}

        <div className="mt-8">
          <div className="mb-2 flex items-center justify-between">
            <h2 className="text-[11px] font-semibold uppercase tracking-[0.14em] text-[color:var(--cp-muted)]">最近打开的画布</h2>
            <span className="text-[11px] text-[color:var(--cp-muted)]">保存在本机浏览器（IndexedDB）</span>
          </div>
          {items === null ? <p className="text-xs text-[color:var(--cp-muted)]">读取中…</p> : items.length === 0 ? (
            <div className="rounded-xl border border-dashed border-[color:var(--cp-border-opaque)] px-6 py-8 text-center text-xs text-[color:var(--cp-muted)]">还没有画布。建议先打开示例，5 分钟内就能看到第一份分析结果。</div>
          ) : (
            <ul className="grid gap-2 sm:grid-cols-2 lg:grid-cols-3">
              {items.map((it) => (
                <li key={it.id} className="group flex items-start gap-3 rounded-xl border border-[color:var(--cp-border)] bg-[color:var(--cp-surface-opaque)] p-3 hover:border-[color:var(--cp-accent)]">
                  <button type="button" className="min-w-0 flex-1 text-left" onClick={() => void open(it.id)}>
                    <div className="flex items-center gap-1.5 text-sm font-semibold"><LayoutTemplate className="size-[14px] text-[color:var(--cp-muted)]" /><span className="truncate">{it.title}</span></div>
                    <div className="mt-1 text-[11px] text-[color:var(--cp-muted)]">{it.sheetCount} 个 Sheet · {it.blockCount} 个块 · {formatTime(it.updatedAt)}</div>
                  </button>
                  {confirmDelete === it.id ? (
                    <span className="flex flex-col gap-1">
                      <Btn variant="danger" className="!py-[2px]" onClick={async () => { await storage.delete(it.id); setConfirmDelete(null); void refresh() }}>确认删除</Btn>
                      <Btn variant="ghost" className="!py-[2px]" onClick={() => setConfirmDelete(null)}>取消</Btn>
                    </span>
                  ) : (
                    <IconBtn icon={<Trash2 />} label="删除画布" className="opacity-0 group-hover:opacity-100" onClick={() => setConfirmDelete(it.id)} />
                  )}
                </li>
              ))}
            </ul>
          )}
        </div>

        <div className="mt-8 grid gap-3 md:grid-cols-2">
          <div className="rounded-xl border border-[color:var(--cp-border)] bg-[color:var(--cp-surface-opaque)] p-4 text-xs leading-5">
            <p className="font-semibold">这个原型能做什么</p>
            <ul className="mt-1 list-disc pl-4 text-[color:var(--cp-muted)]">
              <li>无限画布 + 多 Sheet；文本、表格、图片、视频、许愿格、指标、图表、框架、结果组</li>
              <li>导入 CSV / XLSX、粘贴 Excel 区域或图片、拖入图片文件、直接编辑单元格、AI 单元格</li>
              <li>许愿格可以串联：上一阶段的结果组作为下一阶段的数据来源（AIGC 工作流示例）</li>
              <li>许愿格离线运行（Mock Agent），或接入 HTTP Agent 服务；结果以 CanvasPatch 原子写入、可整体撤销</li>
              <li>数据变化 → 结果标记"需要刷新"；手工修改保护；解除 AI 管理；运行历史</li>
              <li>讲述路径播放；本地自动保存、命名快照、.aicanvas.json 导入导出；反馈表</li>
            </ul>
          </div>
          <div className="rounded-xl border border-[color:var(--cp-border)] bg-[color:var(--cp-surface-opaque)] p-4 text-xs leading-5">
            <p className="font-semibold">未来能力（本原型未提供）</p>
            <ul className="mt-1 list-disc pl-4 text-[color:var(--cp-muted)]">
              <li>BuckyOS DID 身份、群组权限、多人实时协作与在线分享</li>
              <li>后台定时任务（页面关闭后继续运行）、网络数据连接器</li>
              <li>自定义 HTML 交互块沙箱、评论、模板市场、移动端编辑</li>
              <li>Excel 公式引擎 / VBA、PowerPoint 像素级导出</li>
            </ul>
          </div>
        </div>
      </div>
      <FeedbackDialog open={feedback} onClose={() => setFeedback(false)} />
    </div>
  )
}

function StartCard({ icon, title, body, onClick, primary }: { icon: React.ReactNode; title: string; body: string; onClick: () => void; primary?: boolean }) {
  return (
    <button type="button" onClick={onClick} className={`flex flex-col items-start gap-2 rounded-xl border p-4 text-left transition hover:-translate-y-[1px] hover:shadow-[var(--cp-panel-shadow)] ${primary ? 'border-[color:var(--cp-accent)] bg-[color:color-mix(in_srgb,var(--cp-accent)_10%,var(--cp-surface-opaque))]' : 'border-[color:var(--cp-border)] bg-[color:var(--cp-surface-opaque)]'}`}>
      <span className={`inline-flex h-8 w-8 items-center justify-center rounded-lg [&>svg]:size-[16px] ${primary ? 'bg-[color:var(--cp-accent)] text-white' : 'bg-[color:var(--cp-surface-2-opaque)] text-[color:var(--cp-accent)]'}`}>{icon}</span>
      <span className="text-sm font-semibold">{title}</span>
      <span className="text-[11px] leading-5 text-[color:var(--cp-muted)]">{body}</span>
    </button>
  )
}
