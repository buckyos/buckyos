/* ── Self (current user) detail page ── */

import { Chip } from '@mui/material'
import { useSelf, useUsersAgentsStore } from '../../hooks/use-users-agents-store'
import { HeaderSection } from '../sections/HeaderSection'
import { SocialAccountsSection } from '../sections/SocialAccountsSection'
import { InfoFieldsSection } from '../sections/InfoFieldsSection'
import { DIDDocumentSection } from '../sections/DIDDocumentSection'
import { SecuritySection } from '../sections/SecuritySection'
import { useI18n } from '../../../../i18n/provider'

export function SelfDetailPage() {
  const self = useSelf()
  const store = useUsersAgentsStore()
  const { t } = useI18n()

  const handleAvatarEdit = () => {
    const nextUrl = window.prompt(t('usersAgents.self.avatarPrompt'), self.avatarUrl ?? '')
    if (nextUrl !== null) {
      store.updateSelfAvatar(nextUrl.trim() || undefined)
    }
  }

  const handleBioEdit = () => {
    const nextBio = window.prompt(t('usersAgents.self.bioPrompt'), self.bio ?? '')
    if (nextBio !== null) {
      store.updateSelfBio(nextBio.trim())
    }
  }

  return (
    <div className="space-y-4">
      <HeaderSection
        name={self.displayName}
        kind="self"
        avatarUrl={self.avatarUrl}
        did={self.did}
        subtitle={self.bio}
        isOnline
        previewUrl={`/profile/${encodeURIComponent(self.did ?? self.id)}`}
        onAvatarEdit={handleAvatarEdit}
        onSubtitleEdit={handleBioEdit}
        badges={
          <>
            <Chip label={t('usersAgents.role.owner')} size="small" color="primary" variant="outlined" />
            {self.twoFactorEnabled && (
              <Chip label="2FA" size="small" color="success" variant="outlined" />
            )}
          </>
        }
      />

      <InfoFieldsSection title={t('usersAgents.section.profile')} fields={self.info} onFieldChange={store.updateSelfInfo.bind(store)} />

      <SocialAccountsSection entityId={self.id} accounts={self.socialAccounts} ownProfile />

      <InfoFieldsSection title={t('usersAgents.section.settings')} fields={self.settings} />

      <SecuritySection
        twoFactorEnabled={self.twoFactorEnabled}
        lastLogin={self.lastLogin}
      />

      <DIDDocumentSection document={self.didDocument} />
    </div>
  )
}
