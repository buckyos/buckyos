/* ── RelationActions: add friend / allow messages / block / unblock / unfriend / delete contact / remove from collection ──
 *
 * The confirmation strength of each action is fixed here (PRD §11.3, TODO §3.6): removing from a
 * Collection is light, deleting a contact and blocking are explicit, unfriending is strong.
 */

import { Button } from '@mui/material'
import { Ban, Check, FolderMinus, MessageSquareMore, ShieldCheck, Trash2, UserMinus, UserPlus, X } from 'lucide-react'
import { useState, type ReactNode } from 'react'
import { useI18n } from '../../i18n/provider'
import { networkActions } from './cache'
import { useNetworkDialogs } from './dialogs'
import { StateMessage } from './Person'
import { personName, type PersonRef } from './personPresentation'
import { availableActions, networkErrorText, type RelationActionId, type RelationFacts } from './relation'
import { dangerOutlinedSx } from './buttonStyles'
import { useContactEditPermission } from './permissions'

const ICONS: Record<RelationActionId, ReactNode> = {
  add_friend: <UserPlus size={14} />,
  accept_request: <Check size={14} />,
  cancel_request: <X size={14} />,
  allow_messages: <MessageSquareMore size={14} />,
  block: <Ban size={14} />,
  unblock: <ShieldCheck size={14} />,
  unfriend: <UserMinus size={14} />,
  remove_contact: <Trash2 size={14} />,
  remove_from_collection: <FolderMinus size={14} />,
}

const DANGER = new Set<RelationActionId>(['block', 'unfriend', 'remove_contact'])

export interface RelationActionsProps {
  person: PersonRef
  relation: RelationFacts
  /** The user Collection this person is shown in; offers “Remove from collection”. */
  collectionTag?: string
  /** Hide these actions (the host shows its own entry for them). */
  exclude?: RelationActionId[]
  onDone?: (action: RelationActionId) => void
}

export function RelationActions({ person, relation, collectionTag, exclude = [], onDone }: RelationActionsProps) {
  const { t } = useI18n()
  const dialogs = useNetworkDialogs()
  const [pending, setPending] = useState<RelationActionId | null>(null)
  const [error, setError] = useState<string | null>(null)
  const name = personName(person)
  const canEditContacts = useContactEditPermission()
  const actions = canEditContacts
    ? availableActions(relation, { collectionTag, kind: person.kind }).filter(action => !exclude.includes(action))
    : []

  const confirmFor = async (action: RelationActionId): Promise<{ option: boolean } | null> => {
    switch (action) {
      case 'remove_from_collection':
        return dialogs.confirm({
          strength: 'light',
          title: t('network.confirm.removeFromCollection.title'),
          body: t('network.confirm.removeFromCollection.body', undefined, { name, collection: collectionTag ?? '' }),
          confirmLabel: t('network.action.remove_from_collection'),
        })
      case 'remove_contact':
        return dialogs.confirm({
          strength: 'explicit',
          danger: true,
          title: t('network.confirm.removeContact.title', undefined, { name }),
          body: t('network.confirm.removeContact.body', undefined, { name }),
          consequences: [t('network.confirm.removeContact.c1'), t('network.confirm.removeContact.c2'), t('network.confirm.removeContact.c3', undefined, { name })],
          confirmLabel: t('network.action.remove_contact'),
        })
      case 'unfriend':
        return dialogs.confirm({
          strength: 'strong',
          danger: true,
          title: t('network.confirm.unfriend.title', undefined, { name }),
          body: t('network.confirm.unfriend.body', undefined, { name }),
          consequences: [t('network.confirm.unfriend.c1', undefined, { name }), t('network.confirm.unfriend.c2'), t('network.confirm.unfriend.c3')],
          acknowledge: t('network.confirm.unfriend.acknowledge'),
          option: { label: t('network.confirm.unfriend.notify', undefined, { name }), initial: true },
          confirmLabel: t('network.action.unfriend'),
        })
      case 'block':
        return dialogs.confirm({
          strength: 'explicit',
          danger: true,
          title: t('network.confirm.block.title', undefined, { name }),
          body: t('network.confirm.block.body', undefined, { name }),
          consequences: [t('network.confirm.block.c1'), t('network.confirm.block.c2')],
          confirmLabel: t('network.action.block'),
        })
      default:
        return { option: false }
    }
  }

  const run = async (action: RelationActionId) => {
    setError(null)
    if (action === 'add_friend') {
      const result = await dialogs.addFriend({ did: person.did })
      if (result.action === 'done') onDone?.(action)
      return
    }
    const confirmed = await confirmFor(action)
    if (!confirmed) return
    setPending(action)
    try {
      switch (action) {
        case 'accept_request': await networkActions.respondFriendRequest(relation.pendingRequestId!, 'accept'); break
        case 'cancel_request': await networkActions.cancelFriendRequest(relation.pendingRequestId!); break
        case 'allow_messages': await networkActions.allowMessages(person.did, person.name); break
        case 'block': await networkActions.blockContact(person.did); break
        case 'unblock': await networkActions.unblockContact(person.did); break
        case 'unfriend': await networkActions.removeFriend(person.did, confirmed.option); break
        case 'remove_contact': await networkActions.removeContact(person.did); break
        case 'remove_from_collection': await networkActions.updateTags(person.did, { remove: [collectionTag!] }); break
      }
      onDone?.(action)
    } catch (failure) {
      setError(networkErrorText(t, failure))
    } finally {
      setPending(null)
    }
  }

  return (
    <div className="space-y-2" data-testid="relation-actions">
      <div className="flex flex-wrap gap-2">
        {actions.map(action => (
          <Button
            key={action}
            size="small"
            variant={action === 'add_friend' || action === 'accept_request' ? 'contained' : 'outlined'}
            sx={DANGER.has(action) ? dangerOutlinedSx : undefined}
            startIcon={ICONS[action]}
            disabled={pending !== null}
            onClick={() => void run(action)}
            data-action={action}
          >
            {t(`network.action.${action}`)}
          </Button>
        ))}
      </div>
      {error ? <StateMessage tone="danger" testId="relation-actions-error">{error}</StateMessage> : null}
    </div>
  )
}
