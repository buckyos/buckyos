/* ── Shared non-component constants (kept out of component files for fast refresh) ── */

import { BarChart3, Clapperboard, Frame, Gauge, Image as ImageIcon, Sparkles, Table2, Type } from 'lucide-react'
import type { ReactNode } from 'react'
import type { CanvasBlock, WishState } from '../domain/types'
import type { Tone } from './primitives'

export const TYPE_ICON: Record<CanvasBlock['type'], ReactNode> = {
  text: <Type />,
  table: <Table2 />,
  wish: <Sparkles />,
  metric: <Gauge />,
  chart: <BarChart3 />,
  frame: <Frame />,
  group: <Sparkles />,
  interactive: <Frame />,
  image: <ImageIcon />,
  video: <Clapperboard />,
}

export const TYPE_LABEL: Record<CanvasBlock['type'], string> = {
  text: '文本',
  table: '表格',
  wish: '许愿格',
  metric: '指标',
  chart: '图表',
  frame: '框架',
  group: '结果组',
  interactive: '交互块',
  image: '图片',
  video: '视频',
}

export const STATUS_META: Record<'fresh' | 'stale' | 'broken' | 'never_run', { label: string; tone: Tone; glyph: string }> = {
  fresh: { label: '最新', tone: 'success', glyph: '●' },
  stale: { label: '需要刷新', tone: 'warning', glyph: '◐' },
  broken: { label: '来源中断', tone: 'danger', glyph: '✕' },
  never_run: { label: '尚未运行', tone: 'neutral', glyph: '○' },
}

export const WISH_STATE_LABEL: Record<WishState, string> = {
  idle: '待运行',
  planning: '正在理解目标',
  waiting_permission: '等待授权',
  running: '正在生成',
  validating: '正在校验结果',
  applying: '正在写入画布',
  succeeded: '已完成',
  failed: '运行失败',
  cancelled: '已取消',
}

export const RUNNING_STATES: WishState[] = ['planning', 'waiting_permission', 'running', 'validating', 'applying']

export function formatTime(iso?: string): string {
  if (!iso) return '—'
  const d = new Date(iso)
  const diff = Date.now() - d.getTime()
  if (diff < 60_000) return '刚刚'
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)} 分钟前`
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)} 小时前`
  return d.toLocaleString('zh-CN', { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' })
}
