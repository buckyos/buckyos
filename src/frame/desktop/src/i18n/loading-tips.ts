/*
 * Tips shown under the desktop's loading indicator, one picked at random per load:
 * short how-tos for the desktop and a few lines on what BuckyOS is about.
 * Add a tip by adding the same key to both tables; `loadingTipKeys` follows the English one.
 */

export const loadingTipsEn: Record<string, string> = {
  'loadingTips.desktopFolder': 'Your desktop is a folder. Anything you put in Desktop in Files shows up here.',
  'loadingTips.allAppsSearch': 'In All apps, start typing to search and press Enter to open the first match.',
  'loadingTips.appMenu': 'Right-click an app in All apps to pin it to the taskbar or add it to the desktop.',
  'loadingTips.reorder': 'Drag the icons on the taskbar to put your apps in the order you like.',
  'loadingTips.showDesktop': 'The thin strip at the bottom of the taskbar shows the desktop. Click it again to bring your windows back.',
  'loadingTips.pinTaskbar': 'Right-click an empty part of the taskbar to keep it open.',
  'loadingTips.profile': 'Hover over your avatar in the top-left corner to see your profile or sign out.',
  'loadingTips.appearance': 'Wallpaper and appearance follow your account, so every device you sign in on looks the same.',
  'loadingTips.zone': 'BuckyOS brings your devices, apps, data, and AI agents together in one Zone: a personal cloud you control.',
  'loadingTips.dataStays': 'Your files, messages, and AI history live on your own devices, not on someone else\'s platform.',
  'loadingTips.did': 'Your identity is a DID that you own. Nobody can take your account away from you.',
  'loadingTips.agent': 'Jarvis is your built-in agent. It works with your files, messages, and tasks inside your own Zone.',
  'loadingTips.multiUser': 'A Zone can serve your whole family. Every user gets their own files, apps, and agents.',
  'loadingTips.openSource': 'BuckyOS is open source. Issues and pull requests are welcome on GitHub.',
}

export const loadingTipsZh: Record<string, string> = {
  'loadingTips.desktopFolder': '桌面就是一个文件夹：在“文件”里放进 Desktop 的东西，会直接出现在桌面上。',
  'loadingTips.allAppsSearch': '打开“全部应用”后直接输入即可搜索，按回车打开第一个结果。',
  'loadingTips.appMenu': '在“全部应用”里右键一个应用，可以把它固定到任务栏或添加到桌面。',
  'loadingTips.reorder': '拖动任务栏上的图标，可以按自己的习惯排列应用。',
  'loadingTips.showDesktop': '任务栏底部的细条可以一键显示桌面，再点一次就能恢复所有窗口。',
  'loadingTips.pinTaskbar': '在任务栏空白处右键，可以让任务栏一直保持展开。',
  'loadingTips.profile': '把鼠标停在左上角的头像上，可以查看个人资料或退出登录。',
  'loadingTips.appearance': '壁纸和外观设置跟着账号走，在任何设备上登录看到的都是同一个桌面。',
  'loadingTips.zone': 'BuckyOS 把你的设备、应用、数据和 AI Agent 整合进同一个 Zone，构成由你掌控的个人云。',
  'loadingTips.dataStays': '你的文件、消息和 AI 记录都保存在你自己的设备上，而不是别人的平台里。',
  'loadingTips.did': '你的身份是一个属于你自己的 DID，没有人能收走你的账号。',
  'loadingTips.agent': 'Jarvis 是内置的 AI Agent，它在你自己的 Zone 里处理你的文件、消息和任务。',
  'loadingTips.multiUser': '一个 Zone 可以服务全家人，每位用户都有各自独立的文件、应用和 Agent。',
  'loadingTips.openSource': 'BuckyOS 是开源的，欢迎在 GitHub 上提交 Issue 和 Pull Request。',
}

export const loadingTipKeys = Object.keys(loadingTipsEn)
