export default async ({page}) => {
  await page.getByRole('button',{name:'基于此数据创建许愿格',exact:true}).click()
  await page.getByRole('button',{name:'适应全部内容 (F)',exact:true}).click()
}