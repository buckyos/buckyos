export default async ({page}) => {
  await page.getByRole('button',{name:'未命名画布',exact:true}).click()
}