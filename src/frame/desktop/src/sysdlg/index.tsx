/* eslint-disable react-refresh/only-export-components */
import { useOptionalWindowDialog, useWindowDialog, type WindowDialogApi } from '../desktop/windows/dialogs'
import { useMemo } from 'react'
import { buckyos } from 'buckyos'
import { openSelectEmbed } from 'buckyos/app-frame'
import { useI18n } from '../i18n/provider'
import { isMockRuntime } from '../runtime'
import {
  AppInstaller,
  type AppInstallerInternalParams,
} from './AppInstaller'
import { ShareDialog, SHARE_DIALOG_PATH, type ShareDialogParams, type ShareDialogResult } from './ShareDialog'
import { EntityInfo } from '../components/network/EntityInfo'
import { entityInfoInputSchema, type EntityInfoInput, type EntityInfoResult } from '../components/network/schemas'
import { SELECT_DIALOG_PATH, type SelectDialogParams, type SelectDialogResult } from 'buckyos/content'
import { SelectDialog } from './select/SelectDialog'
import { parseSelectDialogParams } from './select/schemas'

export {
  AppInstallerRoute,
  parseAppInstallerLaunchQuery,
  type AppInstallerLaunchErrorCode,
  type AppInstallerLaunchOptions,
  type AppInstallerInternalParams,
} from './AppInstaller'

export const APP_INSTALLER_DIALOG_PATH = 'sysdlg/app_installer' as const
export { SHARE_DIALOG_PATH, type ShareDialogParams, type ShareDialogResult } from './ShareDialog'

/** The profile of a person, Agent or group by DID: who it is, the owner's relation with it, and the entry to message it. */
export const ENTITY_INFO_DIALOG_PATH = 'sysdlg/entity_info' as const
export type EntityInfoDialogParams = EntityInfoInput
export type EntityInfoDialogResult = EntityInfoResult

/** The system file picker: choose files, a folder, or where to save. Cancelling resolves with `{ action: 'cancelled' }`. */
export { SELECT_DIALOG_PATH, type SelectDialogParams, type SelectDialogResult }
export interface SelectDialogCaller {
  /** Keys the "last used folder". */
  id?: string
  /** Shown under the title when another App asked for the dialog; taken from the system's own app list. */
  name?: string
}

export type AppInstallerDialogParams = AppInstallerInternalParams

export type AppInstallerDialogResult =
  | { action: 'background' }
  | { action: 'change-source' }
  | { action: 'close' }
  | { action: 'view-app'; serviceId: string }

export type SystemDialogPath = typeof APP_INSTALLER_DIALOG_PATH | typeof SHARE_DIALOG_PATH | typeof ENTITY_INFO_DIALOG_PATH | typeof SELECT_DIALOG_PATH

type Translate = ReturnType<typeof useI18n>['t']

// Throws a ZodError for parameters outside the contract (`invalid_params` for an App caller).
function openSelectDialog(windowDialog: WindowDialogApi, t: Translate, params?: SelectDialogParams, caller?: SelectDialogCaller): Promise<SelectDialogResult> {
  const input = parseSelectDialogParams(params)
  const title = input.title ?? t(`sysdlg.select.title.${input.mode}`)
  const mobile = windowDialog.surface === 'mobile'
  return windowDialog.open<SelectDialogResult>({
    ariaLabel: title,
    title,
    description: caller?.name ? t('sysdlg.select.requestedBy', 'Requested by {{name}}', { name: caller.name }) : undefined,
    presentation: mobile ? 'sheet' : 'modal',
    size: 'lg',
    dismissible: true,
    closeOnBackdrop: false,
    renderBody: ({ close }) => <SelectDialog input={input} callerId={caller?.id} compact={mobile} onDone={close} />,
  }).then(result => result ?? { action: 'cancelled' })
}

/**
 * The Zone's system entry, which serves `/embed/sysdlg/select`. In the mock runtime the Zone is `localhost` (or
 * `127.0.0.1`) on the dev server's port, and a page on a sub-host of it (`aiws-<id>.localhost`) frames the parent.
 */
function systemOrigin(): string {
  return isMockRuntime()
    ? window.location.origin.replace(/^(https?:\/\/)[a-z0-9.-]+\.(localhost(?::\d+)?)$/, '$1$2')
    : `${window.location.protocol}//${buckyos.getZoneHostName()}`
}


/**
 * The system file picker for UI that also runs where there is no Desktop window (AI Workspace on a Workspace's own
 * origin): there it frames the system page over this page (path C), browsing with the Zone root's own session.
 */
export function useSelectContent(): (params?: SelectDialogParams, caller?: SelectDialogCaller) => Promise<SelectDialogResult> {
  const windowDialog = useOptionalWindowDialog()
  const { t, locale } = useI18n()
  return useMemo(() => windowDialog
    ? (params, caller) => openSelectDialog(windowDialog, t, params, caller)
    : (params) => openSelectEmbed(systemOrigin(), params ?? {}, { locale, theme: document.documentElement.dataset.theme === 'dark' ? 'dark' : 'light' }),
  [windowDialog, t, locale])
}

export function useSystemDialog() {
  const windowDialog = useWindowDialog()
  const { t } = useI18n()

  function open(path: typeof APP_INSTALLER_DIALOG_PATH, params: AppInstallerDialogParams): Promise<AppInstallerDialogResult | undefined>
  function open(path: typeof SHARE_DIALOG_PATH, params: ShareDialogParams): Promise<ShareDialogResult | undefined>
  function open(path: typeof ENTITY_INFO_DIALOG_PATH, params: EntityInfoDialogParams): Promise<EntityInfoDialogResult>
  function open(path: typeof SELECT_DIALOG_PATH, params?: SelectDialogParams, caller?: SelectDialogCaller): Promise<SelectDialogResult>
  function open(
    path: SystemDialogPath,
    params?: AppInstallerDialogParams | ShareDialogParams | EntityInfoDialogParams | SelectDialogParams,
    caller?: SelectDialogCaller,
  ): Promise<AppInstallerDialogResult | ShareDialogResult | EntityInfoDialogResult | SelectDialogResult | undefined> {
    switch (path) {
      case SELECT_DIALOG_PATH:
        return openSelectDialog(windowDialog, t, params as SelectDialogParams | undefined, caller)
      case ENTITY_INFO_DIALOG_PATH: {
        const input = entityInfoInputSchema.parse(params)
        return windowDialog.open<EntityInfoDialogResult>({
          ariaLabel: t('network.entityInfo.title', 'Profile'),
          title: t('network.entityInfo.title', 'Profile'),
          presentation: 'modal',
          size: 'sm',
          dismissible: true,
          closeOnBackdrop: true,
          renderBody: ({ close }) => <EntityInfo params={input} onMessage={input.canMessage ? person => close({ action: 'message', person }) : undefined} />,
        }).then(result => result ?? { action: 'cancelled' })
      }
      case SHARE_DIALOG_PATH:
        return windowDialog.open<ShareDialogResult>({
          ariaLabel: t('share.dialog.title', 'Share'),
          title: t('share.dialog.title', 'Share'),
          presentation: 'modal',
          size: 'md',
          dismissible: true,
          closeOnBackdrop: false,
          renderBody: ({ close }) => <ShareDialog params={params as ShareDialogParams} onDone={(result) => close(result)} />,
        })
      case APP_INSTALLER_DIALOG_PATH:
        return windowDialog.open<AppInstallerDialogResult>({
          ariaLabel: t('appService.install.systemInstaller', 'System App Installer'),
          presentation: 'modal',
          size: 'lg',
          dismissible: false,
          closeOnBackdrop: false,
          renderBody: ({ close }) => (
            <AppInstaller
              launchParams={params as AppInstallerDialogParams}
              onBackground={() => close({ action: 'background' })}
              onChangeSource={() => close({ action: 'change-source' })}
              onClose={() => close({ action: 'close' })}
              onViewApp={(serviceId) => close({ action: 'view-app', serviceId })}
            />
          ),
        })
    }
  }

  return { open }
}
