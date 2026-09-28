export default async env => {
  const {page,browser,root}=env
  await page.getByRole('button',{name:/S01-S03 协作交接试跑 20260928 1 个 Sheet/}).click()
  await page.getByRole('button',{name:'返回画布列表',exact:true}).waitFor()
  await env.exportUi(page,'author-reopened')
  env.authorPage=page
  env.receiverContext=await browser.newContext({viewport:{width:1600,height:1000},acceptDownloads:true})
  await env.receiverContext.tracing.start({screenshots:true,snapshots:true,sources:true})
  env.page=await env.receiverContext.newPage()
  env.page.setDefaultTimeout(15000)
  await env.page.goto('http://127.0.0.1:4179/?scenario=normal')
  await env.page.getByRole('button',{name:'AI Canvas',exact:true}).click()
  await env.page.getByRole('button',{name:'Maximize',exact:true}).click()
  await env.page.getByRole('heading',{name:'最近打开的画布',exact:true}).waitFor()
}