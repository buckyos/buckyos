export default async env => {
  await env.page.getByRole('button',{name:'适应全部内容 (F)',exact:true}).click()
  env.done=true
  return {authorAndReceiver:'separate browser contexts; file exchange only',completedAt:new Date().toISOString()}
}