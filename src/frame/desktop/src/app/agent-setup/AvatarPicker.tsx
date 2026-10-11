/* ── Avatar picker (ported from the OpenDAN WebUI profile dialog) ── */

import { useI18n } from '../../i18n/provider'
import { ImagePicker } from '../../presentation/ImagePicker'
import { AVATAR_IMAGE } from '../../presentation/imageFile'

export function AvatarPicker({
  name,
  avatar,
  fallbackSrc,
  onChange,
}: {
  name: string
  /** The chosen avatar (data URL); null keeps the template / default one. */
  avatar: string | null
  fallbackSrc?: string
  onChange: (avatar: string | null) => void
}) {
  const { t } = useI18n()
  return (
    <ImagePicker
      title={name}
      value={avatar}
      fallbackIcons={fallbackSrc ? [fallbackSrc] : []}
      options={AVATAR_IMAGE}
      labels={{
        choose: t('agentSetup.avatar.choose'),
        remove: t('agentSetup.avatar.remove'),
        usingDefault: t('agentSetup.avatar.default'),
        usingCustom: t('agentSetup.avatar.custom'),
      }}
      onChange={onChange}
      testId="agent-setup-avatar"
      inputTestId="agent-setup-avatar-input"
    />
  )
}
