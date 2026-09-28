export default async env => {
  const {page,root}=env
  const pending=page.waitForEvent('filechooser')
  await page.getByRole('button',{name:/导入 .aicanvas.json 恢复/}).click()
  await (await pending).setFiles(`${root}/evidence/author-reopened.aicanvas.json`)
  await page.getByRole('button',{name:'返回画布列表',exact:true}).waitFor()
  await env.exportUi(page,'receiver-imported')
}