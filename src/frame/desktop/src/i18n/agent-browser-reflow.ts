/* Text of the Agent Browser's pages tidied one by one ("this page only"), shown in home mode. */

export const agentBrowserReflowEn: Record<string, string> = {
  'agentBrowser.reflow.refresh': 'Make this page again',
  'agentBrowser.reflow.frameTitle': 'Reflow page',
  'agentBrowser.reflow.madeAt': 'Made from the site as of {{time}}',
  'agentBrowser.reflow.madeByFallback': 'By {{name}}, as a one-off page',
  'agentBrowser.reflow.source': 'Original page',
  'agentBrowser.reflow.viewSource': 'View the original page',

  'agentBrowser.reflow.notice.agentAsked': '{{name}} was asked. The page updates here when it is done.',
  'agentBrowser.reflow.notice.agentUnreachable': '{{name}} could not be asked in the conversation right now.',

  'agentBrowser.reflow.making.title': 'Tidying this page',
  'agentBrowser.reflow.making.agent': '{{name}} was asked to make it. It shows here when it is done.',
  'agentBrowser.reflow.making.askAgain': 'Ask again',
  'agentBrowser.reflow.unmade.title': 'This page has not been tidied yet',
  'agentBrowser.reflow.unmade.agentUnreachable': '{{name}} could not be asked to make it: the conversation is not available here. Tell it the address in Chat, or try again.',
  'agentBrowser.reflow.askAgent': 'Ask {{name}}',
  'agentBrowser.reflow.unmade.notMade': '{{name}} can make a version of this one page that is easy to read on your phone. The website is not added to your reading.',
  'agentBrowser.reflow.unsupported.title': 'This address cannot be reflowed',
  'agentBrowser.reflow.unsupported.hint': 'Only web pages (http and https addresses) are reflowed.',
  'agentBrowser.reflow.failed.title': 'The page could not be looked up',
  'agentBrowser.reflow.failed.hint': 'Neither {{name}} nor your reflow library answered. Check your connection to the Zone, then try again.',
  'agentBrowser.reflow.busy.agent': '{{name}} is making this page again…',

  'agentBrowser.reflow.ask.redo': 'Please make the reflow page of this address again, from the site as it is now: {{url}}',
  'agentBrowser.reflow.ask.redoDone': 'Keep it in my reflow library when it is done (reflow result put).',
}

export const agentBrowserReflowZh: Record<string, string> = {
  'agentBrowser.reflow.refresh': '重新制作这个页面',
  'agentBrowser.reflow.frameTitle': '重排页',
  'agentBrowser.reflow.madeAt': '基于 {{time}} 的网站制作',
  'agentBrowser.reflow.madeByFallback': '由 {{name}} 单独制作',
  'agentBrowser.reflow.source': '原始页面',
  'agentBrowser.reflow.viewSource': '查看原始页面',

  'agentBrowser.reflow.notice.agentAsked': '已请 {{name}} 处理，做好后这里会更新。',
  'agentBrowser.reflow.notice.agentUnreachable': '现在没法在对话里联系 {{name}}。',

  'agentBrowser.reflow.making.title': '正在整理这一页',
  'agentBrowser.reflow.making.agent': '已请 {{name}} 制作，做好后会显示在这里。',
  'agentBrowser.reflow.making.askAgain': '再请一次',
  'agentBrowser.reflow.unmade.title': '这一页还没有整理',
  'agentBrowser.reflow.unmade.agentUnreachable': '没能请 {{name}} 来制作：这里的对话暂时不可用。可以到“对话”里把网址告诉它，或者重试。',
  'agentBrowser.reflow.askAgent': '请 {{name}} 处理',
  'agentBrowser.reflow.unmade.notMade': '{{name}} 可以把这一页整理成适合手机阅读的版本。这个网站不会加入你的阅读。',
  'agentBrowser.reflow.unsupported.title': '这个地址不能重排',
  'agentBrowser.reflow.unsupported.hint': '只有网页（http 和 https 的地址）可以重排。',
  'agentBrowser.reflow.failed.title': '查不到这一页',
  'agentBrowser.reflow.failed.hint': '{{name}} 和你的重排库都没有应答。请检查到 Zone 的连接，然后重试。',
  'agentBrowser.reflow.busy.agent': '{{name}} 正在重新制作这个页面…',

  'agentBrowser.reflow.ask.redo': '请按网站现在的内容，重新制作这个网址的重排页：{{url}}',
  'agentBrowser.reflow.ask.redoDone': '做好后放进我的重排库（reflow result put）。',
}
