export default async ({page}) => {
  await page.getByRole('button',{name:'更多',exact:true}).first().click()
}