export default async ({page,root,fs}) => {
  await page.getByRole('dialog',{name:'结果包含手工修改'}).getByRole('button',{name:'取消',exact:true}).click()
  await page.getByRole('button',{name:'更多',exact:true}).first().click()
  const pending=page.waitForEvent('download')
  await page.getByRole('menuitem',{name:'导出 .aicanvas.json',exact:true}).click()
  await (await pending).saveAs(`${root}/evidence/after-cancel.aicanvas.json`)
  await page.getByRole('button',{name:'重新运行',exact:true}).first().click()
  await page.getByRole('button',{name:'保留旧结果，生成新版本',exact:true}).click()
}