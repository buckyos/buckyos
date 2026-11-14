/* AI Workspace text: the outline's path nodes — what the caller cannot read, drawn only as the folders on the way to
 * what it can (UI 框架 PRD §4.1.1; ui/surfaces/, ui/datamode/). English first; the Chinese table has the same keys. */

export const en: Record<string, string> = {
  'aiworkspace.pathnode.folder': 'Folder you have no access to',
  'aiworkspace.pathnode.dataFolder': 'Folder you have no access to',
  'aiworkspace.pathnode.canvas': 'Canvas you have no access to',
  'aiworkspace.pathnode.object': 'Object you have no access to',
  'aiworkspace.pathnode.hint': 'You cannot open this; only what is shared with you inside it is listed',
}

export const zh: Record<string, string> = {
  'aiworkspace.pathnode.folder': '无权访问的目录',
  'aiworkspace.pathnode.dataFolder': '无权访问的文件夹',
  'aiworkspace.pathnode.canvas': '无权访问的画布',
  'aiworkspace.pathnode.object': '无权访问的对象',
  'aiworkspace.pathnode.hint': '你没有权限打开它，这里只列出其中授权给你的内容',
}
