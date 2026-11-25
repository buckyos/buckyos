import { useState } from 'react'
import { useI18n } from '../../../i18n/provider'
import { Sheet } from '../shell/Sheet'
import { useAgentBrowser } from '../store/context'
import { useSite, useSiteController } from './useSite'

/** An upgrade request of the site's maintenance Goal: what it is about, and the owner's answer. */
export function UpgradeSheet({ id, onClose }: { id: string; onClose: () => void }) {
  const controller = useSiteController()
  const { t } = useI18n()
  const agent = useAgentBrowser((snapshot) => snapshot.agent)
  const upgrade = useSite((snapshot) => snapshot.site?.upgrades.find((item) => item.id === id))
  const source = useSite((snapshot) => snapshot.site?.sources.find((item) => item.id === upgrade?.source))
  const maintainer = useSite((snapshot) => snapshot.site?.maintainer?.agent ?? null)
  const [busy, setBusy] = useState(false)
  if (!upgrade || !agent) return null
  const name = agent.name
  const mine = maintainer === null || maintainer === agent.agentDid
  const decide = async (decision: 'accepted' | 'dismissed') => {
    setBusy(true)
    await controller.decide(id, decision)
    setBusy(false)
  }
  return (
    <Sheet
      testId="ab-upgrade-sheet"
      title={t('agentBrowser.site.upgrade.title', 'Upgrade request')}
      onClose={onClose}
      footer={upgrade.state === 'open' ? <>
        <button type="button" className="ab-button" disabled={busy} data-testid="ab-upgrade-dismiss" onClick={() => void decide('dismissed')}>{t('agentBrowser.site.upgrade.dismiss', 'Not now')}</button>
        <button type="button" className="ab-button ab-button-primary" disabled={busy || !mine} data-testid="ab-upgrade-accept" onClick={() => void decide('accepted')}>{t('agentBrowser.site.upgrade.accept', 'Upgrade')}</button>
      </> : undefined}
    >
      <div className="ab-upgrade" data-testid="ab-upgrade" data-upgrade={upgrade.id} data-state={upgrade.state}>
        {source ? <p className="ab-upgrade-source">{source.title}</p> : null}
        <p className="ab-upgrade-title">{upgrade.title}</p>
        <p className="ab-upgrade-summary">{upgrade.summary}</p>
        <p className="ab-upgrade-hint">
          {upgrade.state !== 'open'
            ? t('agentBrowser.site.upgrade.decided', 'This request has been answered already.')
            : mine
              ? t('agentBrowser.site.upgrade.hint', '“Upgrade” asks {{name}} in the conversation to do it. What is published stays readable, and the new version takes over once it has been checked. With “Not now” nothing changes, and you are not asked again unless the problem changes.', { name })
              : t('agentBrowser.site.upgrade.otherMaintainer', 'Another Agent maintains this site. Open its browser to have it upgraded.')}
        </p>
      </div>
    </Sheet>
  )
}
