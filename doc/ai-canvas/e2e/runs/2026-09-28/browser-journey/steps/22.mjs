export default async ({page}) => {
  await page.getByText('已保存到本机',{exact:true}).waitFor()
  await page.reload()
  await page.getByRole('button',{name:'AI Canvas',exact:true}).click()
  await page.getByRole('button',{name:'Maximize',exact:true}).click()
  await page.getByRole('heading',{name:'最近打开的画布',exact:true}).waitFor()
}