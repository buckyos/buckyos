export default async ({page}) => {
  await page.getByRole('textbox').fill('S01-S03 协作交接试跑 20260928')
  await page.getByRole('textbox').press('Enter')
  await page.getByRole('button',{name:'导入 Excel / CSV',exact:true}).click()
}