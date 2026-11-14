/* AI Workspace text: the object tree's keyboard and "移动到…" with a place (UI 框架 PRD §5.2; ui/datamode/ObjectTree.tsx,
 * the move dialog of ui/datamode/dialogs.tsx). English first; the Chinese table has the same keys. */

export const en: Record<string, string> = {
  'aiworkspace.objecttree.move.folders': 'Choose a folder',
  'aiworkspace.objecttree.move.here': 'It is in here',
  'aiworkspace.objecttree.move.where': 'Where in “{{folder}}”?',
  'aiworkspace.objecttree.move.first': 'At the top',
  'aiworkspace.objecttree.move.after': 'After “{{name}}”',
  'aiworkspace.objecttree.move.current': 'Where it is now',
  'aiworkspace.objecttree.move.otherFolders': 'Other folders',
  'aiworkspace.objecttree.move.failed': 'Not moved: {{code}}',
}

export const zh: Record<string, string> = {
  'aiworkspace.objecttree.move.folders': '选择文件夹',
  'aiworkspace.objecttree.move.here': '当前所在',
  'aiworkspace.objecttree.move.where': '放在「{{folder}}」里的哪个位置？',
  'aiworkspace.objecttree.move.first': '最前面',
  'aiworkspace.objecttree.move.after': '「{{name}}」之后',
  'aiworkspace.objecttree.move.current': '当前位置',
  'aiworkspace.objecttree.move.otherFolders': '其他文件夹',
  'aiworkspace.objecttree.move.failed': '没有移动：{{code}}',
}
