export default async ({page}) => {
  await page.getByRole('textbox',{name:/在这里用一句话/}).fill('请基于全部三行，按产品汇总销售额，生成销售额总计、图表和汇报摘要。请使用销售额列，不使用数量或单价作为销售额。')
  await page.getByRole('button',{name:'文本',exact:true}).click()
}