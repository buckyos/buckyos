export default async ({page}) => {
  await page.getByRole('button',{name:'取消',exact:true}).waitFor({state:'hidden',timeout:30000})
  await page.getByRole('button',{name:'适应全部内容 (F)',exact:true}).click()
}