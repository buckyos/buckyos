export default async ({page,root,fs}) => {
  const pending=page.waitForEvent('download')
  await page.getByRole('menuitem',{name:'导出 .aicanvas.json',exact:true}).click()
  await (await pending).saveAs(`${root}/evidence/initial.aicanvas.json`)
  await page.getByRole('heading',{name:'本季度经营总结',exact:true}).dblclick()
}