import { X } from 'lucide-react'
import { useI18n } from '../../i18n/provider'
import { EntityInfo } from '../../components/network/EntityInfo'
import type { EntityInfoParams, PickedPerson } from '../../components/network/schemas'

/** The profile of a conversation participant as a details page: the touch layout's form of the entity info dialog. */
export function ProfileDetails({ params, onMessage, onClose }: { params: EntityInfoParams; onMessage?: (person: PickedPerson) => void; onClose: () => void }) {
  const { t } = useI18n()
  return <section className="flex h-full flex-col bg-[color:var(--cp-surface)]" data-testid="profile-details">
    <header className="flex shrink-0 items-center justify-between border-b border-[color:var(--cp-border)] py-1 pl-4 pr-1">
      <h2 className="text-sm font-semibold">{t('network.entityInfo.title')}</h2>
      <button type="button" className="flex min-h-11 min-w-11 items-center justify-center" aria-label={t('messagehub.close')} title={t('messagehub.close')} onClick={onClose}><X size={18} /></button>
    </header>
    <div className="shell-scrollbar flex-1 overflow-y-auto p-4"><EntityInfo params={params} onMessage={onMessage} /></div>
  </section>
}
