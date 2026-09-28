export default async ({page}) => {
  await page.getByRole('button',{name:'导入到画布',exact:true}).click()
  await page.getByRole('dialog',{name:'导入 Excel / CSV',exact:true}).waitFor({state:'hidden'})
}