/* ── Self (current user) detail page ── */

import { Chip } from '@mui/material'
import { useSelf } from '../../hooks/use-users-agents-store'
import { HeaderSection } from '../sections/HeaderSection'
import { BindingsSection } from '../sections/BindingsSection'
import { InfoFieldsSection } from '../sections/InfoFieldsSection'
import { DIDDocumentSection } from '../sections/DIDDocumentSection'
import { SecuritySection } from '../sections/SecuritySection'

export function SelfDetailPage() {
  const self = useSelf()

  return (
    <div className="space-y-4">
      <HeaderSection
        name={self.displayName}
        kind="self"
        avatarUrl={self.avatarUrl}
        did={self.did}
        subtitle={self.bio}
        isOnline
        badges={
          <>
            <Chip label="Owner" size="small" color="primary" variant="outlined" />
            {self.twoFactorEnabled && (
              <Chip label="2FA" size="small" color="success" variant="outlined" />
            )}
          </>
        }
      />

      <BindingsSection bindings={self.bindings} />

      <InfoFieldsSection title="Public Info" fields={self.info} />

      <DIDDocumentSection document={self.didDocument} />

      <SecuritySection
        twoFactorEnabled={self.twoFactorEnabled}
        lastLogin={self.lastLogin}
      />
    </div>
  )
}
