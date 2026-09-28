export default async ({page}) => {
  await page.getByRole('button',{name:'返回画布列表',exact:true}).waitFor({timeout:30000})
}