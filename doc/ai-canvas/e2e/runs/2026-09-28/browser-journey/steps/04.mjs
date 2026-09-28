export default async ({page}) => {
  await page.getByRole('button',{name:/新建空白画布/}).click()
}