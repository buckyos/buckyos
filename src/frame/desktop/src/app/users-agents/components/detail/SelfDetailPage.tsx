/* ── Self (current user) detail page ── */

import { Chip } from '@mui/material'
import { useSelf, useUsersAgentsStore } from '../../hooks/use-users-agents-store'
import { HeaderSection } from '../sections/HeaderSection'
import { SocialAccountsSection } from '../sections/SocialAccountsSection'
import { InfoFieldsSection } from '../sections/InfoFieldsSection'
import { DIDDocumentSection } from '../sections/DIDDocumentSection'
import { SecuritySection } from '../sections/SecuritySection'

export function SelfDetailPage() {
  const self = useSelf()
  const store = useUsersAgentsStore()

  const handleAvatarEdit = () => {
    const nextUrl = window.prompt('Avatar image URL:', self.avatarUrl ?? '')
    if (nextUrl !== null) {
      store.updateSelfAvatar(nextUrl.trim() || undefined)
    }
  }

  const handleBioEdit = () => {
    const nextBio = window.prompt('Bio:', self.bio ?? '')
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
            <Chip label="Owner" size="small" color="primary" variant="outlined" />
            {self.twoFactorEnabled && (
              <Chip label="2FA" size="small" color="success" variant="outlined" />
            )}
          </>
        }
      />

      <InfoFieldsSection title="Profile" fields={self.info} onFieldChange={store.updateSelfInfo.bind(store)} />

      <SocialAccountsSection entityId={self.id} accounts={self.socialAccounts} />

      <InfoFieldsSection title="Settings" fields={self.settings} />

      <SecuritySection
        twoFactorEnabled={self.twoFactorEnabled}
        lastLogin={self.lastLogin}
      />

      <DIDDocumentSection document={self.didDocument} />
    </div>
  )
}
