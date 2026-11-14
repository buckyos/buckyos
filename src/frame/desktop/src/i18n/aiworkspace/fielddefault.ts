/* AI Workspace text: a table field's default value (UI 框架 PRD §7.3) — the field details, the new record's form, the
 * type change's pre-check. English first; the Chinese table has the same keys. */
export const en: Record<string, string> = {
  'aiworkspace.fielddefault.title': 'Default value',
  'aiworkspace.fielddefault.hint': 'A new record that leaves this field empty gets it, whoever adds the record. Existing records keep their values.',
  'aiworkspace.fielddefault.none': 'None',
  'aiworkspace.fielddefault.aria': 'Default value of “{{name}}”',
  'aiworkspace.fielddefault.clear': 'Remove',
  'aiworkspace.fielddefault.setEdit': 'Default of “{{name}}”: {{value}}',
  'aiworkspace.fielddefault.clearEdit': 'Remove the default of “{{name}}”',
  'aiworkspace.fielddefault.noneSubmitter': 'The service fills this field with whoever adds the record: it has no default.',
  'aiworkspace.fielddefault.noneUnique': 'A unique field has no default: every new record would repeat it.',
  'aiworkspace.fielddefault.noneRef': 'A reference field has no default.',
  'aiworkspace.fielddefault.emptyTakes': 'Left empty, it gets the default: {{value}}',
  'aiworkspace.fielddefault.migration': 'The default {{from}} becomes {{to}}.',
  'aiworkspace.fielddefault.migrationFails': 'The default {{from}} cannot be converted: the change is refused, or the default is removed when the cells that fail are cleared.',
  'aiworkspace.fielddefault.optionIsDefault': 'This option is the field\'s default: change the default first, or use “Delete & clear”',
}

export const zh: Record<string, string> = {
  'aiworkspace.fielddefault.title': '默认值',
  'aiworkspace.fielddefault.hint': '新建记录没有填这个字段时取这个值，不管是谁添加的。已有记录不变。',
  'aiworkspace.fielddefault.none': '无',
  'aiworkspace.fielddefault.aria': '“{{name}}”的默认值',
  'aiworkspace.fielddefault.clear': '清除',
  'aiworkspace.fielddefault.setEdit': '“{{name}}”的默认值改为 {{value}}',
  'aiworkspace.fielddefault.clearEdit': '清除“{{name}}”的默认值',
  'aiworkspace.fielddefault.noneSubmitter': '这个字段由服务填写添加记录的人，没有默认值。',
  'aiworkspace.fielddefault.noneUnique': '唯一字段没有默认值：每条新记录都会重复它。',
  'aiworkspace.fielddefault.noneRef': '引用字段没有默认值。',
  'aiworkspace.fielddefault.emptyTakes': '留空时取默认值：{{value}}',
  'aiworkspace.fielddefault.migration': '默认值 {{from}} 将变为 {{to}}。',
  'aiworkspace.fielddefault.migrationFails': '默认值 {{from}} 无法转换：迁移会被拒绝；选择“失败的单元格清空”时默认值会被去掉。',
  'aiworkspace.fielddefault.optionIsDefault': '这个选项是字段的默认值：先改默认值，或用“删并清空”',
}
