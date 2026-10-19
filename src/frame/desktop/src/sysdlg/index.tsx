/* eslint-disable react-refresh/only-export-components */
import { useWindowDialog } from '../desktop/windows/dialogs'
import { useI18n } from '../i18n/provider'
import {
  AppInstaller,
  type AppInstallerInternalParams,
} from './AppInstaller'
import { ShareDialog, SHARE_DIALOG_PATH, type ShareDialogParams, type ShareDialogResult } from './ShareDialog'

export {
  AppInstallerRoute,
  parseAppInstallerLaunchQuery,
  type AppInstallerLaunchErrorCode,
  type AppInstallerLaunchOptions,
  type AppInstallerInternalParams,
} from './AppInstaller'

export const APP_INSTALLER_DIALOG_PATH = 'sysdlg/app_installer' as const
export { SHARE_DIALOG_PATH, type ShareDialogParams, type ShareDialogResult } from './ShareDialog'

export type AppInstallerDialogParams = AppInstallerInternalParams

export type AppInstallerDialogResult =
  | { action: 'background' }
  | { action: 'change-source' }
  | { action: 'close' }
  | { action: 'view-app'; serviceId: string }

export type SystemDialogPath = typeof APP_INSTALLER_DIALOG_PATH | typeof SHARE_DIALOG_PATH

export function useSystemDialog() {
  const windowDialog = useWindowDialog()
  const { t } = useI18n()

  function open(path: typeof APP_INSTALLER_DIALOG_PATH, params: AppInstallerDialogParams): Promise<AppInstallerDialogResult | undefined>
  function open(path: typeof SHARE_DIALOG_PATH, params: ShareDialogParams): Promise<ShareDialogResult | undefined>
  function open(
    path: SystemDialogPath,
    params: AppInstallerDialogParams | ShareDialogParams,
  ): Promise<AppInstallerDialogResult | ShareDialogResult | undefined> {
    switch (path) {
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
