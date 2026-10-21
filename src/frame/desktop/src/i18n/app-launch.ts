/* Text a web App window shows while the desktop signs the user into the App. */

export const appLaunchEn: Record<string, string> = {
  'appLaunch.opening': 'Opening {{title}}…',
  'appLaunch.accessDenied': 'You do not have access to {{title}}.',
  'appLaunch.invalidTarget': 'Unable to open {{title}}.',
  'appLaunch.sessionInactive': 'Your sign-in could not be confirmed, so {{title}} was not opened.',
  'appLaunch.failed': '{{title}} could not be opened right now.',
}

export const appLaunchZh: Record<string, string> = {
  'appLaunch.opening': '正在打开 {{title}}…',
  'appLaunch.accessDenied': '你没有使用 {{title}} 的权限。',
  'appLaunch.invalidTarget': '无法打开 {{title}}。',
  'appLaunch.sessionInactive': '无法确认你的登录状态，{{title}} 没有打开。',
  'appLaunch.failed': '暂时无法打开 {{title}}。',
}
