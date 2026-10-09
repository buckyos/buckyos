/* Preset permission groups (phase two §6.3): capability sets interpreted as roles; the backend's actual set rules. */

import type { Capability } from '../../api/types'

export const PRESETS: { id: string; label: string; caps: Capability[] }[] = [
  { id: 'reader', label: '阅读者', caps: ['read'] },
  { id: 'commenter', label: '评论者', caps: ['read', 'comment'] },
  { id: 'editor', label: '编辑者', caps: ['read', 'append', 'update', 'delete', 'structure', 'comment'] },
  { id: 'manager', label: '管理者', caps: ['read', 'append', 'update', 'delete', 'structure', 'comment', 'export', 'manage'] },
]

export function presetOf(caps: Capability[]): string {
  const key = [...caps].sort().join(',')
  return PRESETS.find((p) => [...p.caps].sort().join(',') === key)?.label ?? '自定义'
}
