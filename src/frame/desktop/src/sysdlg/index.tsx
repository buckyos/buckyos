/* eslint-disable react-refresh/only-export-components */
import { useWindowDialog } from '../desktop/windows/dialogs'
import { useI18n } from '../i18n/provider'
import {
  AppInstaller,
  type AppInstallerLaunchParams,
} from './AppInstaller'

export {
  AppInstallerRoute,
  parseAppInstallerLaunchQuery,
  type AppInstallerLaunchErrorCode,
  type AppInstallerLaunchOptions,
  type AppInstallerLaunchParams,
} from './AppInstaller'

export const APP_INSTALLER_DIALOG_PATH = 'sysdlg/app_installer' as const

export type AppInstallerDialogParams = AppInstallerLaunchParams

export type AppInstallerDialogResult =
  | { action: 'background' }
  | { action: 'change-source' }
  | { action: 'close' }
  | { action: 'view-app'; serviceId: string }

export type SystemDialogPath = typeof APP_INSTALLER_DIALOG_PATH

export function useSystemDialog() {
  const windowDialog = useWindowDialog()
  const { t } = useI18n()

  const open = (
    path: SystemDialogPath,
    params: AppInstallerDialogParams,
  ): Promise<AppInstallerDialogResult | undefined> => {
    switch (path) {
      case APP_INSTALLER_DIALOG_PATH:
        return windowDialog.open<AppInstallerDialogResult>({
          ariaLabel: t('appService.install.systemInstaller', 'System App Installer'),
          presentation: 'modal',
          size: 'lg',
          dismissible: false,
          closeOnBackdrop: false,
          renderBody: ({ close }) => (
            <AppInstaller
              launchParams={params}
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
