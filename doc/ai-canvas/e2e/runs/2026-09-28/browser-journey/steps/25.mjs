export default async env => {
  const {page}=env
  await page.getByText('人工复核 HANDOFF_REVIEW_20260928：B 产品销售额待二次核实，请保留此说明。',{exact:true}).dblclick()
  const text=page.getByRole('textbox',{name:/支持 Markdown/})
  await text.fill((await text.inputValue())+'\n\n接收者复核 RECEIVER_REVIEW_20260928：已核对新结果销售额 140，旧说明继续保留。')
  await text.press('Escape')
  await page.getByText('已保存到本机',{exact:true}).waitFor()
  await env.exportUi(page,'receiver-edited')
  await page.reload()
  await page.getByRole('button',{name:'AI Canvas',exact:true}).click()
  await page.getByRole('button',{name:'Maximize',exact:true}).click()
  await page.getByRole('button',{name:/S01-S03 协作交接试跑 20260928 1 个 Sheet/}).click()
  await page.getByRole('button',{name:'返回画布列表',exact:true}).waitFor()
  await env.exportUi(page,'receiver-reopened')
  await env.exportUi(env.authorPage,'author-after-handoff')
}