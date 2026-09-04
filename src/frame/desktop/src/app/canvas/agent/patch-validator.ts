/* ── CanvasPatch validation (PRD §13.5). Nothing is applied when this fails. ── */

import type { CanvasDocument, CanvasBlock } from '../domain/types'
import { MAX_IMAGE_BYTES, MIN_BLOCK_HEIGHT, MIN_BLOCK_WIDTH } from '../domain/types'
import { wouldCreateCycle } from '../domain/selectors'
import type { CanvasPatch } from './contracts'

export interface PatchValidation {
  ok: boolean
  errors: string[]
  conflict: boolean
}

const BLOCK_TYPES = new Set(['text', 'table', 'wish', 'metric', 'chart', 'frame', 'group', 'interactive', 'image', 'video'])
const MAX_OPS = 50
const MAX_VISIBLE_BLOCKS = 20
const MAX_TABLE_CELLS = 5_000
const MAX_TEXT_CHARS = 50_000
const MAX_HTML_BYTES = 200_000

function finite(n: unknown): n is number {
  return typeof n === 'number' && Number.isFinite(n)
}

export function validatePatch(doc: CanvasDocument, patch: CanvasPatch, wishId: string): PatchValidation {
  const errors: string[] = []
  const conflict = patch.baseCanvasRevision !== doc.revision
  if (conflict) errors.push(`画布在运行期间已变化（基线 ${patch.baseCanvasRevision}，当前 ${doc.revision}）`)
  if (patch.protocolVersion !== '0.1') errors.push(`不支持的协议版本: ${String(patch.protocolVersion)}`)
  if (!Array.isArray(patch.operations)) {
    return { ok: false, errors: [...errors, '补丁缺少 operations'], conflict }
  }
  if (patch.operations.length > MAX_OPS) errors.push(`操作数量超限: ${patch.operations.length} > ${MAX_OPS}`)

  const wish = doc.blocks[wishId]
  if (!wish || wish.type !== 'wish') errors.push('许愿格不存在')
  const sheetId = wish?.sheetId

  const createdIds = new Set<string>()
  let visible = 0
  const knows = (id: string) => createdIds.has(id) || Boolean(doc.blocks[id])
  const ownedByWish = (id: string) => {
    const b = doc.blocks[id]
    return Boolean(b?.generated && b.generated.wishBlockId === wishId && !b.generated.detached)
  }

  patch.operations.forEach((op, i) => {
    const at = `操作#${i + 1}`
    switch (op.op) {
      case 'createBlock': {
        const b = op.block as CanvasBlock
        if (!b || typeof b.id !== 'string') return errors.push(`${at}: 块缺少 id`)
        if (knows(b.id)) errors.push(`${at}: 块 id 重复 ${b.id}`)
        if (!BLOCK_TYPES.has(b.type)) errors.push(`${at}: 未知块类型 ${String(b.type)}`)
        if (b.sheetId !== sheetId) errors.push(`${at}: 不允许写入其他 Sheet`)
        const r = b.rect
        if (!r || !finite(r.x) || !finite(r.y) || !finite(r.width) || !finite(r.height)) {
          errors.push(`${at}: 块坐标或尺寸不是有限数`)
        } else if (b.type !== 'group' && (r.width < MIN_BLOCK_WIDTH || r.height < MIN_BLOCK_HEIGHT)) {
          errors.push(`${at}: 块尺寸小于最小可操作尺寸`)
        }
        if (b.type !== 'group') visible += 1
        if (b.type === 'table') {
          const cells = b.content.rows.length * b.content.columns.length
          if (cells > MAX_TABLE_CELLS) errors.push(`${at}: 表格输出超过 ${MAX_TABLE_CELLS} 个单元格`)
        }
        if (b.type === 'text' && b.content.text.length > MAX_TEXT_CHARS) {
          errors.push(`${at}: 文本超过 ${MAX_TEXT_CHARS} 字符`)
        }
        if (b.type === 'interactive') {
          const size = b.content.html.length + b.content.css.length + b.content.js.length
          if (size > MAX_HTML_BYTES) errors.push(`${at}: 自定义 HTML 体积超限`)
          if (!b.content.manifest?.name) errors.push(`${at}: 自定义交互块缺少 manifest`)
        }
        if (b.type === 'wish') errors.push(`${at}: Agent 不允许创建许愿格`)
        if (b.type === 'image') {
          if (typeof b.content?.src !== 'string') errors.push(`${at}: 图片缺少 src`)
          else if (b.content.src.length > MAX_IMAGE_BYTES) errors.push(`${at}: 图片体积超过 ${MAX_IMAGE_BYTES / 1024 / 1024}MB`)
          else if (!/^(data:image\/|https?:\/\/)/.test(b.content.src)) errors.push(`${at}: 图片 src 必须是 data:image 或 http(s) 地址`)
        }
        if (b.type === 'video') {
          const total = (b.content?.src?.length ?? 0) + (b.content?.frames ?? []).reduce((n, f) => n + f.src.length, 0)
          if (total > MAX_IMAGE_BYTES * 4) errors.push(`${at}: 视频体积超限`)
          if (!b.content?.src && !b.content?.frames?.length) errors.push(`${at}: 视频既没有文件也没有帧序列`)
        }
        if (b.type === 'chart' && b.content.data.kind === 'tableBlock') {
          const src = b.content.data.blockId
          if (!knows(src)) errors.push(`${at}: 图表引用的表格不存在 ${src}`)
          else if (wouldCreateCycle(doc, wishId, src)) errors.push(`${at}: 引用会产生循环依赖`)
        }
        createdIds.add(b.id)
        break
      }
      case 'updateBlock': {
        if (!knows(op.blockId)) errors.push(`${at}: 引用的块不存在 ${op.blockId}`)
        else if (!createdIds.has(op.blockId) && !ownedByWish(op.blockId)) {
          errors.push(`${at}: 不允许修改用户块 ${op.blockId}`)
        }
        break
      }
      case 'createBinding': {
        if (!knows(op.binding?.targetBlockId)) errors.push(`${at}: 绑定目标不存在`)
        if (!knows(op.binding?.source?.blockId)) errors.push(`${at}: 绑定来源不存在`)
        break
      }
      case 'createGroup': {
        if (!knows(op.groupId)) errors.push(`${at}: 结果组不存在 ${op.groupId}`)
        for (const c of op.childBlockIds) if (!knows(c)) errors.push(`${at}: 结果组成员不存在 ${c}`)
        break
      }
      case 'resizeToFit':
        if (!knows(op.blockId)) errors.push(`${at}: 块不存在 ${op.blockId}`)
        break
      case 'addPresentationStep':
        if (!doc.presentationPaths.some((p) => p.id === op.pathId)) errors.push(`${at}: 讲述路径不存在`)
        break
      case 'updateTableCells': {
        const t = doc.blocks[op.blockId]
        if (!t || t.type !== 'table') return errors.push(`${at}: 表格不存在 ${op.blockId}`)
        for (const c of op.cells) {
          const row = t.content.rows.find((r) => r.id === c.rowId)
          if (!row) return errors.push(`${at}: 行不存在 ${c.rowId}`)
          const existing = row.cells[c.columnId]
          if (existing && existing.kind !== 'ai') errors.push(`${at}: 不允许覆盖用户单元格`)
        }
        break
      }
      default:
        errors.push(`${at}: 未知操作 ${(op as { op: string }).op}`)
    }
  })
  if (visible > MAX_VISIBLE_BLOCKS) errors.push(`单次最多创建 ${MAX_VISIBLE_BLOCKS} 个可见块，实际 ${visible}`)
  return { ok: errors.length === 0, errors, conflict }
}
