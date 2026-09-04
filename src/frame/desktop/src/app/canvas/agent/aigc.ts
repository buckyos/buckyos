/* ── Offline AIGC generator for the Mock Agent: character sheets → storyboard → video preview ──
 *
 * Everything here is deterministic and pure (seeded by the prompt/context), so the same input
 * always draws the same pictures. A real Agent would call image / video models and return the
 * same CanvasPatch shape; only the pixels differ.
 */

import { tableContentFromMatrix } from '../domain/factories'
import type { CanvasBlock, ImageBlockContent, VideoBlockContent, VideoFrame } from '../domain/types'
import type { AgentContextItem, AgentRunRequest } from './contracts'
import { hash, mk, rng, type Layout, type MockResult } from './mock-blocks'

export type AigcKind = 'characters' | 'storyboard' | 'video'

type TableItem = Extract<AgentContextItem, { kind: 'table' }>
type ImageItem = Extract<AgentContextItem, { kind: 'image' }>

export interface CharacterSpec {
  name: string
  role: string
  look: string
  personality: string
  colorText: string
  outfit: string
  color: string
  accent: string
  robot: boolean
}

export interface ShotSpec {
  no: number
  setting: string
  action: string
  chars: string[]
  shotType: string
  seconds: number
}

/* ── context helpers ── */

export function flattenContext(items: AgentContextItem[]): AgentContextItem[] {
  const out: AgentContextItem[] = []
  for (const it of items) {
    out.push(it)
    if (it.kind === 'group') out.push(...flattenContext(it.items))
  }
  return out
}

function findCol(cols: string[], ...names: string[]): string | undefined {
  for (const n of names) {
    const hit = cols.find((c) => c === n) ?? cols.find((c) => c.toLowerCase().includes(n.toLowerCase()))
    if (hit) return hit
  }
  return undefined
}

export function isCharacterTable(t: TableItem): boolean {
  const cols = t.columns.map((c) => c.name)
  return Boolean(findCol(cols, '角色', '人物', '姓名', '名字', 'name', 'character')) && Boolean(findCol(cols, '外貌', '外形', '描述', '定位', '性格', '主色', 'look', 'role'))
}

/**
 * Which stage a wish asks for. Prompts mention neighbouring stages ("…方便后续故事板引用"),
 * so the decision leans on what the data sources actually contain, with the prompt as a tie-breaker.
 */
export function detectAigc(req: AgentRunRequest): AigcKind | null {
  const prompt = req.wish.prompt
  const items = flattenContext(req.context)
  const images = items.filter((i): i is ImageItem => i.kind === 'image')
  const hasFrames = images.some((i) => /^S\d+/.test(i.title)) || items.some((i) => i.kind === 'table' && Boolean(findCol(i.columns.map((c) => c.name), '镜号', '时长', 'shot')))
  const hasCharImages = images.some((i) => !/^S\d+/.test(i.title))
  const hasScript = items.some((i) => i.kind === 'text' && /剧本|大纲|故事|script/i.test(i.title))
  const charTable = items.find((i): i is TableItem => i.kind === 'table' && isCharacterTable(i))
  const wantsVideo = /视频|成片|合成|剪辑|video|render/i.test(prompt)
  const wantsStoryboard = /故事板|分镜|storyboard/i.test(prompt)
  const wantsCharacters = /角色|人物|立绘|形象|设定图|character/i.test(prompt)
  if (wantsVideo && hasFrames) return 'video'
  if (wantsStoryboard && (hasScript || hasCharImages || !charTable)) return 'storyboard'
  if (charTable && wantsCharacters) return 'characters'
  if (wantsStoryboard) return 'storyboard'
  if (wantsVideo && images.length) return 'video'
  return null
}

/* ── colours & style ── */

const COLOR_WORDS: Array<[RegExp, string]> = [
  [/青|cyan|teal/i, '#22d3ee'],
  [/蓝|blue|navy/i, '#3b82f6'],
  [/紫|purple|violet/i, '#a855f7'],
  [/朱|红|red|crimson/i, '#ef4444'],
  [/橙|orange/i, '#f97316'],
  [/琥珀|amber|金|gold|黄|yellow/i, '#f59e0b'],
  [/绿|green/i, '#22c55e'],
  [/粉|pink|magenta/i, '#ec4899'],
  [/白|white|银|silver/i, '#e5e7eb'],
  [/黑|black|炭/i, '#334155'],
  [/灰|gray|grey/i, '#94a3b8'],
  [/棕|褐|brown/i, '#a16207'],
]

export function pickColor(text: string, seed: string): string {
  for (const [re, hex] of COLOR_WORDS) if (re.test(text)) return hex
  return `hsl(${hash(seed) % 360} 70% 60%)`
}

function accentFor(color: string, seed: string): string {
  const h = (hash(seed) % 360) + 180
  return color.startsWith('hsl') ? `hsl(${h % 360} 80% 70%)` : ['#fde68a', '#a5f3fc', '#f9a8d4', '#bbf7d0'][hash(seed) % 4]
}

function isRobot(text: string): boolean {
  return /机器人|机械|AI|android|robot|droid/i.test(text)
}

export function styleLabel(req: AgentRunRequest): string {
  const items = flattenContext(req.context)
  const styleText = items.find((i): i is Extract<AgentContextItem, { kind: 'text' }> => i.kind === 'text' && (/风格|style/i.test(i.title) || /风格[:：]/.test(i.text)))
  const raw = styleText ? styleText.text : req.wish.prompt
  const line = raw
    .split('\n')
    .map((l) => l.replace(/^#+\s*/, '').replace(/^风格[:：]\s*/, '').trim())
    .find((l) => l.length > 0 && !/^风格|^视觉风格$/.test(l))
  return (line ?? '默认风格').slice(0, 22)
}

/* ── svg helpers ── */

function esc(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')
}

function dataUrl(svg: string): string {
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`
}

const FONT = 'font-family="system-ui,-apple-system,Segoe UI,PingFang SC,Noto Sans CJK SC,sans-serif"'

/** A simplified figure standing on (x, baseY); height ≈ 200 × scale. */
function figure(x: number, baseY: number, s: number, c: CharacterSpec, seed: number): string {
  const r = rng(seed)
  const skin = ['#f1d3b3', '#e8c39e', '#d9a679', '#c68a5b'][seed % 4]
  const hair = ['#111827', '#3f2a1d', '#7c2d12', '#1e293b', '#4c1d95'][seed % 5]
  const parts: string[] = []
  parts.push(`<ellipse cx="${x}" cy="${baseY}" rx="${44 * s}" ry="${8 * s}" fill="#000" opacity="0.35"/>`)
  parts.push(`<path d="M${x - 40 * s},${baseY} L${x - 34 * s},${baseY - 130 * s} Q${x},${baseY - 150 * s} ${x + 34 * s},${baseY - 130 * s} L${x + 40 * s},${baseY} Z" fill="${c.color}"/>`)
  parts.push(`<rect x="${x - 6 * s}" y="${baseY - 132 * s}" width="${12 * s}" height="${62 * s}" rx="${4 * s}" fill="${c.accent}" opacity="0.85"/>`)
  if (/围裙/.test(c.outfit)) parts.push(`<rect x="${x - 26 * s}" y="${baseY - 100 * s}" width="${52 * s}" height="${90 * s}" rx="${6 * s}" fill="#f8fafc" opacity="0.85"/>`)
  if (/背包|包/.test(c.outfit)) parts.push(`<rect x="${x - 58 * s}" y="${baseY - 125 * s}" width="${24 * s}" height="${60 * s}" rx="${6 * s}" fill="${c.accent}"/>`)
  if (c.robot) {
    const hy = baseY - 190 * s
    parts.push(`<rect x="${x - 26 * s}" y="${hy}" width="${52 * s}" height="${46 * s}" rx="${10 * s}" fill="#cbd5e1"/>`)
    parts.push(`<line x1="${x}" y1="${hy}" x2="${x}" y2="${hy - 18 * s}" stroke="#cbd5e1" stroke-width="${3 * s}"/><circle cx="${x}" cy="${hy - 20 * s}" r="${5 * s}" fill="${c.accent}"/>`)
    parts.push(`<rect x="${x - 17 * s}" y="${hy + 16 * s}" width="${12 * s}" height="${8 * s}" rx="${2 * s}" fill="${c.accent}"/><rect x="${x + 5 * s}" y="${hy + 16 * s}" width="${12 * s}" height="${8 * s}" rx="${2 * s}" fill="${c.accent}"/>`)
  } else {
    const cy = baseY - 165 * s
    parts.push(`<circle cx="${x}" cy="${cy}" r="${28 * s}" fill="${skin}"/>`)
    parts.push(`<path d="M${x - 29 * s},${cy - 4 * s} Q${x},${cy - 44 * s} ${x + 29 * s},${cy - 4 * s} Q${x + 20 * s},${cy - 22 * s} ${x},${cy - 26 * s} Q${x - 20 * s},${cy - 22 * s} ${x - 29 * s},${cy - 4 * s} Z" fill="${hair}"/>`)
    parts.push(`<circle cx="${x - 10 * s}" cy="${cy + 2 * s}" r="${2.5 * s}" fill="#111827"/><circle cx="${x + 10 * s}" cy="${cy + 2 * s}" r="${2.5 * s}" fill="#111827"/>`)
    if (/帽/.test(c.outfit)) parts.push(`<rect x="${x - 34 * s}" y="${cy - 40 * s}" width="${68 * s}" height="${16 * s}" rx="${4 * s}" fill="${c.accent}"/>`)
    if (/眼镜|护目镜/.test(c.outfit)) parts.push(`<circle cx="${x - 10 * s}" cy="${cy + 2 * s}" r="${8 * s}" fill="none" stroke="${c.accent}" stroke-width="${2 * s}"/><circle cx="${x + 10 * s}" cy="${cy + 2 * s}" r="${8 * s}" fill="none" stroke="${c.accent}" stroke-width="${2 * s}"/>`)
    if (/耳机|耳麦/.test(c.outfit)) parts.push(`<path d="M${x - 30 * s},${cy} A${30 * s},${30 * s} 0 0 1 ${x + 30 * s},${cy}" fill="none" stroke="${c.accent}" stroke-width="${4 * s}"/>`)
  }
  if (/伞/.test(c.outfit)) {
    const ux = x + 50 * s
    const uy = baseY - 200 * s
    parts.push(`<path d="M${ux - 60 * s},${uy + 30 * s} A${60 * s},${60 * s} 0 0 1 ${ux + 60 * s},${uy + 30 * s} Z" fill="${c.accent}" opacity="0.9"/><line x1="${ux}" y1="${uy + 30 * s}" x2="${ux}" y2="${baseY - 60 * s}" stroke="#e2e8f0" stroke-width="${3 * s}"/>`)
  }
  if (r() > 0.5) parts.push(`<circle cx="${x + 30 * s}" cy="${baseY - 110 * s}" r="${5 * s}" fill="${c.accent}" opacity="0.7"/>`)
  return parts.join('')
}

/* ── character sheet ── */

export function characterCard(c: CharacterSpec, style: string): string {
  const seed = hash(`${c.name}|${c.look}|${style}`)
  const r = rng(seed)
  const deco: string[] = []
  for (let i = 0; i < 7; i++) {
    const cx = Math.round(r() * 480)
    const cy = Math.round(r() * 420)
    const rad = Math.round(10 + r() * 60)
    deco.push(`<circle cx="${cx}" cy="${cy}" r="${rad}" fill="${i % 2 ? c.accent : c.color}" opacity="${(0.05 + r() * 0.12).toFixed(2)}"/>`)
  }
  for (let i = 0; i < 4; i++) {
    const y = Math.round(60 + r() * 380)
    deco.push(`<line x1="0" y1="${y}" x2="480" y2="${y - 40 + Math.round(r() * 80)}" stroke="${c.accent}" stroke-width="1" opacity="0.18"/>`)
  }
  const traits = c.personality.split(/[、,，/\s]+/).filter(Boolean).slice(0, 3)
  const chips = traits
    .map((t, i) => {
      const w = t.length * 13 + 18
      const x = 24 + i * (w + 8)
      return `<rect x="${x}" y="562" width="${w}" height="24" rx="12" fill="#fff" opacity="0.14"/><text x="${x + w / 2}" y="579" text-anchor="middle" font-size="12" fill="#f8fafc" ${FONT}>${esc(t)}</text>`
    })
    .join('')
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 480 600" width="480" height="600">
<defs>
<linearGradient id="bg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#0b1020"/><stop offset="1" stop-color="${c.color}" stop-opacity="0.55"/></linearGradient>
<radialGradient id="glow"><stop offset="0" stop-color="${c.accent}" stop-opacity="0.5"/><stop offset="1" stop-color="${c.accent}" stop-opacity="0"/></radialGradient>
</defs>
<rect width="480" height="600" fill="#0b1020"/><rect width="480" height="600" fill="url(#bg)"/>
<circle cx="240" cy="260" r="220" fill="url(#glow)"/>
${deco.join('')}
${figure(240, 470, 1.55, c, seed)}
<rect x="0" y="484" width="480" height="116" fill="#020617" opacity="0.55"/>
<text x="24" y="522" font-size="32" font-weight="700" fill="#f8fafc" ${FONT}>${esc(c.name)}</text>
<text x="24" y="548" font-size="15" fill="${c.accent}" ${FONT}>${esc(c.role)}${c.colorText ? ` · 主色 ${esc(c.colorText)}` : ''}</text>
${chips}
<text x="20" y="30" font-size="12" fill="#e2e8f0" opacity="0.8" ${FONT}>角色设定图 · ${esc(style)}</text>
<text x="460" y="30" text-anchor="end" font-size="11" fill="#e2e8f0" opacity="0.6" ${FONT}>Mock AI · 离线生成</text>
</svg>`
  return dataUrl(svg)
}

export function parseCharacters(table: TableItem): CharacterSpec[] {
  const cols = table.columns.map((c) => c.name)
  const name = findCol(cols, '角色', '人物', '姓名', '名字', 'name', 'character')
  const role = findCol(cols, '定位', '身份', '职业', 'role')
  const look = findCol(cols, '外貌', '外形', '描述', 'look', 'appearance')
  const personality = findCol(cols, '性格', 'personality')
  const color = findCol(cols, '主色', '颜色', '色调', 'color')
  const outfit = findCol(cols, '服装', '道具', '穿着', '装扮', 'outfit')
  const str = (row: Record<string, unknown>, col?: string) => (col && row[col] != null ? String(row[col]) : '')
  return table.rows
    .map((row) => {
      const n = str(row, name).trim()
      if (!n) return null
      const all = [str(row, role), str(row, look), str(row, outfit)].join(' ')
      const colorText = str(row, color)
      const c = pickColor(colorText || all, n)
      return {
        name: n,
        role: str(row, role) || '角色',
        look: str(row, look),
        personality: str(row, personality),
        colorText,
        outfit: str(row, outfit),
        color: c,
        accent: accentFor(c, n),
        robot: isRobot(all),
      }
    })
    .filter((c): c is CharacterSpec => Boolean(c))
    .slice(0, 6)
}

function charactersFromImages(images: ImageItem[]): CharacterSpec[] {
  return images
    .filter((im) => !/^S\d+/.test(im.title))
    .map((im) => {
      const meta = `${im.alt ?? ''} ${im.caption ?? ''}`
      const color = pickColor(meta, im.title)
      return { name: im.title, role: '', look: im.alt ?? '', personality: '', colorText: '', outfit: meta, color, accent: accentFor(color, im.title), robot: isRobot(meta) }
    })
}

function generateCharacters(req: AgentRunRequest, layout: Layout): MockResult {
  const items = flattenContext(req.context)
  const table = items.find((i): i is TableItem => i.kind === 'table' && isCharacterTable(i))!
  const chars = parseCharacters(table)
  const style = styleLabel(req)
  const blocks: CanvasBlock[] = []
  const warnings: string[] = []
  if (chars.length === 0) warnings.push('角色表中没有可用的角色名')
  chars.forEach((c, i) => {
    const content: ImageBlockContent = {
      src: characterCard(c, style),
      alt: `${c.role}，${c.look}${c.colorText ? `，主色${c.colorText}` : ''}${c.outfit ? `，${c.outfit}` : ''}`,
      caption: c.role,
      fit: 'cover',
      naturalWidth: 480,
      naturalHeight: 600,
      source: { kind: 'generated', prompt: `${style} 角色设定图：${c.name}，${c.look}`, seed: hash(c.name) },
    }
    blocks.push(mk(layout, 'image', c.name, { x: i * 236, y: 0, width: 220, height: 303 }, content))
  })
  const width = Math.max(460, chars.length * 236 - 16)
  const lines = [
    `## 角色设定说明`,
    `风格：**${style}**。以下 ${chars.length} 张设定图由角色表逐行生成，修改表格中的外貌 / 主色 / 服装后重新运行即可刷新。`,
    ...chars.map((c) => `- **${c.name}**（${c.role}）：${c.look || '—'}${c.colorText ? `；主色 ${c.colorText}` : ''}${c.outfit ? `；${c.outfit}` : ''}`),
    `\n后续的故事板会引用这组设定图，以保持角色外观一致。`,
  ]
  blocks.push(mk(layout, 'text', '设定说明', { x: 0, y: 319, width, height: 150 }, { text: lines.join('\n'), format: 'markdown' }))
  return {
    blocks,
    warnings,
    assumptions: ['每个角色一张竖版设定图（480×600）', '主色取自"主色"列，缺失时按名字推断'],
    summary: `角色设定图 ×${chars.length} · ${style}`,
  }
}

/* ── storyboard ── */

const SHOT_CYCLE = ['远景', '中景', '特写', '全景', '中景', '近景', '远景', '中景']

export function parseScript(text: string, names: string[]): ShotSpec[] {
  const shots: ShotSpec[] = []
  const numbered = text.split('\n').map((l) => /^\s*(\d+)[.、．)]\s*(.+)$/.exec(l)).filter(Boolean) as RegExpExecArray[]
  const pushShot = (setting: string, action: string, explicit: string[]) => {
    const no = shots.length + 1
    const mentioned = names.filter((n) => action.includes(n) || setting.includes(n))
    const chars = explicit.length ? explicit : mentioned
    let shotType = SHOT_CYCLE[(no - 1) % SHOT_CYCLE.length]
    if (chars.length === 0 && (shotType === '特写' || shotType === '近景')) shotType = '全景'
    if (chars.length > 2 && shotType === '特写') shotType = '中景'
    shots.push({ no, setting: setting.trim() || `场景 ${no}`, action: action.trim(), chars, shotType, seconds: 4 + (hash(action) % 3) })
  }
  if (numbered.length) {
    for (const m of numbered) {
      let body = m[2].trim()
      let explicit: string[] = []
      const paren = /[（(]([^()（）]+)[)）]\s*$/.exec(body)
      if (paren) {
        explicit = paren[1].split(/[、,，/\s]+/).filter(Boolean)
        body = body.slice(0, paren.index).trim()
      }
      const sep = body.search(/[:：]/)
      const setting = sep >= 0 ? body.slice(0, sep) : ''
      const action = sep >= 0 ? body.slice(sep + 1) : body
      pushShot(setting, action, explicit)
    }
  } else {
    const sentences = text
      .split('\n')
      .map((l) => l.replace(/^#+\s*/, '').trim())
      .filter((l) => l && !/^一句话|^logline/i.test(l))
      .flatMap((l) => l.split(/[。！？!?]/))
      .map((s) => s.trim())
      .filter((s) => s.length > 4)
    for (const s of sentences.slice(0, 6)) pushShot('', s, [])
  }
  return shots.slice(0, 8)
}

function sceneKind(setting: string): 'dawn' | 'interior' | 'street' | 'bridge' | 'tower' | 'rooftop' | 'generic' {
  if (/日出|黎明|清晨|早晨/.test(setting)) return 'dawn'
  if (/面馆|餐|店|室内|屋内|房间|厨房/.test(setting)) return 'interior'
  if (/街|市|巷/.test(setting)) return 'street'
  if (/桥|高架|公路/.test(setting)) return 'bridge'
  if (/塔|顶层|云端/.test(setting)) return 'tower'
  if (/天台|楼顶|屋顶/.test(setting)) return 'rooftop'
  return 'generic'
}

export function storyboardFrame(shot: ShotSpec, chars: CharacterSpec[], style: string): string {
  const seed = hash(`${shot.no}|${shot.setting}|${shot.action}|${style}`)
  const r = rng(seed)
  const kind = sceneKind(shot.setting)
  const rain = /雨/.test(shot.setting) || /雨/.test(shot.action)
  const night = /夜|晚/.test(shot.setting)
  const palette: Record<typeof kind, [string, string, string]> = {
    dawn: ['#1e1b4b', '#f97316', '#fde68a'],
    interior: ['#3b2a1a', '#b45309', '#fbbf24'],
    street: ['#0f172a', '#312e81', '#22d3ee'],
    bridge: ['#111827', '#1f2937', '#a5b4fc'],
    tower: ['#0c1a3a', '#1d4ed8', '#e0f2fe'],
    rooftop: ['#0b1020', '#1e3a8a', '#93c5fd'],
    generic: ['#111827', '#334155', '#cbd5e1'],
  }
  const [c0, c1, acc] = palette[kind]
  const parts: string[] = []
  parts.push(`<defs><linearGradient id="sky" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="${c0}"/><stop offset="1" stop-color="${c1}"/></linearGradient></defs>`)
  parts.push(`<rect width="640" height="360" fill="url(#sky)"/>`)
  if (kind === 'dawn') parts.push(`<circle cx="${320 + Math.round(r() * 120 - 60)}" cy="230" r="46" fill="#fbbf24" opacity="0.9"/><rect x="0" y="250" width="640" height="110" fill="#1e1b4b" opacity="0.6"/>`)
  if (kind === 'interior') {
    for (let i = 0; i < 4; i++) {
      const x = 90 + i * 150
      parts.push(`<line x1="${x}" y1="0" x2="${x}" y2="70" stroke="#78350f" stroke-width="2"/><circle cx="${x}" cy="82" r="14" fill="#fbbf24" opacity="0.9"/><circle cx="${x}" cy="82" r="40" fill="#fbbf24" opacity="0.12"/>`)
    }
    parts.push(`<rect x="0" y="270" width="640" height="90" fill="#451a03" opacity="0.9"/>`)
  }
  if (kind === 'street') {
    for (let i = 0; i < 9; i++) {
      const x = Math.round(r() * 640)
      const h = Math.round(80 + r() * 180)
      const col = i % 2 ? '#22d3ee' : '#e879f9'
      parts.push(`<rect x="${x}" y="${300 - h}" width="${6 + Math.round(r() * 10)}" height="${h}" fill="${col}" opacity="0.7"/><rect x="${x - 10}" y="${300 - h}" width="${30}" height="${h}" fill="${col}" opacity="0.12"/>`)
    }
    parts.push(`<rect x="0" y="300" width="640" height="60" fill="#020617" opacity="0.8"/>`)
  }
  if (kind === 'bridge') {
    parts.push(`<polygon points="0,360 640,360 420,250 220,250" fill="#0f172a" opacity="0.9"/>`)
    for (let i = 0; i < 8; i++) parts.push(`<line x1="${i * 90}" y1="0" x2="320" y2="250" stroke="#94a3b8" stroke-width="1.5" opacity="0.4"/>`)
  }
  if (kind === 'tower') {
    parts.push(`<polygon points="320,20 350,300 290,300" fill="#0f172a" opacity="0.85"/><circle cx="320" cy="18" r="5" fill="#fca5a5"/>`)
    for (let i = 0; i < 5; i++) parts.push(`<ellipse cx="${Math.round(r() * 640)}" cy="${Math.round(200 + r() * 100)}" rx="${60 + Math.round(r() * 80)}" ry="16" fill="#e0f2fe" opacity="0.2"/>`)
  }
  if (kind === 'rooftop' || kind === 'generic' || kind === 'dawn') {
    for (let i = 0; i < 14; i++) {
      const w = 20 + Math.round(r() * 50)
      const h = 40 + Math.round(r() * 150)
      const x = Math.round(r() * 640)
      parts.push(`<rect x="${x}" y="${300 - h}" width="${w}" height="${h}" fill="#020617" opacity="${(0.5 + r() * 0.4).toFixed(2)}"/>`)
      if (night || kind !== 'dawn') for (let k = 0; k < 4; k++) parts.push(`<rect x="${x + 4 + Math.round(r() * (w - 10))}" y="${300 - h + 8 + Math.round(r() * (h - 16))}" width="4" height="6" fill="${acc}" opacity="0.7"/>`)
    }
    if (kind === 'rooftop') parts.push(`<rect x="0" y="296" width="640" height="64" fill="#0f172a"/><line x1="0" y1="296" x2="640" y2="296" stroke="${acc}" stroke-width="2" opacity="0.6"/>`)
    if (night && kind !== 'dawn') parts.push(`<circle cx="${560 + Math.round(r() * 40)}" cy="${50 + Math.round(r() * 30)}" r="22" fill="#fef3c7" opacity="0.85"/>`)
  }
  // characters
  const scale = shot.shotType === '远景' ? 0.55 : shot.shotType === '全景' ? 0.8 : shot.shotType === '中景' ? 1.15 : shot.shotType === '近景' ? 1.6 : 2.4
  const baseY = shot.shotType === '特写' ? 640 : shot.shotType === '近景' ? 470 : shot.shotType === '中景' ? 400 : 330
  const cast = shot.chars.map((n) => chars.find((c) => c.name === n) ?? { name: n, role: '', look: '', personality: '', colorText: '', outfit: '', color: '#94a3b8', accent: '#e2e8f0', robot: isRobot(n) })
  const shown = shot.shotType === '特写' ? cast.slice(0, 1) : cast
  shown.forEach((c, i) => {
    const x = Math.round((640 * (i + 1)) / (shown.length + 1))
    parts.push(figure(x, baseY, scale, c, hash(c.name)))
  })
  if (rain) {
    for (let i = 0; i < 70; i++) {
      const x = Math.round(r() * 640)
      const y = Math.round(r() * 360)
      parts.push(`<line x1="${x}" y1="${y}" x2="${x - 6}" y2="${y + 18}" stroke="#bae6fd" stroke-width="1" opacity="0.45"/>`)
    }
  }
  // cinematic chrome
  parts.push(`<rect x="0" y="0" width="640" height="24" fill="#000" opacity="0.75"/><rect x="0" y="336" width="640" height="24" fill="#000" opacity="0.75"/>`)
  parts.push(`<rect x="10" y="30" width="${118}" height="22" rx="11" fill="#000" opacity="0.55"/><text x="20" y="45" font-size="12" font-weight="700" fill="#fff" ${FONT}>S${String(shot.no).padStart(2, '0')} · ${esc(shot.shotType)} · ${shot.seconds}s</text>`)
  parts.push(`<text x="630" y="45" text-anchor="end" font-size="11" fill="#fff" opacity="0.7" ${FONT}>故事板 · Mock AI</text>`)
  const subtitle = shot.action.length > 38 ? `${shot.action.slice(0, 37)}…` : shot.action
  parts.push(`<rect x="0" y="300" width="640" height="36" fill="#000" opacity="0.45"/><text x="320" y="324" text-anchor="middle" font-size="14" fill="#fff" ${FONT}>${esc(subtitle)}</text>`)
  return dataUrl(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 640 360" width="640" height="360">${parts.join('')}</svg>`)
}

function generateStoryboard(req: AgentRunRequest, layout: Layout): MockResult {
  const items = flattenContext(req.context)
  const style = styleLabel(req)
  const charTable = items.find((i): i is TableItem => i.kind === 'table' && isCharacterTable(i))
  const images = items.filter((i): i is ImageItem => i.kind === 'image')
  const chars = charTable ? parseCharacters(charTable) : charactersFromImages(images)
  const script = items.find((i): i is Extract<AgentContextItem, { kind: 'text' }> => i.kind === 'text' && /剧本|大纲|故事|script/i.test(i.title)) ?? items.find((i): i is Extract<AgentContextItem, { kind: 'text' }> => i.kind === 'text' && !/风格|style/i.test(i.title))
  const warnings: string[] = []
  const assumptions = ['每个分场对应一个镜头，景别按 远景→中景→特写 循环', '时长 4–6 秒由动作描述推断']
  if (!script) warnings.push('未找到剧本文本，改用许愿格中的目标生成镜头')
  if (chars.length === 0) warnings.push('未找到角色设定，人物以占位剪影表示')
  const shots = parseScript(script?.text ?? req.wish.prompt, chars.map((c) => c.name))
  if (shots.length === 0) {
    return { blocks: [mk(layout, 'text', '说明', { x: 0, y: 0, width: 420, height: 140 }, { text: '剧本里没有可识别的分场。请用 "1. 场景：动作（角色）" 的格式写分场后重新运行。', format: 'markdown' })], warnings: [...warnings, '剧本没有分场'], assumptions, summary: '故事板：缺少分场' }
  }
  const blocks: CanvasBlock[] = []
  const cols = 3
  const fw = 288
  const fh = 162 + 28
  const gridWidth = cols * fw + (cols - 1) * 16
  const matrix: Array<Array<string | number>> = [['镜号', '场景', '景别', '出场角色', '动作描述', '时长(秒)']]
  for (const s of shots) matrix.push([`S${String(s.no).padStart(2, '0')}`, s.setting, s.shotType, s.chars.join('、') || '—', s.action, s.seconds])
  const tableHeight = 28 * (shots.length + 1) + 40
  blocks.push(mk(layout, 'table', '分镜表', { x: 0, y: 0, width: gridWidth, height: tableHeight }, tableContentFromMatrix(matrix, { hasHeader: true, source: { kind: 'manual' } })))
  shots.forEach((s, i) => {
    const content: ImageBlockContent = {
      src: storyboardFrame(s, chars, style),
      alt: s.action,
      caption: `${s.setting} · ${s.shotType}`,
      fit: 'cover',
      naturalWidth: 640,
      naturalHeight: 360,
      source: { kind: 'generated', prompt: `${style} 故事板 S${s.no}：${s.setting}，${s.action}`, seed: hash(s.action) },
    }
    blocks.push(mk(layout, 'image', `S${String(s.no).padStart(2, '0')} · ${s.shotType}`, { x: (i % cols) * (fw + 16), y: tableHeight + 20 + Math.floor(i / cols) * (fh + 16), width: fw, height: fh }, content))
  })
  const total = shots.reduce((n, s) => n + s.seconds, 0)
  return { blocks, warnings, assumptions, summary: `故事板 ${shots.length} 个镜头 · 约 ${total} 秒 · ${style}` }
}

/* ── video preview ── */

function generateVideo(req: AgentRunRequest, layout: Layout): MockResult {
  const items = flattenContext(req.context)
  const frames0 = items
    .filter((i): i is ImageItem => i.kind === 'image')
    .filter((im) => im.src)
    .sort((a, b) => a.title.localeCompare(b.title, 'zh-CN', { numeric: true }))
  const table = items.find((i): i is TableItem => i.kind === 'table' && Boolean(findCol(i.columns.map((c) => c.name), '时长', 'duration')))
  const durationCol = table ? findCol(table.columns.map((c) => c.name), '时长', 'duration') : undefined
  const actionCol = table ? findCol(table.columns.map((c) => c.name), '动作', '描述', 'action') : undefined
  const noCol = table ? findCol(table.columns.map((c) => c.name), '镜号', 'shot') : undefined
  const warnings: string[] = []
  if (frames0.length === 0) {
    return { blocks: [mk(layout, 'text', '说明', { x: 0, y: 0, width: 420, height: 140 }, { text: '数据来源里没有可用的画面。请先生成故事板，再把故事板结果组作为来源。', format: 'markdown' })], warnings: ['没有画面来源'], assumptions: [], summary: '视频：缺少画面' }
  }
  const frames: VideoFrame[] = frames0.map((im, i) => {
    const row = table?.rows.find((r) => noCol && String(r[noCol] ?? '') && im.title.startsWith(String(r[noCol]))) ?? table?.rows[i]
    const secs = row && durationCol ? Number(row[durationCol]) : NaN
    const caption = row && actionCol ? String(row[actionCol] ?? '') : im.alt ?? im.title
    return { src: im.src, durationMs: (Number.isFinite(secs) && secs > 0 ? secs : 4) * 1000, caption: `${im.title.split(' · ')[0]} · ${caption}` }
  })
  if (!table) warnings.push('未找到分镜表，每个镜头按 4 秒计')
  const total = frames.reduce((n, f) => n + f.durationMs, 0)
  const style = styleLabel(req)
  const video: VideoBlockContent = {
    frames,
    poster: frames[0].src,
    durationMs: total,
    width: 640,
    height: 360,
    caption: `逐帧预览 · ${frames.length} 个镜头 · ${(total / 1000).toFixed(0)} 秒`,
    source: { kind: 'generated', prompt: `${style} 短片合成：${frames.length} 个镜头`, seed: hash(req.wish.prompt) },
  }
  const blocks: CanvasBlock[] = [
    mk(layout, 'video', '最终视频（预览）', { x: 0, y: 0, width: 640, height: 452 }, video),
    mk(layout, 'metric', '总时长', { x: 656, y: 0, width: 230, height: 120 }, { label: '总时长', value: total / 1000, unit: '秒', tone: 'positive', note: `${frames.length} 个镜头` }),
    mk(layout, 'metric', '镜头数', { x: 656, y: 136, width: 230, height: 120 }, { label: '镜头数', value: frames.length, unit: '个', tone: 'neutral', note: '来自故事板' }),
    mk(layout, 'text', '导出说明', { x: 656, y: 272, width: 230, height: 180 }, {
      text: `## 成片说明\n- 分辨率 1280×720 · 16:9\n- 镜头顺序与时长来自分镜表\n- Mock Agent 只能输出逐帧预览；接入真实视频模型后，这里会是可下载的 MP4，其余流程不变。`,
      format: 'markdown',
    }),
  ]
  return { blocks, warnings, assumptions: ['镜头顺序按故事板编号', '时长取自分镜表"时长(秒)"列'], summary: `最终视频预览：${frames.length} 个镜头，${(total / 1000).toFixed(0)} 秒` }
}

export function generateAigc(kind: AigcKind, req: AgentRunRequest, layout: Layout): MockResult {
  if (kind === 'characters') return generateCharacters(req, layout)
  if (kind === 'storyboard') return generateStoryboard(req, layout)
  return generateVideo(req, layout)
}
