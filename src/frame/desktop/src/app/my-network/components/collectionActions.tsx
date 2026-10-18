/* ── User Collection actions: a Collection exists while one contact carries its tag ── */

import { useMemo } from 'react'
import { useWindowDialog } from '../../../desktop/windows/dialogs'
import { useI18n } from '../../../i18n/provider'
import { networkActions } from '../../../components/network/cache'
import { useNetworkDialogs } from '../../../components/network/dialogs'
import { userTags } from '../../../components/network/relation'
import type { Contact } from '../../../components/network/types'
import { CollectionChooser, CollectionNameForm } from './CollectionForms'

const PERSON_KINDS = ['user', 'agent', 'msgtunnel', 'unknown'] as const

function existingTags(contacts: readonly Contact[]) {
  return [...new Set(contacts.flatMap(contact => userTags(contact.tags ?? [])))].sort()
}

export function useCollectionActions() {
  const dialog = useWindowDialog()
  const dialogs = useNetworkDialogs()
  const { t } = useI18n()

  return useMemo(() => {
    const askName = (title: string, existing: string[], submitLabel: string, initial?: string, hint?: string) =>
      dialog.open<string>({
        title,
        size: 'sm',
        renderBody: ({ close, dismiss }) => <CollectionNameForm initial={initial} existing={existing} submitLabel={submitLabel} hint={hint} onSubmit={close} onCancel={dismiss} />,
      })

    const addTo = async (tag: string, dids: readonly string[]) => {
      for (const did of dids) await networkActions.updateTags(did, { add: [tag] })
    }

    return {
      addTo,
      /** Name → members; returns the new tag, or undefined when nothing was created. */
      create: async (contacts: readonly Contact[]): Promise<string | undefined> => {
        const name = await askName(t('myNetwork.collection.create'), existingTags(contacts), t('myNetwork.collection.chooseMembers'), undefined, t('myNetwork.collection.createHint'))
        if (!name) return undefined
        const picked = await dialogs.pickPeople({
          title: t('myNetwork.collection.addTitle', undefined, { name }),
          confirmLabel: t('myNetwork.collection.createConfirm'),
          filter: { kinds: [...PERSON_KINDS] },
        })
        if (picked.action !== 'selected' || picked.people.length === 0) return undefined
        await addTo(name, picked.people.map(person => person.did))
        return name
      },
      addMembers: async (tag: string, members: readonly Contact[]) => {
        const picked = await dialogs.pickPeople({
          title: t('myNetwork.collection.addTitle', undefined, { name: tag }),
          confirmLabel: t('myNetwork.collection.addConfirm'),
          filter: { kinds: [...PERSON_KINDS], exclude: members.map(contact => ({ did: contact.did, reason: t('myNetwork.collection.alreadyIn') })) },
        })
        if (picked.action === 'selected') await addTo(tag, picked.people.map(person => person.did))
      },
      /** Renaming re-tags every member. */
      rename: async (tag: string, members: readonly Contact[], contacts: readonly Contact[]): Promise<string | undefined> => {
        const name = await askName(t('myNetwork.collection.renameTitle'), existingTags(contacts).filter(item => item !== tag), t('myNetwork.collection.rename'), tag)
        if (!name || name === tag) return undefined
        for (const contact of members) await networkActions.updateTags(contact.did, { add: [name], remove: [tag] })
        return name
      },
      /** Deleting removes the tag; the people stay contacts. */
      remove: async (tag: string, members: readonly Contact[]): Promise<boolean> => {
        const confirmed = await dialogs.confirm({
          strength: 'explicit',
          danger: true,
          title: t('myNetwork.collection.deleteTitle', undefined, { name: tag }),
          body: t('myNetwork.collection.deleteBody', undefined, { count: members.length }),
          confirmLabel: t('myNetwork.collection.delete'),
        })
        if (!confirmed) return false
        for (const contact of members) await networkActions.updateTags(contact.did, { remove: [tag] })
        return true
      },
      removeMany: async (tag: string, dids: readonly string[]): Promise<boolean> => {
        const confirmed = await dialogs.confirm({
          strength: 'light',
          title: t('network.confirm.removeFromCollection.title'),
          body: t('myNetwork.collection.removeManyBody', undefined, { count: dids.length, collection: tag }),
          confirmLabel: t('network.action.remove_from_collection'),
        })
        if (!confirmed) return false
        for (const did of dids) await networkActions.updateTags(did, { remove: [tag] })
        return true
      },
      /** Choose (or name) the user Collection a selection goes to. */
      choose: (contacts: readonly Contact[]) =>
        dialog.open<string>({
          title: t('myNetwork.list.addToCollection'),
          size: 'sm',
          renderBody: ({ close, dismiss }) => <CollectionChooser collections={existingTags(contacts)} onPick={close} onCancel={dismiss} />,
        }),
    }
  }, [dialog, dialogs, t])
}
