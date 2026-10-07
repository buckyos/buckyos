/* The two phase-two demos (phase two §11), migrated from the ai-canvas prototype as commit
 * sequences through the public interface: data in the data tree, Blocks on free Surfaces, the Mock
 * wish executor as a definition entity, and a hand-written declarative definition (extension
 * sample 2). Nothing is pre-executed: the wishes run explicitly from the UI. */

import { mockWishDefOp } from '../ui/wish/mockWishDef'
import { unwrap, type AiwsClient } from './client'
import { PROTOCOL_VERSION, type Json, type Operation, type Placement, type WorkspaceSummary } from './types'

interface DemoCommit { message: string; operations: Operation[] }

function keys(n: number): string[] {
  // base-36 keys with gaps, never ending in 0
  return Array.from({ length: n }, (_, i) => `${(i + 1).toString(36)}z`)
}

function text(blockId: string, t: string, type: 'paragraph' | 'heading' = 'paragraph', level = 2): Json {
  return type === 'heading' ? { type: 'heading', attrs: { block_id: blockId, level }, content: [{ type: 'text', text: t }] } : { type: 'paragraph', attrs: { block_id: blockId }, content: [{ type: 'text', text: t }] }
}

function doc(lines: string[], prefix: string): Json {
  const content: Json[] = []
  lines.forEach((line, i) => {
    const h = /^(#{1,3})\s+(.*)$/.exec(line)
    if (h) content.push(text(`${prefix}-${i}`, h[2], 'heading', h[1].length))
    else if (line.trim()) content.push(text(`${prefix}-${i}`, line.replace(/^- /, '· ')))
  })
  return { type: 'doc', content }
}

function surface(id: string, title: string, key: string, mode: 'free' | 'flow' = 'free'): Operation[] {
  return [
    { op: 'entity.create', entity_id: `${id}-content`, type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: key, name: title, payload: { kind: 'folder', title, system: 'surface_content', surface_id: id } },
    { op: 'entity.create', entity_id: id, type_id: 'buckyos.container', parent_id: 'surfaces', order_key: key, name: title, payload: { kind: 'surface', layout: { mode }, title, content_folder_id: `${id}-content` } },
  ]
}

function cell(id: string, surfaceId: string, key: string, view: string, sourceId: string | undefined, placement: Placement, extra: Record<string, Json> = {}): Operation {
  return { op: 'entity.create', entity_id: id, type_id: 'buckyos.cell', parent_id: surfaceId, order_key: key, placement, payload: { view: { type: view }, ...(sourceId ? { source_ref: { entity_id: sourceId } } : {}), ...extra } }
}

function wish(id: string, parentId: string, key: string, title: string, prompt: string, inputs: string[], surfaceId: string, outputName: string): Operation {
  return { op: 'entity.create', entity_id: id, type_id: 'buckyos.wish', parent_id: parentId, order_key: key, name: title, payload: {
    title, prompt, executor: 'mock', output_mode: 'overwrite', inputs: inputs.map((entity_id) => ({ entity_id, version: { mode: 'follow' } })),
    output: { container_id: parentId, surface_id: surfaceId, name: outputName },
  } }
}

// ---- demo 1: quarterly business analysis (§11.1)

const SALES_ROWS: [string, string, number, number, number, number][] = [
  ['华东', '智能手表', 128000, 76000, 120000, 112000], ['华东', '降噪耳机', 96000, 58000, 100000, 101000], ['华东', '桌面音箱', 42000, 30000, 50000, 47000],
  ['华南', '智能手表', 101000, 61000, 95000, 88000], ['华南', '降噪耳机', 74000, 44000, 90000, 86000], ['华南', '桌面音箱', 23000, 25000, 40000, 31000],
  ['华北', '智能手表', 88000, 55000, 100000, 92000], ['华北', '降噪耳机', 66000, 40000, 70000, 61000], ['华北', '桌面音箱', 38000, 26000, 36000, 29000],
  ['西南', '智能手表', 54000, 36000, 70000, 60000], ['西南', '降噪耳机', 41000, 27000, 45000, 39000], ['西南', '桌面音箱', 19000, 14000, 20000, 18000],
]

export function quarterlyDemoCommits(): DemoCommit[] {
  const k = keys(40)
  const sales: Operation = { op: 'entity.create', entity_id: 'sales', type_id: 'buckyos.table-source', parent_id: 'data', order_key: k[0], name: '原始销售数据', payload: {
    title_field_id: 'product', description: '2026 Q2 销售明细（示例）',
    fields: [
      { field_id: 'region', name: '区域', type: 'text', required: true }, { field_id: 'product', name: '产品', type: 'text', required: true },
      { field_id: 'revenue', name: '销售额', type: 'number' }, { field_id: 'cost', name: '成本', type: 'number' }, { field_id: 'target', name: '目标', type: 'number' }, { field_id: 'prev', name: '上季度销售额', type: 'number' },
    ],
  } }
  const rows: Operation = { op: 'table.insert_records', source_id: 'sales', records: SALES_ROWS.map(([region, product, revenue, cost, target, prev], i) => ({ record_id: `s-${i + 1}`, values: { region, product, revenue, cost, target, prev } })) }
  const intro: Operation = { op: 'entity.create', entity_id: 'intro', type_id: 'buckyos.richtext', parent_id: 'data', order_key: k[1], name: '说明', payload: { content: doc(['# 2026 Q2 经营分析', '这是一张普通的季度销售表。你可以在数据源模式中直接修改数字，也可以在画布上的许愿格里用一句话说出你想要的分析。许愿格由模拟执行器处理：结果固定构造，标明"模拟"。'], 'intro') } }
  const kpi: Operation = { op: 'entity.create', entity_id: 'def-kpi', type_id: 'buckyos.block-def', parent_id: 'data', order_key: k[2], name: 'kpi-card', payload: {
    def_id: 'sample.kpi-card', version: 1, kind: 'declarative', title: '指标卡（声明式样本）', description: '人工准备、模拟 AI 产出的声明式 Block 定义：绑定表格，显示合计与分组柱图；查看模式有专用展现与动作。',
    accepts: ['buckyos.table-source'], default_size: { w: 320, h: 240 },
    declarative: {
      layout: 'stack', accent: '#4f8df7',
      items: [{ kind: 'metric', label: '销售额合计', bind: 'sum:销售额', format: 'currency' }, { kind: 'metric', label: '成本合计', bind: 'sum:成本', format: 'currency' }, { kind: 'bar', label: '按区域', bind: '销售额', by: '区域', format: 'currency' }],
      view: { items: [{ kind: 'badge', text: '查看模式' }, { kind: 'metric', label: '销售额合计', bind: 'sum:销售额', format: 'currency' }, { kind: 'list', label: '产品', bind: '产品', limit: 4 }] },
      actions: [{ id: 'open-source', label: '打开数据表', command: 'open_source', modes: ['edit', 'view'] }, { id: 'open-def', label: '查看定义', command: 'open_definition', modes: ['view'] }],
      inspector: [{ key: 'accent', label: '主色', kind: 'color', modes: ['edit'] }, { key: 'note', label: '备注', kind: 'text', modes: ['edit', 'view'] }],
    },
  } }
  const main = surface('sf-analysis', '经营分析', k[0])
  const detail = surface('sf-detail', '数据明细', k[1])
  const wishOp = wish('wish-analysis', 'sf-analysis-content', k[0], '季度经营分析', '基于「原始销售数据」生成本季度经营分析：核心指标、各区域对比、异常明细和管理层总结', ['sales'], 'sf-analysis', '经营分析结果')
  return [
    { message: '销售数据与说明', operations: [sales, rows, intro] },
    { message: 'Block 定义：Mock 许愿格与指标卡样本', operations: [mockWishDefOp(k[3]) as Operation, kpi] },
    { message: '两张画布', operations: [...main, ...detail] },
    { message: '经营分析画布', operations: [
      wishOp,
      cell('blk-intro', 'sf-analysis', k[0], 'richtext', 'intro', { x: 40, y: 40, w: 900, h: 120 }, { title: '说明' }),
      cell('blk-sales', 'sf-analysis', k[1], 'table', 'sales', { x: 40, y: 190, w: 900, h: 420 }, { title: '原始销售数据', fields: [{ field_id: 'region', width: 90 }, { field_id: 'product', width: 120 }, { field_id: 'revenue', width: 110 }, { field_id: 'cost', width: 100 }, { field_id: 'target', width: 100 }, { field_id: 'prev', width: 130 }] }),
      cell('blk-wish', 'sf-analysis', k[2], 'wish', 'wish-analysis', { x: 980, y: 190, w: 420, h: 300 }, { title: '许愿格：季度经营分析' }),
      cell('blk-kpi', 'sf-analysis', k[3], 'declarative', 'sales', { x: 980, y: 40, w: 420, h: 130 }, { title: '指标卡（声明式）', def_ref: { entity_id: 'def-kpi' } }),
      cell('blk-frame', 'sf-analysis', k[4], 'frame', undefined, { x: 960, y: 520, w: 1100, h: 820 }, { title: '运行后结果将出现在这里', config: { color: '#4f8df7' } }),
    ] },
    { message: '数据明细画布：同一份数据的另一组 Block', operations: [
      cell('blk-sales-2', 'sf-detail', k[0], 'table', 'sales', { x: 40, y: 40, w: 760, h: 360 }, { title: '华东明细', filter: { op: 'cmp', field_id: 'region', operator: 'eq', value: '华东' } }),
      cell('blk-chart-2', 'sf-detail', k[1], 'sample.bar-chart', 'sales', { x: 840, y: 40, w: 420, h: 300 }, { title: '各区域销售额', config: { value: 'revenue', by: 'region' } }),
      cell('blk-metric-2', 'sf-detail', k[2], 'sample.metric', 'sales', { x: 840, y: 360, w: 220, h: 120 }, { title: '销售额合计', config: { field: 'revenue', aggregation: 'sum' } }),
    ] },
  ]
}

// ---- demo 2: AI short-film workflow (§11.2)

const SCRIPT = ['# 《雨夜快递员》剧本大纲', '一句话：暴雨夜，快递员阿澈和机器人小满要在天亮前把一个神秘包裹送到城市最高处。', '## 分场',
  '1. 天台 · 夜 · 雨：阿澈收到加急订单，小满提示"目的地：云顶塔"。（阿澈、小满）', '2. 霓虹街道 · 夜：两人穿过拥挤的夜市，躲开巡逻无人机。（阿澈、小满）',
  '3. 老周面馆 · 夜：老周递来一碗热面，说出包裹的秘密。（阿澈、老周）', '4. 高架桥 · 夜 · 雨：无人机追逐，小满用磁吸臂救下阿澈。（阿澈、小满）',
  '5. 云顶塔顶 · 黎明：包裹打开，是一株会发光的种子。（阿澈、小满、老周）', '6. 天台 · 日出：城市苏醒，两人吃着面看日出。（阿澈、小满）']
const STYLE = ['# 视觉风格', '风格：赛博朋克 × 水彩，霓虹青紫为主色，胶片颗粒，16:9 电影宽幅', '- 夜景以冷色为主，暖色只出现在面馆与日出', '- 角色造型简洁，剪影可辨识']
const CHARACTERS: [string, string, string, string, string, string][] = [
  ['阿澈', '快递员 · 主角', '20 岁出头，短发，眼神倔强', '倔强、话少、讲义气', '青蓝', '防水骑行服、背包、耳机'],
  ['小满', 'AI 机器人助手', '圆头方身的小型机器人，胸口有指示灯', '话痨、乐观、爱吐槽', '琥珀', '磁吸臂、折叠伞'],
  ['老周', '面馆老板 · 引路人', '50 岁，微胖，总是系着围裙', '温和、神秘、爱讲故事', '朱红', '围裙、老花眼镜'],
]

export function filmDemoCommits(): DemoCommit[] {
  const k = keys(40)
  const script: Operation = { op: 'entity.create', entity_id: 'script', type_id: 'buckyos.richtext', parent_id: 'data', order_key: k[0], name: '剧本大纲', payload: { content: doc(SCRIPT, 'script') } }
  const characters: Operation = { op: 'entity.create', entity_id: 'characters', type_id: 'buckyos.table-source', parent_id: 'data', order_key: k[1], name: '角色设定', payload: {
    title_field_id: 'name', fields: [{ field_id: 'name', name: '角色', type: 'text', required: true }, { field_id: 'role', name: '定位', type: 'text' }, { field_id: 'look', name: '外貌', type: 'text' }, { field_id: 'personality', name: '性格', type: 'text' }, { field_id: 'color', name: '主色', type: 'text' }, { field_id: 'outfit', name: '服装/道具', type: 'text' }],
  } }
  const rows: Operation = { op: 'table.insert_records', source_id: 'characters', records: CHARACTERS.map(([name, role, look, personality, color, outfit], i) => ({ record_id: `ch-${i + 1}`, values: { name, role, look, personality, color, outfit } })) }
  const style: Operation = { op: 'entity.create', entity_id: 'style', type_id: 'buckyos.richtext', parent_id: 'data', order_key: k[2], name: '视觉风格', payload: { content: doc(STYLE, 'style') } }
  const howto: Operation = { op: 'entity.create', entity_id: 'howto', type_id: 'buckyos.richtext', parent_id: 'data', order_key: k[3], name: '这张画布怎么用', payload: { content: doc(['# AI 短片工作流：角色设定 → 角色图 → 故事板 → 成片', '从左到右的生成流程，但每个节点都是普通的画布对象：左侧是你能直接修改的剧本 / 角色表 / 风格，每个阶段是一个许愿格，上一阶段的结果文件夹就是下一阶段的输入。改一改角色表或剧本，下游结果会标记为"需要刷新"，依次显式刷新即可。全部由模拟执行器生成，不调用真实模型。'], 'howto') } }
  const sf = surface('sf-film', '短片工作流', k[0])
  const w1 = wish('wish-characters', 'sf-film-content', k[0], '许愿格 ①：生成角色设定图', '根据「角色设定」表和「视觉风格」，为每个角色生成一张竖版角色设定图，并附一段设定说明。保持每个角色的主色与道具特征，方便后续故事板引用。', ['characters', 'style'], 'sf-film', '角色设定图')
  const w2 = wish('wish-storyboard', 'sf-film-content', k[1], '许愿格 ②：生成故事板', '根据「剧本大纲」的分场，结合「角色设定图」和「视觉风格」生成故事板：每个分场一个镜头，输出分镜表（镜号、场景、景别、出场角色、动作、时长）和每个镜头的画面。', ['script', 'style', 'wish-characters-out-data'], 'sf-film', '故事板')
  const w3 = wish('wish-video', 'sf-film-content', k[2], '许愿格 ③：合成最终视频', '把「故事板」按镜号顺序合成为最终视频：使用分镜表中的时长，输出可播放的成片预览，并给出总时长和镜头数。', ['wish-storyboard-out-data'], 'sf-film', '成片')
  // the second and third wishes reference result folders that do not exist until the previous stage ran: the
  // references are declared after the folders exist (see below), so the fixture declares them without inputs first
  const stripInputs = (op: Operation) => ({ ...op, payload: { ...(op.payload as Record<string, Json>), inputs: ((op.payload as { inputs: { entity_id: string }[] }).inputs).filter((i) => !i.entity_id.startsWith('wish-')) } })
  return [
    { message: '剧本、角色表、风格', operations: [script, characters, rows, style, howto] },
    { message: 'Mock 许愿格定义', operations: [mockWishDefOp(k[4]) as Operation] },
    { message: '工作流画布', operations: [
      ...sf, w1, stripInputs(w2), stripInputs(w3),
      cell('blk-howto', 'sf-film', k[0], 'richtext', 'howto', { x: 60, y: 40, w: 1280, h: 130 }, { title: '这张画布怎么用' }),
      cell('blk-frame-in', 'sf-film', k[1], 'frame', undefined, { x: 40, y: 200, w: 600, h: 900 }, { title: '① 输入：剧本 · 角色表 · 风格（可直接修改）', config: { color: '#4f8df7' } }),
      cell('blk-script', 'sf-film', k[2], 'richtext', 'script', { x: 60, y: 240, w: 560, h: 380 }, { title: '剧本大纲' }),
      cell('blk-characters', 'sf-film', k[3], 'table', 'characters', { x: 60, y: 640, w: 560, h: 220 }, { title: '角色设定' }),
      cell('blk-style', 'sf-film', k[4], 'richtext', 'style', { x: 60, y: 880, w: 560, h: 180 }, { title: '视觉风格' }),
      cell('blk-frame-a', 'sf-film', k[5], 'frame', undefined, { x: 680, y: 200, w: 1300, h: 900 }, { title: '② 角色设定图（许愿格 ①）', config: { color: '#22c55e' } }),
      cell('blk-wish-1', 'sf-film', k[6], 'wish', 'wish-characters', { x: 700, y: 240, w: 420, h: 300 }, { title: '许愿格 ①：生成角色设定图' }),
      cell('blk-frame-b', 'sf-film', k[7], 'frame', undefined, { x: 2020, y: 200, w: 1500, h: 1100 }, { title: '③ 故事板（许愿格 ②，引用 ②）', config: { color: '#f59e0b' } }),
      cell('blk-wish-2', 'sf-film', k[8], 'wish', 'wish-storyboard', { x: 2040, y: 240, w: 420, h: 300 }, { title: '许愿格 ②：生成故事板' }),
      cell('blk-frame-c', 'sf-film', k[9], 'frame', undefined, { x: 3560, y: 200, w: 1200, h: 900 }, { title: '④ 最终视频（许愿格 ③，引用 ③）', config: { color: '#ef4444' } }),
      cell('blk-wish-3', 'sf-film', k[10], 'wish', 'wish-video', { x: 3580, y: 240, w: 420, h: 300 }, { title: '许愿格 ③：合成最终视频' }),
    ] },
  ]
}

/** A template stopped after its workspace was created: the partial workspace exists and is named here. */
export class TemplateFailure extends Error {
  readonly workspace: WorkspaceSummary
  readonly stage: string
  constructor(workspace: WorkspaceSummary, stage: string, cause: unknown) {
    super(`${stage}：${cause instanceof Error ? cause.message : String(cause)}`, { cause })
    this.name = 'TemplateFailure'
    this.workspace = workspace
    this.stage = stage
  }
}

export type DemoKind = 'quarterly' | 'film'

export async function createDemoWorkspace(client: AiwsClient, kind: DemoKind, title: string): Promise<WorkspaceSummary> {
  const commits = kind === 'quarterly' ? quarterlyDemoCommits() : filmDemoCommits()
  const workspace = unwrap(await client.wsCreate(title))
  for (const [index, commit] of commits.entries()) {
    try {
      const result = await client.commit({
        protocol_version: PROTOCOL_VERSION, workspace_id: workspace.workspace_id, epoch: workspace.epoch,
        idempotency_key: `demo/${kind}/${index + 1}`, session_id: 'demo', origin: 'human', message: commit.message, operations: commit.operations,
      })
      if (result.status !== 'accepted') throw new Error(`提交未被接受：${result.code} ${JSON.stringify(result)}`)
    } catch (error) {
      throw new TemplateFailure(workspace, `写入模板内容的第 ${index + 1}/${commits.length} 步失败`, error)
    }
  }
  return workspace
}
