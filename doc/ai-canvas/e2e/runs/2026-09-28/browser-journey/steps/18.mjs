export default async ({page}) => {
  const box=page.getByRole('textbox',{name:/支持 Markdown/})
  await box.fill((await box.inputValue())+'\n\n人工复核 HANDOFF_REVIEW_20260928：B 产品销售额待二次核实，请保留此说明。')
  await box.press('Escape')
  const candidates=page.getByText('60',{exact:true})
  const count=await candidates.count()
  await candidates.first().dblclick()
  return {matchingValueCells:count}
}