/* eslint-disable react-refresh/only-export-components */
import { useWindowDialog } from '../desktop/windows/dialogs'
import { useI18n } from '../i18n/provider'
import {
  AppInstaller,
  type AppInstallerInternalParams,
} from './AppInstaller'
import { ShareDialog, SHARE_DIALOG_PATH, type ShareDialogParams, type ShareDialogResult } from './ShareDialog'
import { EntityInfo } from '../components/network/EntityInfo'
import { entityInfoInputSchema, type EntityInfoInput, type EntityInfoResult } from '../components/network/schemas'

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

export type AppInstallerDialogParams = AppInstallerInternalParams

export type AppInstallerDialogResult =
  | { action: 'background' }
  | { action: 'change-source' }
  | { action: 'close' }
  | { action: 'view-app'; serviceId: string }

export type SystemDialogPath = typeof APP_INSTALLER_DIALOG_PATH | typeof SHARE_DIALOG_PATH | typeof ENTITY_INFO_DIALOG_PATH

export function useSystemDialog() {
  const windowDialog = useWindowDialog()
  const { t } = useI18n()

  function open(path: typeof APP_INSTALLER_DIALOG_PATH, params: AppInstallerDialogParams): Promise<AppInstallerDialogResult | undefined>
  function open(path: typeof SHARE_DIALOG_PATH, params: ShareDialogParams): Promise<ShareDialogResult | undefined>
  function open(path: typeof ENTITY_INFO_DIALOG_PATH, params: EntityInfoDialogParams): Promise<EntityInfoDialogResult>
  function open(
    path: SystemDialogPath,
    params: AppInstallerDialogParams | ShareDialogParams | EntityInfoDialogParams,
  ): Promise<AppInstallerDialogResult | ShareDialogResult | EntityInfoDialogResult | undefined> {
    switch (path) {
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
