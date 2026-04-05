import {
  InMemoryConversationMessageReader,
  IndexedDbConversationMessageReader,
} from '../messagehub/conversation/history/data-source'
import type { AppendableConversationMessageReader } from '../messagehub/conversation/history/types'
import type {
  ConversationStatusType,
  MessageDeliveryStatus,
  MessageObject,
  MsgContent,
} from '../messagehub/protocol/msgobj'
import {
  getMockEntityDid,
  MOCK_SELF_DID,
  mockSessions,
} from '../messagehub/mock/data'

const codeAssistantEntityId = 'agent-coder'
const storageNamespace = 'buckyos.mock.codeassistant.history'
const databaseName = 'buckyos-mock-message-history'
const storageVersion = 'v4'
const defaultPageSize = 40
const trustedImageUri = 'https://upload.wikimedia.org/wikipedia/commons/9/95/Museo_di_Santa_Giulia_Coro_delle_Monache_Deposizione_Paolo_da_Caylina_Brescia.jpg'

const writerDid = MOCK_SELF_DID
const assistantDid = getMockEntityDid(codeAssistantEntityId)

const userPrompts = [
  '继续把剩余的中间件也统一到新鉴权流程里。',
  '把这个模块拆成更清晰的 service / adapter / types 三层。',
  '顺手把类型定义也收一下，尽量别再有 any。',
  '这个接口的错误处理还不够一致，再补一下。',
  '看一下这里是否值得加缓存，避免重复请求。',
  '把测试补齐，尤其是失败路径和回滚路径。',
  '日志有点乱，整理成结构化输出。',
  '我还想看一个更适合 review 的 diff summary。',
  '这个 PR 描述帮我按风险和收益重写一版。',
  '把这个改动对应的文档一起更新掉。',
]

const assistantReplies = [
  '我先沿着调用链把依赖收拢，再决定拆分边界。这样可以避免为了分层而分层。',
  '这里我会先补一版最小可行重构，再看是否需要扩大影响面。先稳住行为，再谈抽象。',
  '我已经定位到几个重复逻辑点，适合抽成共享 helper，并保留清晰的回退路径。',
  '这段代码的真正问题不是复杂，而是职责混杂。我会先把读取、转换、提交这三段切开。',
  '测试我会按成功、权限不足、超时、服务异常四类补，避免只覆盖 happy path。',
  '这个模块现在更像是“能跑”，不是“好维护”。我会顺手整理命名和边界，减少后续修改成本。',
  '我会把输出整理成 review 友好的小步提交思路，方便你评估是否值得继续推进。',
]

const longBlocks = [
  [
    '我先给出一个分步计划：',
    '1. 识别协议层字段与 UI 映射的重复点',
    '2. 提取只读数据访问接口，避免组件直接接触完整数组',
    '3. 在渲染层引入虚拟列表并保留日期/状态条目扩展点',
    '4. 用现有 mock 数据验证首屏、回到底部、长文本、状态消息几种路径',
  ].join('\n'),
  [
    '这一轮改动我会控制风险：',
    '- 先不动协议语义，只动展示层接线',
    '- 保持消息对象字段名和后端 serde 输出一致',
    '- 所有 UI 补充信息都挂到 meta 风格字段里',
    '- 如果虚拟列表和动态高度冲突，优先保证可读性，再做性能细化',
  ].join('\n'),
  [
    '当前观察到的几个风险点：',
    '- 旧组件默认假设可以随机访问整段消息数组',
    '- 日期分隔是在 render 时即时插入，不利于统一索引',
    '- 长文本和状态条目高度差异大，必须走真实测量',
    '- 如果只做“可见区域刚好渲染”，快速滚动时会有明显空白',
  ].join('\n'),
]

const diagnosticBlocks = [
  [
    '补充一下更细的诊断摘录：',
    '- auth middleware 在进入路由层之前已经做过一次 header 解析',
    '- refresh token 的失败路径会把上游错误语义抹平',
    '- tracing span 里混入了 UI 级字段，后续不利于聚合',
    '- 旧 helper 默认依赖一个可变全局对象，这会让测试隔离变差',
  ].join('\n'),
  [
    '这一段是我准备放进 review 描述里的上下文：',
    '本次改动不是单纯替换函数名，而是把原本分散在 transport、adapter、handler 三层的鉴权判断统一到一个入口。',
    '这样做的收益是行为一致，但风险是历史调用链里可能还残留绕过入口的旧路径，所以回归测试一定要覆盖。',
  ].join('\n'),
  [
    '附一份更长的实现说明：',
    '```',
    'load request context',
    '  -> resolve actor',
    '  -> validate token',
    '  -> normalize auth state',
    '  -> dispatch to route handler',
    '  -> map domain error to protocol error',
    '```',
    '这里我会保留原有 fallback，但把决策点收敛到同一个模块里，避免多处分叉。',
  ].join('\n'),
]

const userImageCaptions = [
  '我把需要对照的参考图贴在这里，你顺便确认一下图片消息在长历史里滚动时是否稳定。',
  '这张图先挂进上下文里，后面如果要继续做图文混排，可以直接沿用同一套 refs 结构。',
  '这里插一条图片消息，主要是为了压一下虚拟列表和图片高度测量这条路径。',
]

const assistantImageCaptions = [
  '我把参考图重新贴一遍，方便你直接在当前线程里验证图片消息的渲染和滚动表现。',
  '这一条改成图片消息，主要用于观察长列表里图片卡片和普通文本卡片混排时的性能。',
  '补一条带可信域 uri_hint 的图片样本，这样可以直接覆盖自动预览路径。',
]

export async function createCodeAssistantMockReaders(): Promise<Record<string, AppendableConversationMessageReader>> {
  const seeds = buildCodeAssistantSessionSeeds()

  if (typeof window === 'undefined') {
    return Object.fromEntries(
      Object.entries(seeds).map(([sessionId, messages]) => [
        sessionId,
        InMemoryConversationMessageReader.fromMessages(
          messages,
          defaultPageSize,
          `memory:${sessionId}`,
        ),
      ]),
    ) as Record<string, AppendableConversationMessageReader>
  }

  const entries = await Promise.all(
    Object.entries(seeds).map(async ([sessionId, messages]) => [
      sessionId,
      await IndexedDbConversationMessageReader.seed({
        databaseName,
        namespace: storageNamespace,
        sessionId,
        messages,
        pageSize: defaultPageSize,
        version: storageVersion,
      }),
    ] as const),
  )

  return Object.fromEntries(entries) as Record<string, AppendableConversationMessageReader>
}

function buildCodeAssistantSessionSeeds(): Record<string, readonly MessageObject[]> {
  return {
    'session-coder-1': buildThread({
      sessionId: 'session-coder-1',
      topic: 'Auth Module Refactor',
      iterationCount: 900,
      startedAtMs: Date.now() - 160 * 24 * 3600_000,
      includeStatus: true,
    }),
    'session-coder-2': buildThread({
      sessionId: 'session-coder-2',
      topic: 'API Documentation',
      iterationCount: 420,
      startedAtMs: Date.now() - 110 * 24 * 3600_000,
      includeStatus: false,
    }),
    'session-coder-3': buildThread({
      sessionId: 'session-coder-3',
      topic: 'Bug Investigation #142',
      iterationCount: 560,
      startedAtMs: Date.now() - 84 * 24 * 3600_000,
      includeStatus: true,
    }),
  }
}

function buildThread({
  sessionId,
  topic,
  iterationCount,
  startedAtMs,
  includeStatus,
}: {
  sessionId: string
  topic: string
  iterationCount: number
  startedAtMs: number
  includeStatus: boolean
}) {
  const messages: MessageObject[] = []
  let currentAtMs = startedAtMs

  messages.push(createChatMessage({
    id: `${sessionId}:intro:user`,
    from: writerDid,
    to: [assistantDid],
    senderName: 'You',
    content: `先从 ${topic} 开始。给我一个可执行的重构路径，不要只讲抽象。`,
    createdAtMs: currentAtMs,
    deliveryStatus: 'read',
    sessionId,
  }))
  currentAtMs += 6 * 60_000

  messages.push(createChatMessage({
    id: `${sessionId}:intro:assistant`,
    from: assistantDid,
    to: [writerDid],
    senderName: 'CodeAssistant',
    content: `收到。我会先做一次现状梳理，然后把 ${topic} 拆成几段可以独立验证的小改动。`,
    createdAtMs: currentAtMs,
    sessionId,
  }))

  for (let index = 0; index < iterationCount; index += 1) {
    currentAtMs += computeGapMs(index)

    const userContent = buildUserPrompt(topic, index)
    messages.push(
      shouldUseImageMessage(index, 'user')
        ? createImageMessage({
          id: `${sessionId}:u:image:${index}`,
          from: writerDid,
          to: [assistantDid],
          senderName: 'You',
          content: buildImageCaption({
            topic,
            index,
            sender: 'user',
            fallback: userContent,
          }),
          createdAtMs: currentAtMs,
          deliveryStatus: 'read',
          sessionId,
        })
        : createChatMessage({
          id: `${sessionId}:u:${index}`,
          from: writerDid,
          to: [assistantDid],
          senderName: 'You',
          content: userContent,
          createdAtMs: currentAtMs,
          deliveryStatus: 'read',
          sessionId,
        }),
    )

    if (includeStatus && index % 5 === 2) {
      currentAtMs += 45_000
      messages.push(createStatusMessage({
        id: `${sessionId}:status:lead:${index}`,
        from: assistantDid,
        to: [writerDid],
        senderName: 'CodeAssistant',
        label: pickStatusLabel(index),
        statusType: pickStatusType(index),
        createdAtMs: currentAtMs,
        sessionId,
      }))
    }

    currentAtMs += 4 * 60_000

    const assistantContent = buildAssistantReply(topic, index)
    messages.push(
      shouldUseImageMessage(index, 'assistant')
        ? createImageMessage({
          id: `${sessionId}:a:image:${index}`,
          from: assistantDid,
          to: [writerDid],
          senderName: 'CodeAssistant',
          content: buildImageCaption({
            topic,
            index,
            sender: 'assistant',
            fallback: assistantContent,
          }),
          createdAtMs: currentAtMs,
          deliveryStatus: index > iterationCount - 4 ? 'delivered' : undefined,
          sessionId,
        })
        : createChatMessage({
          id: `${sessionId}:a:${index}`,
          from: assistantDid,
          to: [writerDid],
          senderName: 'CodeAssistant',
          content: assistantContent,
          createdAtMs: currentAtMs,
          deliveryStatus: index > iterationCount - 4 ? 'delivered' : undefined,
          sessionId,
        }),
    )

    if (includeStatus && index % 9 === 4) {
      currentAtMs += 90_000
      messages.push(createStatusMessage({
        id: `${sessionId}:status:${index}`,
        from: assistantDid,
        to: [writerDid],
        senderName: 'CodeAssistant',
        label: pickStatusLabel(index),
        statusType: pickStatusType(index),
        createdAtMs: currentAtMs,
        sessionId,
      }))
    }

    if (includeStatus && index % 14 === 7) {
      currentAtMs += 30_000
      messages.push(createStatusMessage({
        id: `${sessionId}:status:burst-a:${index}`,
        from: assistantDid,
        to: [writerDid],
        senderName: 'CodeAssistant',
        label: '正在合并中间结果并准备下一轮输出...',
        statusType: 'processing',
        createdAtMs: currentAtMs,
        sessionId,
      }))
      currentAtMs += 35_000
      messages.push(createStatusMessage({
        id: `${sessionId}:status:burst-b:${index}`,
        from: assistantDid,
        to: [writerDid],
        senderName: 'CodeAssistant',
        label: '正在重新测量影响范围并校验差异...',
        statusType: 'info',
        createdAtMs: currentAtMs,
        sessionId,
      }))
    }
  }

  return messages
}

function buildUserPrompt(topic: string, index: number) {
  const prompt = userPrompts[index % userPrompts.length]
  if (index % 17 === 8) {
    return [
      `${prompt}`,
      `主题仍然是：${topic}。`,
      '我想顺便压一下长文本路径，所以这条消息会包含更多上下文：',
      '- 请明确哪些地方只是命名调整',
      '- 哪些地方是行为改变',
      '- 哪些地方需要补 migration note',
      '- 如果存在潜在性能回退，也请直接点出来',
      '',
      '另外，这次我希望结果可以直接拿去做 review，不要停留在泛泛而谈的建议层。',
    ].join('\n')
  }

  if (index % 12 === 0) {
    return `${prompt}\n\n顺便关注一下主题：${topic}。\n这次我更关心未来维护成本，不只是眼前能跑。`
  }

  if (index % 7 === 3) {
    return `${prompt}\n如果需要改接口，请把兼容策略也一起说明。`
  }

  return prompt
}

function buildAssistantReply(topic: string, index: number) {
  const base = assistantReplies[index % assistantReplies.length]
  const suffix = index % 5 === 0
    ? `\n\n${longBlocks[index % longBlocks.length]}`
    : ''

  const diagnosticSuffix = index % 13 === 5
    ? `\n\n${diagnosticBlocks[index % diagnosticBlocks.length]}`
    : ''

  if (index % 16 === 9) {
    return [
      `${base}`,
      '',
      `下面给一版更长的、适合压测 UI 的输出样本，主题仍然围绕 ${topic}：`,
      '1. 先固定协议对象边界，避免 UI 再造一层 message DTO。',
      '2. 再把 reader 约束成 totalCount + readRange，避免历史消息直接暴露成大数组。',
      '3. 接着把时间标签、状态条目和消息对象统一投影到一个虚拟序列里。',
      '4. 最后再看滚动到底部、插入新消息、长文本折行、连续状态条目是否还稳定。',
      '',
      '如果这几个点里有任何一个仍依赖“整段数组在内存里”的假设，后续一旦切到真实数据源就会暴露问题。',
      '',
      diagnosticBlocks[index % diagnosticBlocks.length],
      suffix,
    ].join('\n')
  }

  if (index % 11 === 6) {
    return `${base}\n\n这次我会围绕 ${topic} 给出一版更细的落地说明：\n- 哪些文件先动\n- 哪些行为必须保持不变\n- 哪些测试应该先补\n${diagnosticSuffix}${suffix}`
  }

  if (index % 8 === 2) {
    return `${base}\n\n我已经把问题收敛到一个更具体的点：数据读取边界不清晰，导致后续组件改动会牵一发而动全身。${diagnosticSuffix}${suffix}`
  }

  return `${base}${diagnosticSuffix}${suffix}`
}

function pickStatusLabel(index: number) {
  const labels = [
    '正在整理调用链和依赖关系...',
    '正在写回归测试并补失败路径...',
    '正在把输出重构成 review 友好的提交说明...',
    '正在校验重构前后行为是否一致...',
  ]
  return labels[index % labels.length]
}

function pickStatusType(index: number): ConversationStatusType {
  const types: ConversationStatusType[] = [
    'processing',
    'info',
    'processing',
    'typing',
  ]
  return types[index % types.length]
}

function computeGapMs(index: number) {
  if (index > 0 && index % 10 === 0) {
    return 25 * 3600_000
  }

  if (index > 0 && index % 18 === 0) {
    return 13 * 3600_000
  }

  if (index % 4 === 0) {
    return 95 * 60_000
  }

  if (index % 3 === 0) {
    return 3 * 60_000
  }

  return (14 + (index % 5) * 7) * 60_000
}

function shouldUseImageMessage(
  index: number,
  sender: 'user' | 'assistant',
) {
  return sender === 'assistant'
    ? index % 6 === 1
    : index % 6 === 4
}

function buildImageCaption({
  topic,
  index,
  sender,
  fallback,
}: {
  topic: string
  index: number
  sender: 'user' | 'assistant'
  fallback: string
}) {
  const seed = sender === 'assistant'
    ? assistantImageCaptions[index % assistantImageCaptions.length]
    : userImageCaptions[index % userImageCaptions.length]

  return [
    seed,
    `当前主题：${topic}。`,
    fallback,
  ].join('\n\n')
}

function createChatMessage({
  id,
  from,
  to,
  senderName,
  content,
  createdAtMs,
  deliveryStatus,
  sessionId,
  contentOverride,
}: {
  id: string
  from: string
  to: string[]
  senderName: string
  content: string
  createdAtMs: number
  deliveryStatus?: MessageDeliveryStatus
  sessionId: string
  contentOverride?: MsgContent
}): MessageObject {
  return {
    from,
    to,
    kind: 'chat',
    created_at_ms: createdAtMs,
    content: contentOverride ?? {
      format: 'text/plain',
      content,
    },
    ui_message_id: id,
    ui_sender_name: senderName,
    ui_delivery_status: deliveryStatus,
    ui_session_id: sessionId,
  }
}

function createImageMessage({
  id,
  from,
  to,
  senderName,
  content,
  createdAtMs,
  deliveryStatus,
  sessionId,
}: {
  id: string
  from: string
  to: string[]
  senderName: string
  content: string
  createdAtMs: number
  deliveryStatus?: MessageDeliveryStatus
  sessionId: string
}) {
  return createChatMessage({
    id,
    from,
    to,
    senderName,
    content,
    createdAtMs,
    deliveryStatus,
    sessionId,
    contentOverride: {
      format: 'application/octet-stream',
      content,
      refs: [
        {
          role: 'input',
          label: 'Museo di Santa Giulia',
          target: {
            type: 'data_obj',
            obj_id: `${id}:image-ref`,
            uri_hint: trustedImageUri,
          },
        },
      ],
    },
  })
}

function createStatusMessage({
  id,
  from,
  to,
  senderName,
  label,
  statusType,
  createdAtMs,
  sessionId,
}: {
  id: string
  from: string
  to: string[]
  senderName: string
  label: string
  statusType: ConversationStatusType
  createdAtMs: number
  sessionId: string
}): MessageObject {
  return {
    from,
    to,
    kind: 'notify',
    created_at_ms: createdAtMs,
    content: {
      format: 'text/plain',
      content: label,
    },
    ui_item_kind: 'status',
    ui_message_id: id,
    ui_sender_name: senderName,
    ui_session_id: sessionId,
    ui_status_type: statusType,
  }
}

export function getCodeAssistantSessionIds() {
  return (mockSessions[codeAssistantEntityId] ?? []).map((session) => session.id)
}
