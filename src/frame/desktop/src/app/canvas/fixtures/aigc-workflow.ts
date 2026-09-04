/* ── Sample canvas 2: an AIGC short-film workflow (character sheets → storyboard → final video) ──
 *
 * Laid out left→right like a node graph, but every "node" is an ordinary canvas object:
 * inputs are text / table blocks, each stage is a wish block, and each stage's output group is
 * the next stage's data source. Editing an input marks everything downstream "需要刷新".
 * The sample opens fully executed (results are produced by the same offline generator the Mock
 * Agent uses), so the dependency chain can be explored immediately.
 */

import { generateAigc, type AigcKind } from '../agent/aigc'
import { buildContext } from '../agent/context'
import type { AgentRunRequest, CanvasPatch, CanvasPatchOperation } from '../agent/contracts'
import { mk, type Layout } from '../agent/mock-blocks'
import { createEmptyDocument, createFrameBlock, createTableBlock, createTextBlock, createWishBlock, tableContentFromMatrix } from '../domain/factories'
import { nowIso } from '../domain/ids'
import { applyCommand } from '../domain/reducer'
import { cameraToFit, refKey } from '../domain/selectors'
import type { CanvasDocument, WishBlock } from '../domain/types'

export const AIGC_TEMPLATE_ID = 'aigc-short-film-workflow'
export const AIGC_IDS = {
  intro: 'blk_aigc_intro',
  script: 'blk_aigc_script',
  characters: 'blk_aigc_characters',
  style: 'blk_aigc_style',
  wishCharacters: 'blk_aigc_wish_characters',
  wishStoryboard: 'blk_aigc_wish_storyboard',
  wishVideo: 'blk_aigc_wish_video',
  frameInput: 'blk_aigc_frame_input',
  frameCharacters: 'blk_aigc_frame_characters',
  frameStoryboard: 'blk_aigc_frame_storyboard',
  frameVideo: 'blk_aigc_frame_video',
}

export const AIGC_SCRIPT = `# 《雨夜快递员》剧本大纲
一句话：暴雨夜，快递员阿澈和机器人小满要在天亮前把一个神秘包裹送到城市最高处。

## 分场
1. 天台 · 夜 · 雨：阿澈收到加急订单，小满提示"目的地：云顶塔"。（阿澈、小满）
2. 霓虹街道 · 夜：两人穿过拥挤的夜市，躲开巡逻无人机。（阿澈、小满）
3. 老周面馆 · 夜：老周递来一碗热面，说出包裹的秘密。（阿澈、老周）
4. 高架桥 · 夜 · 雨：无人机追逐，小满用磁吸臂救下阿澈。（阿澈、小满）
5. 云顶塔顶 · 黎明：包裹打开，是一株会发光的种子。（阿澈、小满、老周）
6. 天台 · 日出：城市苏醒，两人吃着面看日出。（阿澈、小满）`

export const AIGC_CHARACTERS: Array<Array<string | number>> = [
  ['角色', '定位', '外貌', '性格', '主色', '服装/道具'],
  ['阿澈', '快递员 · 主角', '20 岁出头，短发，眼神倔强', '倔强、话少、讲义气', '青蓝', '防水骑行服、背包、耳机'],
  ['小满', 'AI 机器人助手', '圆头方身的小型机器人，胸口有指示灯', '话痨、乐观、爱吐槽', '琥珀', '磁吸臂、折叠伞'],
  ['老周', '面馆老板 · 引路人', '50 岁，微胖，总是系着围裙', '温和、神秘、爱讲故事', '朱红', '围裙、老花眼镜'],
]

export const AIGC_STYLE = `# 视觉风格
风格：赛博朋克 × 水彩，霓虹青紫为主色，胶片颗粒，16:9 电影宽幅
- 夜景以冷色为主，暖色只出现在面馆与日出
- 角色造型简洁，剪影可辨识`

export const AIGC_PROMPTS = {
  characters: '根据角色设定表和视觉风格，为每个角色生成一张竖版角色设定图，并附一段设定说明。保持每个角色的主色与道具特征，方便后续故事板引用。',
  storyboard: '根据剧本大纲的分场，结合角色设定图和视觉风格生成故事板：每个分场一个镜头，输出分镜表（镜号、场景、景别、出场角色、动作、时长）和每个镜头的画面。',
  video: '把故事板按镜号顺序合成为最终视频：使用分镜表中的时长，输出可播放的成片预览，并给出总时长和镜头数。',
}

function request(doc: CanvasDocument, wish: WishBlock, runId: string, anchor: { x: number; y: number }): AgentRunRequest {
  return {
    protocolVersion: '0.1',
    runId,
    canvas: { id: doc.id, revision: doc.revision, locale: 'zh-CN' },
    wish: { blockId: wish.id, prompt: wish.content.prompt, outputPreference: wish.content.outputPreference },
    context: buildContext(doc, wish).items,
    destination: { sheetId: wish.sheetId, anchor, maxWidth: 1000 },
    capabilities: ['read_canvas_context', 'create_standard_blocks', 'generate_media'],
  }
}

/** Run one stage synchronously with the offline generator and apply it exactly like the runner would. */
function runStage(doc: CanvasDocument, wishId: string, kind: AigcKind, runId: string, anchor: { x: number; y: number }): CanvasDocument {
  const wish = doc.blocks[wishId] as WishBlock
  const req = request(doc, wish, runId, anchor)
  const layout: Layout = { sheetId: wish.sheetId, x: anchor.x, y: anchor.y, runId, n: 0 }
  const result = generateAigc(kind, req, layout)
  const group = mk(layout, 'group', result.summary.slice(0, 40), { x: 0, y: 0, width: 10, height: 10 }, { childBlockIds: result.blocks.map((b) => b.id), summary: result.summary })
  const operations: CanvasPatchOperation[] = [...result.blocks.map((b): CanvasPatchOperation => ({ op: 'createBlock', block: b })), { op: 'createBlock', block: group }, { op: 'resizeToFit', blockId: group.id }]
  const patch: CanvasPatch = { protocolVersion: '0.1', runId, baseCanvasRevision: doc.revision, summary: result.summary, assumptions: result.assumptions, warnings: result.warnings, operations }
  let next = applyCommand(doc, { type: 'APPLY_AGENT_PATCH', patch, wishId, adapter: 'mock', replaceGroupIds: [] })
  next = applyCommand(next, { type: 'WISH_SET_STATE', id: wishId, state: 'succeeded', runId })
  const startedAt = nowIso()
  next = applyCommand(next, {
    type: 'WISH_PUSH_HISTORY',
    id: wishId,
    summary: {
      runId,
      startedAt,
      finishedAt: startedAt,
      status: 'succeeded',
      promptExcerpt: wish.content.prompt.slice(0, 80),
      sourceRevisions: wish.content.contextRefs.map((r) => ({ refKey: refKey(r), revision: doc.blocks[r.blockId]?.dataRevision ?? r.revision })),
      adapter: 'mock',
      groupId: group.id,
    },
  })
  return next
}

function addSource(doc: CanvasDocument, wishId: string, sourceId: string): CanvasDocument {
  const wish = doc.blocks[wishId] as WishBlock
  const src = doc.blocks[sourceId]
  if (!src) return doc
  return applyCommand(doc, { type: 'UPDATE_BLOCK', id: wishId, patch: { content: { ...wish.content, contextRefs: [...wish.content.contextRefs, { kind: 'block', blockId: sourceId, revision: src.dataRevision }] } } })
}

export function createAigcWorkflowCanvas(): CanvasDocument {
  let doc = createEmptyDocument('AI 短片工作流示例')
  doc.metadata.sourceTemplateId = AIGC_TEMPLATE_ID
  const sheet = doc.sheets[0]
  sheet.name = '短片工作流'
  const sheetId = sheet.id

  const intro = createTextBlock({
    id: AIGC_IDS.intro,
    sheetId,
    title: '这张画布怎么用',
    rect: { x: 60, y: 40, width: 1280, height: 120 },
    text: '# AI 短片工作流：角色设定 → 角色图 → 故事板 → 成片\n这是一条像 ComfyUI 那样从左到右的生成流程，但每个节点都是普通的画布对象：左侧是你能直接修改的**剧本 / 角色表 / 风格**，每个阶段是一个**许愿格**，上一阶段的结果组就是下一阶段的数据来源。试试改一改角色表里的"主色"或剧本里的某个分场，下游结果会标记为"需要刷新"，依次点"重新运行"即可全部更新。',
  })
  const script = createTextBlock({ id: AIGC_IDS.script, sheetId, title: '剧本大纲', rect: { x: 60, y: 220, width: 560, height: 400 }, text: AIGC_SCRIPT })
  const characters = createTableBlock({
    id: AIGC_IDS.characters,
    sheetId,
    title: '角色设定',
    rect: { x: 60, y: 650, width: 560, height: 190 },
    content: tableContentFromMatrix(AIGC_CHARACTERS, { hasHeader: true, source: { kind: 'sample', filename: '角色设定.xlsx', importedAt: nowIso() } }),
  })
  const style = createTextBlock({ id: AIGC_IDS.style, sheetId, title: '视觉风格', rect: { x: 60, y: 870, width: 560, height: 150 }, text: AIGC_STYLE })

  const wishCharacters = createWishBlock({
    id: AIGC_IDS.wishCharacters,
    sheetId,
    title: '许愿格 ①：生成角色设定图',
    rect: { x: 700, y: 220, width: 420, height: 300 },
    prompt: AIGC_PROMPTS.characters,
    contextRefs: [
      { kind: 'block', blockId: characters.id, revision: 0 },
      { kind: 'block', blockId: style.id, revision: 0 },
    ],
  })
  const wishStoryboard = createWishBlock({
    id: AIGC_IDS.wishStoryboard,
    sheetId,
    title: '许愿格 ②：生成故事板',
    rect: { x: 1480, y: 220, width: 420, height: 300 },
    prompt: AIGC_PROMPTS.storyboard,
    contextRefs: [
      { kind: 'block', blockId: script.id, revision: 0 },
      { kind: 'block', blockId: style.id, revision: 0 },
    ],
  })
  const wishVideo = createWishBlock({
    id: AIGC_IDS.wishVideo,
    sheetId,
    title: '许愿格 ③：合成最终视频',
    rect: { x: 2470, y: 220, width: 420, height: 300 },
    prompt: AIGC_PROMPTS.video,
    contextRefs: [],
  })

  const frames = [
    createFrameBlock({ id: AIGC_IDS.frameInput, sheetId, title: '① 输入：剧本 · 角色表 · 风格（可直接修改）', rect: { x: 40, y: 200, width: 600, height: 840 }, color: 'var(--cp-accent)' }),
    createFrameBlock({ id: AIGC_IDS.frameCharacters, sheetId, title: '② 角色设定图（AI 生成）', rect: { x: 680, y: 200, width: 760, height: 840 }, color: 'var(--cp-success)' }),
    createFrameBlock({ id: AIGC_IDS.frameStoryboard, sheetId, title: '③ 故事板（AI 生成，引用 ②）', rect: { x: 1460, y: 200, width: 970, height: 840 }, color: 'var(--cp-warning)' }),
    createFrameBlock({ id: AIGC_IDS.frameVideo, sheetId, title: '④ 最终视频（AI 生成，引用 ③）', rect: { x: 2450, y: 200, width: 960, height: 840 }, color: 'var(--cp-danger)' }),
  ]

  const inputs = [...frames, intro, script, characters, style, wishCharacters, wishStoryboard, wishVideo]
  doc.blocks = Object.fromEntries(inputs.map((b) => [b.id, b]))
  sheet.blockIds = inputs.map((b) => b.id)

  // execute the three stages in order, chaining each output group into the next wish
  doc = runStage(doc, wishCharacters.id, 'characters', 'run_aigc_characters', { x: 716, y: 604 })
  const groupA = (doc.blocks[wishCharacters.id] as WishBlock).content.generatedGroupIds[0]
  doc = addSource(doc, wishStoryboard.id, groupA)
  doc = runStage(doc, wishStoryboard.id, 'storyboard', 'run_aigc_storyboard', { x: 1496, y: 604 })
  const groupB = (doc.blocks[wishStoryboard.id] as WishBlock).content.generatedGroupIds[0]
  doc = addSource(doc, wishVideo.id, groupB)
  doc = runStage(doc, wishVideo.id, 'video', 'run_aigc_video', { x: 2486, y: 604 })

  const viewport = { width: 1100, height: 640 }
  const ts = nowIso()
  doc.presentationPaths = [
    {
      id: 'path_aigc',
      name: '工作流讲解',
      createdAt: ts,
      updatedAt: ts,
      steps: [
        { id: 'step_aigc_0', title: '一条从左到右的生成流程', note: '每个阶段都是许愿格，结果留在画布上并记得自己来自哪里。', camera: { x: 20, y: 10, zoom: 0.3 }, targetBlockIds: [], transitionMs: 600 },
        ...frames.map((f, i) => ({ id: `step_aigc_${i + 1}`, title: f.title ?? `阶段 ${i + 1}`, note: ['修改这里的任何内容，下游结果都会标记"需要刷新"', '角色表逐行生成设定图；主色与道具会保持一致', '分镜表 + 每镜画面；人物外观来自 ② 的设定图', '按分镜表时长合成的成片预览；接入真实模型后为 MP4'][i], camera: cameraToFit(f.rect, viewport, 60), targetBlockIds: [f.id], transitionMs: 700 })),
      ],
    },
  ]
  doc.activeSheetId = sheetId
  doc.sheets = doc.sheets.map((s) => (s.id === sheetId ? { ...s, camera: { x: 20, y: 10, zoom: 0.3 } } : s))
  return { ...doc, revision: 0 }
}
