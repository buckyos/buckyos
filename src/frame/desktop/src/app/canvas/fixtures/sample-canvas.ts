/* ── Sample canvas: quarterly sales table + preset wish + placeholder frame ── */

import { createEmptyDocument, createFrameBlock, createTableBlock, createTextBlock, createWishBlock, tableContentFromMatrix } from '../domain/factories'
import { nowIso } from '../domain/ids'
import type { CanvasDocument } from '../domain/types'
import { QUARTERLY_SALES_MATRIX, SAMPLE_WISH_PROMPT } from './quarterly-sales'

export const SAMPLE_TEMPLATE_ID = 'quarterly-sales-analysis'
export const SAMPLE_IDS = {
  table: 'blk_sample_sales_table',
  wish: 'blk_sample_wish',
  frame: 'blk_sample_result_frame',
  intro: 'blk_sample_intro',
}

export function createSampleCanvas(): CanvasDocument {
  const doc = createEmptyDocument('季度经营分析示例')
  doc.metadata.sourceTemplateId = SAMPLE_TEMPLATE_ID
  const sheet = doc.sheets[0]
  sheet.name = '经营分析'
  sheet.camera = { x: 60, y: 60, zoom: 0.85 }

  const table = createTableBlock({
    id: SAMPLE_IDS.table,
    sheetId: sheet.id,
    title: '原始销售数据',
    rect: { x: 80, y: 140, width: 900, height: 520 },
    content: tableContentFromMatrix(QUARTERLY_SALES_MATRIX, {
      hasHeader: true,
      source: { kind: 'sample', filename: '2026Q2_sales.xlsx', worksheet: '销售明细', importedAt: nowIso() },
    }),
  })

  const intro = createTextBlock({
    id: SAMPLE_IDS.intro,
    sheetId: sheet.id,
    title: '说明',
    rect: { x: 80, y: 40, width: 900, height: 80 },
    text: '# 2026 Q2 经营分析\n这是一张普通的季度销售表。你可以直接修改表格里的数字，也可以在右侧的许愿格里用一句话说出你想要的分析。',
  })

  const wish = createWishBlock({
    id: SAMPLE_IDS.wish,
    sheetId: sheet.id,
    title: '许愿格：季度经营分析',
    rect: { x: 1040, y: 140, width: 420, height: 360 },
    prompt: SAMPLE_WISH_PROMPT,
    contextRefs: [{ kind: 'block', blockId: table.id, revision: 0 }],
  })

  const frame = createFrameBlock({
    id: SAMPLE_IDS.frame,
    sheetId: sheet.id,
    title: '运行后结果将出现在这里',
    rect: { x: 1040, y: 540, width: 1040, height: 760 },
    color: 'var(--cp-accent-soft)',
  })

  doc.blocks = { [intro.id]: intro, [table.id]: table, [wish.id]: wish, [frame.id]: frame }
  sheet.blockIds = [frame.id, intro.id, table.id, wish.id]
  doc.presentationPaths = [
    { id: 'path_sample', name: '季度汇报', steps: [], createdAt: nowIso(), updatedAt: nowIso() },
  ]
  return doc
}
