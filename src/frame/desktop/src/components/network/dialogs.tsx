/* ── Opening the network components as window dialogs; every call resolves to its result (never rejects on dismiss) ──
 *
 * Works inside any WindowDialogProvider: Desktop windows, the mobile window sheet, and standalone
 * routes that mount their own provider (HomeStation portal). Narrow screens get a sheet.
 */

import { useMediaQuery } from '@mui/material'
import { useMemo } from 'react'
import { useWindowDialog } from '../../desktop/windows/dialogs'
import { useI18n } from '../../i18n/provider'
import { AddFriendDialog } from './AddFriendDialog'
import { ConfirmBody, type ConfirmOptions, type ConfirmResult } from './ConfirmBody'
import { FindPeople } from './FindPeople'
import { JoinGroupDialog } from './JoinGroupDialog'
import { PeoplePicker } from './PeoplePicker'
import { pickedPerson } from './personPresentation'
import {
  addFriendInputSchema,
  findPeopleInputSchema,
  joinGroupInputSchema,
  peoplePickerInputSchema,
  type AddFriendInput,
  type AddFriendResult,
  type FindPeopleInput,
  type FindPeopleResult,
  type JoinGroupInput,
  type JoinGroupResult,
  type PeoplePickerInput,
  type PeoplePickerResult,
} from './schemas'

export function useNetworkDialogs() {
  const dialog = useWindowDialog()
  const { t } = useI18n()
  const narrow = useMediaQuery('(max-width: 767px)')
  const presentation = narrow ? 'sheet' : 'modal'

  return useMemo(() => ({
    findPeople: async (input: FindPeopleInput = {}): Promise<FindPeopleResult> => {
      const params = findPeopleInputSchema.parse(input)
      const result = await dialog.open<FindPeopleResult>({
        title: params.title ?? t('network.find.title'),
        presentation,
        size: 'md',
        renderBody: ({ close }) => (
          <FindPeople
            initialInput={params.input}
            kinds={params.kinds}
            limit={params.limit}
            autoFocus
            actions={params.actions.map(action => ({
              id: action.id,
              label: action.label,
              onSelect: candidate => close({ action: 'invoke', actionId: action.id, person: pickedPerson(candidate) }),
            }))}
          />
        ),
      })
      return result ?? { action: 'cancelled' }
    },
    pickPeople: async (input: PeoplePickerInput = {}): Promise<PeoplePickerResult> => {
      const params = peoplePickerInputSchema.parse(input)
      const result = await dialog.open<PeoplePickerResult>({
        title: params.title ?? t(params.mode === 'single' ? 'network.picker.titleSingle' : 'network.picker.title'),
        presentation,
        size: 'md',
        renderBody: ({ close }) => <PeoplePicker params={params} onDone={close} />,
      })
      return result ?? { action: 'cancelled' }
    },
    addFriend: async (input: AddFriendInput = {}): Promise<AddFriendResult> => {
      const params = addFriendInputSchema.parse(input)
      const result = await dialog.open<AddFriendResult>({
        title: t('network.addFriend.title'),
        presentation,
        size: 'md',
        renderBody: ({ close }) => <AddFriendDialog params={params} onDone={close} />,
      })
      return result ?? { action: 'cancelled' }
    },
    joinGroup: async (input: JoinGroupInput = {}): Promise<JoinGroupResult> => {
      const params = joinGroupInputSchema.parse(input)
      const result = await dialog.open<JoinGroupResult>({
        title: t('network.joinGroup.title'),
        presentation,
        size: 'md',
        renderBody: ({ close }) => <JoinGroupDialog params={params} onDone={close} />,
      })
      return result ?? { action: 'cancelled' }
    },
    confirm: async (options: ConfirmOptions): Promise<ConfirmResult | null> => {
      const result = await dialog.open<ConfirmResult | null>({
        title: options.title,
        presentation,
        size: 'sm',
        renderBody: ({ close }) => <ConfirmBody options={options} onClose={close} />,
      })
      return result ?? null
    },
  }), [dialog, t, presentation])
}

export type NetworkDialogs = ReturnType<typeof useNetworkDialogs>
