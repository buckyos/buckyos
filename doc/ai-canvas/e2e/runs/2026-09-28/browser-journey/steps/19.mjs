export default async ({page,root}) => {
  const cell=page.getByRole('textbox',{name:'',exact:true})
  await cell.fill('90')
  await cell.press('Enter')
  await page.getByRole('button',{name:'更多',exact:true}).first().click()
  const pending=page.waitForEvent('download')
  await page.getByRole('menuitem',{name:'导出 .aicanvas.json',exact:true}).click()
  await (await pending).saveAs(`${root}/evidence/before-refresh.aicanvas.json`)
  await page.getByRole('button',{name:'重新运行',exact:true}).first().click()
}