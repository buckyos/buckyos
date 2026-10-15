import type { useI18n } from '../../../i18n/provider'
import { PUBLIC_PRINCIPAL, ZONE_USERS_PRINCIPAL } from '../data/permissions'
import type { AccessLevel, PrincipalOption } from '../data/permissions'

export type Translate = ReturnType<typeof useI18n>['t']
export function levelLabel(t: Translate, level: AccessLevel): string {
  switch (level) {
    case 'manage': return t('filebrowser.permissions.level.manage', 'Full control')
    case 'write': return t('filebrowser.permissions.level.write', 'Read & write')
    case 'read': return t('filebrowser.permissions.level.read', 'Read')
    case 'traverse': return t('filebrowser.permissions.level.traverse', 'Pass-through only')
    default: return t('filebrowser.permissions.level.none', 'No access')
  }
}

export function principalLabel(t: Translate, principal: string, users: PrincipalOption[]): string {
  if (principal === PUBLIC_PRINCIPAL) return t('filebrowser.permissions.public', 'Public (anyone, no sign-in)')
  if (principal === ZONE_USERS_PRINCIPAL) return t('filebrowser.permissions.zoneUsers', 'All zone users')
  const user = users.find((item) => item.principal === principal)
  return user?.label ?? principal.replace(/^user:/, '')
}
