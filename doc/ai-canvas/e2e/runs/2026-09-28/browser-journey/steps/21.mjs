export default async env => {
  const {page,root,fs}=env
  env.exportUi=async (p,name) => {
    await p.getByRole('button',{name:'更多',exact:true}).first().click()
    const pending=p.waitForEvent('download')
    await p.getByRole('menuitem',{name:'导出 .aicanvas.json',exact:true}).click()
    await (await pending).saveAs(`${root}/evidence/${name}.aicanvas.json`)
  }
  await page.getByRole('button',{name:'取消',exact:true}).waitFor({state:'hidden',timeout:30000})
  await page.getByRole('button',{name:'适应全部内容 (F)',exact:true}).click()
  await env.exportUi(page,'after-refresh')
  await fs.writeFile(`${root}/evidence/refreshed-ui.txt`,await page.locator('body').ariaSnapshot())
  await page.getByRole('button',{name:'撤销 (Ctrl/⌘+Z)',exact:true}).click()
  await env.exportUi(page,'after-undo')
  await page.getByRole('button',{name:'重做 (Ctrl/⌘+Shift+Z)',exact:true}).click()
  await env.exportUi(page,'after-redo')
  const doc=JSON.parse(await fs.readFile(`${root}/evidence/after-redo.aicanvas.json`,'utf8'))
  return {exportKeys:Object.keys(doc)}
}