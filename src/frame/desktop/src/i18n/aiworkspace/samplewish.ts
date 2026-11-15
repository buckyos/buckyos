/* AI Workspace text: the "task summary" wish block the sample project template adds to its flow page (许愿格交互升级
 * §3.10 D, D10; api/sample.ts), written into the workspace in the language at creation time. The prompt quotes the task
 * table by its name in the sample (`{{tasks}}`): that is how the analysis finds the input. English first; the Chinese
 * table has the same keys. */
export const en: Record<string, string> = {
  'aiworkspace.samplewish.commit': 'Wish block: task summary',
  'aiworkspace.samplewish.wish.title': 'Task summary',
  'aiworkspace.samplewish.wish.prompt': 'Summarize “{{tasks}}”: count the tasks and total the budget by status, list the open tasks by owner, point out the tasks without a due date or an owner, and end with a progress note of at most three sentences.',
  'aiworkspace.samplewish.wish.output': 'Task summary results',
  'aiworkspace.samplewish.block.wish': 'Wish block: task summary',
}

export const zh: Record<string, string> = {
  'aiworkspace.samplewish.commit': '许愿格：任务汇总',
  'aiworkspace.samplewish.wish.title': '任务汇总',
  'aiworkspace.samplewish.wish.prompt': '汇总「{{tasks}}」：按状态统计任务数和预算合计，按负责人列出未完成的任务，指出缺少截止日期或负责人的任务，最后写一段不超过三句话的进展小结。',
  'aiworkspace.samplewish.wish.output': '任务汇总结果',
  'aiworkspace.samplewish.block.wish': '许愿格：任务汇总',
}
