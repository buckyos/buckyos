export default async ({page}) => {
  await page.getByRole('dialog',{name:'导入 Excel / CSV',exact:true}).getByText('正在后台解析 sales.csv…',{exact:true}).waitFor({state:'hidden'})
}