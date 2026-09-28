export default async ({page}) => {
  await page.getByRole('button',{name:'Maximize',exact:true}).click()
  await page.getByRole('button',{name:/新建空白画布/}).waitFor()
}