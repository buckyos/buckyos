from pathlib import Path
import json, hashlib, shutil, subprocess
root=Path(__file__).resolve().parent
p=root/'evidence'
load=lambda name:json.loads((p/(name+'.aicanvas.json')).read_text())
d={name:load(name) for name in ['initial','before-refresh','after-cancel','after-refresh','after-undo','after-redo','author-reopened','receiver-imported','receiver-edited','receiver-reopened','author-after-handoff']}
checks=[]
def check(name,actual,expected):
 checks.append({'name':name,'actual':actual,'expected':expected,'pass':actual==expected})
def blocks(name,kind):return [b for b in d[name]['blocks'].values() if b['type']==kind]
def source(name):return next(b for b in blocks(name,'table') if b['title']=='sales')
def matrix(block):return [[r['cells'][c['id']]['value'] for c in block['content']['columns']] for r in block['content']['rows']]
def text(name):return '\n'.join(b['content']['text'] for b in blocks(name,'text'))
check('导入三行四列原值',matrix(source('initial')),[['A',2,10,20],['B',3,20,60],['C',1,30,30]])
check('首次销售额指标',[b['content']['value'] for b in blocks('initial','metric') if b['content']['label']=='本季度销售额'],[110])
check('Prompt 请求的图表至少一张',len(blocks('initial','chart'))>=1,True)
check('取消刷新前后整份文档一致',d['before-refresh']==d['after-cancel'],True)
old=d['before-refresh']; new=d['after-refresh']
check('新增块数和绑定数',[len(new['blocks'])-len(old['blocks']),len(new['bindings'])-len(old['bindings'])],[6,1])
check('旧结果和用户块保持原内容',all(b==new['blocks'].get(k) for k,b in old['blocks'].items() if b['type']!='wish'),True)
check('更新来源后两版本销售额',[b['content']['value'] for b in blocks('after-refresh','metric') if b['content']['label']=='本季度销售额'],[110,140])
check('保留人工说明',text('after-refresh').count('HANDOFF_REVIEW_20260928'),1)
def undo_business(doc):
 result=json.loads(json.dumps(doc))
 for b in result['blocks'].values():
  if b['type']=='wish':
   b['content'].pop('runHistory',None)
   b['content'].pop('lastRunId',None)
 for sheet in result['sheets']:sheet.pop('camera',None)
 return {k:result[k] for k in ['blocks','bindings','sheets','activeSheetId','presentationPaths','comments','revision']}
check('撤销移除本次生成内容和绑定并恢复既有业务对象',undo_business(d['after-undo'])==undo_business(old),True)
check('一次重做恢复生成后的完整文档',d['after-redo']==d['after-refresh'],True)
check('作者重开文档逐字段一致',d['after-redo']==d['author-reopened'],True)
changed=[k for k in d['author-reopened'] if d['author-reopened'][k]!=d['receiver-imported'].get(k)]
check('文件导入只有副本身份和元数据变化',changed,['id','updatedAt','metadata'])
check('接收者空白存储起点','还没有画布' in json.loads((p/'23.json').read_text())['ui'],True)
check('接收者新增复核意见且原说明保留',[text('receiver-edited').count('RECEIVER_REVIEW_20260928'),text('receiver-edited').count('HANDOFF_REVIEW_20260928')],[1,1])
check('接收者编辑保存重开逐字段一致',d['receiver-edited']==d['receiver-reopened'],True)
check('作者副本未被接收方修改',d['author-after-handoff']==d['author-reopened'],True)
check('来源只引用表格',blocks('initial','wish')[0]['content']['contextRefs'],[{'kind':'block','blockId':source('initial')['id'],'revision':0}])
summary={'checks':checks,'passed':sum(c['pass'] for c in checks),'failed':sum(not c['pass'] for c in checks),'undo_comparison_scope':{'excluded':['wish.content.runHistory','wish.content.lastRunId','sheet.camera'],'reason':'TC-009 判据是整组生成内容和绑定一次性移除及重做恢复；不要求删除运行历史或撤回当前导航。原始导出完整保留这些差异。'},'limitations':['请求选择来自导出 contextRefs，不是实际请求捕获，TA-C1 无法由此通过。','图表断言是本次用户目标，不是完整 PRD-21.1 的执行。','未验证图表数值、权限、真实协作、媒体和全部边界。']}
assert summary == json.loads((p/'verification.json').read_text()), 'Archived verification differs from recomputed facts'
print(json.dumps(summary,ensure_ascii=False,indent=2))
assert [c['name'] for c in checks if not c['pass']]==['Prompt 请求的图表至少一张']
