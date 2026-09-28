export default async ({page,fs,root}) => {
  await page.getByRole('textbox',{name:/支持 Markdown/}).fill('OUTSIDE_SOURCE_20260928：这段合成资料没有选为分析来源。')
  await page.getByRole('textbox',{name:/支持 Markdown/}).press('Escape')
  await page.getByRole('button',{name:'运行',exact:true}).click()
  await fs.writeFile(`${root}/evidence/running-ui.txt`,await page.locator('body').ariaSnapshot())
}