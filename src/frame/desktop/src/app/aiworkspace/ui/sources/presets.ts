/* Preset permission groups (phase two §6.3): capability sets interpreted as roles; the backend's actual set rules. */

import type { Capability } from '../../api/types'
import { aiwsT } from '../../i18n'

export const PRESETS: { id: string; label: string; caps: Capability[] }[] = [
  { id: 'reader', get label() { return aiwsT('aiworkspace.data.perm.reader') }, caps: ['read'] },
  { id: 'commenter', get label() { return aiwsT('aiworkspace.data.perm.commenter') }, caps: ['read', 'comment'] },
  { id: 'editor', get label() { return aiwsT('aiworkspace.data.perm.editor') }, caps: ['read', 'append', 'update', 'delete', 'structure', 'comment'] },
  { id: 'manager', get label() { return aiwsT('aiworkspace.data.perm.manager') }, caps: ['read', 'append', 'update', 'delete', 'structure', 'comment', 'export', 'manage', 'run'] },
]

export function presetOf(caps: Capability[]): string {
  const key = [...caps].sort().join(',')
  return PRESETS.find((p) => [...p.caps].sort().join(',') === key)?.label ?? aiwsT('aiworkspace.data.perm.custom')
}
