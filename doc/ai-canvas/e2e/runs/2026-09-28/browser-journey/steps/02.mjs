export default async ({page}) => {
  await page.getByRole('button',{name:'AI Canvas',exact:true}).click()
}