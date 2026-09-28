export default async ({page,root}) => {
  const picker = page.waitForEvent('filechooser')
  await page.getByRole('button',{name:'选择文件',exact:true}).click()
  await (await picker).setFiles(`${root}/fixtures/sales.csv`)
}