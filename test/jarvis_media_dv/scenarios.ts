export type AssetKey =
  | "image_primary"
  | "image_secondary"
  | "image_ocr"
  | "audio_sfx"
  | "audio_speech"
  | "video_fresh"
  | "video_subtitle"
  | "document_facts"
  | "archive_mixed"
  | "archive_single_document"
  | "archive_multiple_documents"
  | "archive_nested"
  | "archive_empty"
  | "archive_corrupt"
  | "archive_encrypted"
  | "archive_path_traversal"
  | "archive_many_files"
  | "archive_large_expansion"
  | "archive_deep_nesting";

export type ArtifactExpectation =
  | "image/"
  | "audio/"
  | "video/"
  | "text/"
  | "application/pdf"
  | "application/zip";

export interface StepExpectation {
  artifact?: ArtifactExpectation;
  artifacts?: ArtifactExpectation[];
  attachmentCount?: { min: number; max: number };
  textRequired?: boolean;
  textAny?: string[];
  textAll?: string[];
  textNone?: string[];
}

export interface ScenarioStep {
  id: string;
  prompt: string;
  attachment?: AssetKey;
  attachments?: AssetKey[];
  replyToStep?: string;
  expect: StepExpectation;
  maxWaitMs?: number;
  sendWithoutWaiting?: boolean;
  delayAfterSendMs?: number;
  duplicateInbound?: boolean;
  assertUniqueOutbound?: boolean;
  messageKind?: "chat" | "group_msg";
  sourceDid?: string;
  review: string[];
}

export interface Scenario {
  id: string;
  suite: "smoke" | "linked" | "matrix";
  title: string;
  purpose: string;
  requiredAssets: AssetKey[];
  requiresGroup?: boolean;
  coverage?: {
    status: "not_applicable" | "platform_limitation";
    reason: string;
  };
  steps: ScenarioStep[];
}

type MatrixSpec = {
  id: string;
  title: string;
  prompt: string;
  attachment?: AssetKey;
  expect: StepExpectation;
  maxWaitMs?: number;
  review: string[];
};

const MATRIX_SPECS: MatrixSpec[] = [
  {
    id: "text_to_text",
    title: "文本 → 文本",
    prompt: "把这段文字概括成一句中文：BuckyOS 是面向个人智能设备的操作系统，Jarvis 可以通过工具处理文本和多媒体任务。",
    expect: { textRequired: true, textAny: ["BuckyOS", "Jarvis"] },
    review: ["输出是一句忠于原意的中文摘要"],
  },
  {
    id: "text_to_image",
    title: "文本 → 图片",
    prompt: "根据这段描述生成并发送一张图片：清晨的湖边有一座白色灯塔，天空呈淡蓝色，画面写实。",
    expect: { textRequired: true, artifact: "image/" },
    maxWaitMs: 360_000,
    review: ["收到图片附件", "画面包含清晨、湖、白色灯塔等关键元素"],
  },
  {
    id: "text_to_audio",
    title: "文本 → 音频",
    prompt: "把“欢迎参加 BuckyOS 多媒体转换测试”转换成中文语音并发送音频。",
    expect: { textRequired: true, artifact: "audio/" },
    maxWaitMs: 360_000,
    review: ["收到可播放音频", "朗读内容与输入文字一致"],
  },
  {
    id: "text_to_video",
    title: "文本 → 视频",
    prompt: "根据文字生成并发送一段 4 秒短视频：一只纸飞机从书桌上起飞，缓慢飞向打开的窗户，写实风格。",
    expect: { textRequired: true, artifact: "video/" },
    maxWaitMs: 720_000,
    review: ["收到可播放视频", "视频表现纸飞机从书桌飞向窗户"],
  },
  {
    id: "image_to_text",
    title: "图片 → 文本",
    prompt: "用一段中文客观描述这张图片的主体、背景和主要颜色。",
    attachment: "image_primary",
    expect: { textRequired: true },
    review: ["描述与图片实际内容一致"],
  },
  {
    id: "image_to_image",
    title: "图片 → 图片",
    prompt: "把这张图片改成傍晚暖色光线，保持主体和构图不变，并发送编辑后的图片。",
    attachment: "image_primary",
    expect: { textRequired: true, artifact: "image/" },
    maxWaitMs: 360_000,
    review: ["收到编辑后的图片", "主体与构图保留，光线变为傍晚暖色"],
  },
  {
    id: "image_to_audio",
    title: "图片 → 音频",
    prompt: "理解这张图片，用中文生成一段不超过 20 秒的语音讲解，并发送音频。",
    attachment: "image_primary",
    expect: { textRequired: true, artifact: "audio/" },
    maxWaitMs: 360_000,
    review: ["收到语音讲解", "语音内容与图片一致"],
  },
  {
    id: "image_to_video",
    title: "图片 → 视频",
    prompt: "用这张图片生成并发送一段 4 秒短视频，镜头缓慢推进，主体有轻微自然运动。",
    attachment: "image_primary",
    expect: { textRequired: true, artifact: "video/" },
    maxWaitMs: 720_000,
    review: ["收到视频附件", "视频使用本条消息的图片作为视觉来源"],
  },
  {
    id: "audio_to_text",
    title: "音频 → 文本",
    prompt: "逐字转写这段语音，并用文本回复。",
    attachment: "audio_speech",
    expect: { textRequired: true, textAny: ["四八二七", "4827"] },
    review: ["转写包含正确的测试编号"],
  },
  {
    id: "audio_to_image",
    title: "音频 → 图片",
    prompt: "理解这段语音表达的内容，根据其中的主题创作并发送一张配图。",
    attachment: "audio_speech",
    expect: { textRequired: true, artifact: "image/" },
    maxWaitMs: 360_000,
    review: ["收到图片附件", "配图主题与语音内容存在明确联系"],
  },
  {
    id: "audio_to_audio",
    title: "音频 → 音频",
    prompt: "增强这段语音的清晰度并减少背景噪声，保持原话内容，发送处理后的音频。",
    attachment: "audio_speech",
    expect: { textRequired: true, artifact: "audio/" },
    maxWaitMs: 360_000,
    review: ["收到处理后的音频", "语义未改变且清晰度没有明显下降"],
  },
  {
    id: "audio_to_video",
    title: "音频 → 视频",
    prompt: "理解这段语音的内容，根据它的主题生成并发送一段 4 秒短视频。",
    attachment: "audio_speech",
    expect: { textRequired: true, artifact: "video/" },
    maxWaitMs: 720_000,
    review: ["收到视频附件", "视频主题与语音内容存在明确联系"],
  },
  {
    id: "video_to_text",
    title: "视频 → 文本",
    prompt: "用中文描述这段视频的场景、主体、动作和时间变化。",
    attachment: "video_fresh",
    expect: { textRequired: true },
    maxWaitMs: 360_000,
    review: ["描述覆盖视频中的动作和时间变化", "内容与原视频一致"],
  },
  {
    id: "video_to_image",
    title: "视频 → 图片",
    prompt: "从这段视频中选取一帧能代表主要内容的画面，作为静态图片发送给我。",
    attachment: "video_fresh",
    expect: { textRequired: true, artifact: "image/" },
    maxWaitMs: 360_000,
    review: ["收到静态图片", "图片确实来自原视频且具有代表性"],
  },
  {
    id: "video_to_audio",
    title: "视频 → 音频",
    prompt: "理解这段视频，用中文生成一段不超过 20 秒的语音解说，并发送音频。",
    attachment: "video_fresh",
    expect: { textRequired: true, artifact: "audio/" },
    maxWaitMs: 360_000,
    review: ["收到语音解说", "解说内容与视频一致"],
  },
  {
    id: "video_to_video",
    title: "视频 → 视频",
    prompt: "把这段视频转换为温暖的电影色调，保持原有主体、动作和时长，并发送处理后的视频。",
    attachment: "video_fresh",
    expect: { textRequired: true, artifact: "video/" },
    maxWaitMs: 720_000,
    review: ["收到处理后的视频", "主体与动作保持一致，色调符合要求"],
  },
];

function conversionMatrixScenarios(): Scenario[] {
  return MATRIX_SPECS.map((spec) => ({
    id: `matrix_${spec.id}`,
    suite: "matrix",
    title: spec.title,
    purpose: `验证 ${spec.title} 的真实端到端转换、附件投递和终态回复。`,
    requiredAssets: spec.attachment ? [spec.attachment] : [],
    steps: [{
      id: "convert",
      prompt: spec.prompt,
      ...(spec.attachment ? { attachment: spec.attachment } : {}),
      expect: spec.expect,
      ...(spec.maxWaitMs ? { maxWaitMs: spec.maxWaitMs } : {}),
      review: spec.review,
    }],
  }));
}

function archiveBoundaryScenario(input: {
  id: string;
  title: string;
  asset: AssetKey;
  prompt: string;
  evidence: string[];
}): Scenario {
  return {
    id: input.id,
    suite: "smoke",
    title: input.title,
    purpose: `验证 Jarvis 对${input.title}执行安全边界处理，并给出明确诊断。`,
    requiredAssets: [input.asset],
    steps: [{
      id: "inspect",
      prompt: input.prompt,
      attachment: input.asset,
      expect: {
        textRequired: true,
        textAny: input.evidence,
        attachmentCount: { min: 0, max: 0 },
      },
      review: ["未在解压目录之外写入文件", "未继续处理被拒绝的归档内容", "回复明确说明拒绝原因"],
    }],
  };
}

export const ASSET_ENV: Record<AssetKey, string> = {
  image_primary: "JARVIS_DV_IMAGE_PRIMARY_ID",
  image_secondary: "JARVIS_DV_IMAGE_SECONDARY_ID",
  image_ocr: "JARVIS_DV_IMAGE_OCR_ID",
  audio_sfx: "JARVIS_DV_AUDIO_SFX_ID",
  audio_speech: "JARVIS_DV_AUDIO_SPEECH_ID",
  video_fresh: "JARVIS_DV_VIDEO_FRESH_ID",
  video_subtitle: "JARVIS_DV_VIDEO_SUBTITLE_ID",
  document_facts: "JARVIS_DV_DOCUMENT_FACTS_ID",
  archive_mixed: "JARVIS_DV_ARCHIVE_MIXED_ID",
  archive_single_document: "JARVIS_DV_ARCHIVE_SINGLE_DOCUMENT_ID",
  archive_multiple_documents: "JARVIS_DV_ARCHIVE_MULTIPLE_DOCUMENTS_ID",
  archive_nested: "JARVIS_DV_ARCHIVE_NESTED_ID",
  archive_empty: "JARVIS_DV_ARCHIVE_EMPTY_ID",
  archive_corrupt: "JARVIS_DV_ARCHIVE_CORRUPT_ID",
  archive_encrypted: "JARVIS_DV_ARCHIVE_ENCRYPTED_ID",
  archive_path_traversal: "JARVIS_DV_ARCHIVE_PATH_TRAVERSAL_ID",
  archive_many_files: "JARVIS_DV_ARCHIVE_MANY_FILES_ID",
  archive_large_expansion: "JARVIS_DV_ARCHIVE_LARGE_EXPANSION_ID",
  archive_deep_nesting: "JARVIS_DV_ARCHIVE_DEEP_NESTING_ID",
};

export const ASSET_FILE: Record<AssetKey, string> = {
  image_primary: "assets/image_primary.png",
  image_secondary: "assets/image_secondary.png",
  image_ocr: "assets/image_ocr.png",
  audio_sfx: "assets/audio_sfx.wav",
  audio_speech: "assets/audio_speech.wav",
  video_fresh: "assets/video_fresh.mp4",
  video_subtitle: "assets/video_subtitle.vtt",
  document_facts: "assets/document_facts.md",
  archive_mixed: "assets/archive_mixed.zip",
  archive_single_document: "../aicc_test/fixtures/zip_single_document.zip",
  archive_multiple_documents: "../aicc_test/fixtures/zip_multiple_documents.zip",
  archive_nested: "../aicc_test/fixtures/zip_nested.zip",
  archive_empty: "../aicc_test/fixtures/zip_empty.zip",
  archive_corrupt: "../aicc_test/fixtures/zip_corrupt.zip",
  archive_encrypted: "../aicc_test/fixtures/zip_encrypted_flag.zip",
  archive_path_traversal: "../aicc_test/fixtures/zip_path_traversal.zip",
  archive_many_files: "../aicc_test/fixtures/zip_many_files.zip",
  archive_large_expansion: "../aicc_test/fixtures/zip_large_expansion.zip",
  archive_deep_nesting: "../aicc_test/fixtures/zip_deep_nesting.zip",
};

export const ASSET_LABEL: Record<AssetKey, string> = {
  image_primary: "image/png",
  image_secondary: "image/png",
  image_ocr: "image/png",
  audio_sfx: "audio/wav",
  audio_speech: "audio/wav",
  video_fresh: "video/mp4",
  video_subtitle: "text/vtt",
  document_facts: "text/markdown",
  archive_mixed: "application/zip",
  archive_single_document: "application/zip",
  archive_multiple_documents: "application/zip",
  archive_nested: "application/zip",
  archive_empty: "application/zip",
  archive_corrupt: "application/zip",
  archive_encrypted: "application/zip",
  archive_path_traversal: "application/zip",
  archive_many_files: "application/zip",
  archive_large_expansion: "application/zip",
  archive_deep_nesting: "application/zip",
};

export const ASSET_DESCRIPTION: Record<AssetKey, string> = {
  image_primary: "主图片：粉色花朵、岩石和绿叶组成的原创插图，用于描述、编辑和图生视频",
  image_secondary: "第二张明显不同的原创山地公路插图，用于附件归属测试",
  image_ocr: "包含唯一文字 BUCKYOS-DV-4827 的图片",
  audio_sfx: "只有蜂鸣、敲击或环境音效且完全没有人声的短音频",
  audio_speech: "清晰朗读“今天的测试编号是四八二七”的短音频",
  video_fresh: "用户上传的 CC0 花朵 MP4，并非当前 Provider 生成的视频",
  video_subtitle: "带时间码和事实码 AICC-SUBTITLE-2468 的确定性 WebVTT 字幕",
  document_facts: "包含唯一事实码 AICC-DOC-7319 和结构化项目数据的 Markdown 文档",
  archive_mixed: "包含中英文文档、CSV、图片和空目录的确定性 ZIP",
  archive_single_document: "只含一个确定性文档的 ZIP",
  archive_multiple_documents: "包含多个确定性文档的 ZIP",
  archive_nested: "包含嵌套 ZIP 的确定性归档",
  archive_empty: "空 ZIP 安全边界 fixture",
  archive_corrupt: "损坏 ZIP 安全边界 fixture",
  archive_encrypted: "设置加密标志的 ZIP 安全边界 fixture",
  archive_path_traversal: "包含路径穿越文件名的 ZIP 安全边界 fixture",
  archive_many_files: "超过文件数量限制的 ZIP 安全边界 fixture",
  archive_large_expansion: "高压缩比、超过总解压量限制的 ZIP fixture",
  archive_deep_nesting: "超过嵌套深度限制的 ZIP fixture",
};

export const SCENARIOS: Scenario[] = [
  {
    id: "image_describe",
    suite: "smoke",
    title: "图片理解",
    purpose: "验证附件可以被媒体理解工具读取，回复与实际图片一致。",
    requiredAssets: ["image_primary"],
    steps: [
      {
        id: "describe",
        prompt: "请客观描述这张图片的主体、背景和主要颜色。",
        attachment: "image_primary",
        expect: { textRequired: true },
        review: [
          "描述与主图片一致",
          "回复没有声称生成了新图片",
          "没有暴露对象 ID、容器路径或内部工具协议",
        ],
      },
    ],
  },
  {
    id: "image_ocr",
    suite: "smoke",
    title: "图片文字识别",
    purpose: "验证图片文字提取能够由可用的媒体理解能力完成。",
    requiredAssets: ["image_ocr"],
    steps: [
      {
        id: "ocr",
        prompt: "识别图片中的文字，保持原有字符顺序。",
        attachment: "image_ocr",
        expect: { textRequired: true, textAny: ["BUCKYOS-DV-4827"] },
        review: [
          "结果包含 BUCKYOS-DV-4827",
          "已经取得文字时不会因额外的专用 OCR 路由失败而把整个任务报告为失败",
        ],
      },
    ],
  },
  {
    id: "image_edit",
    suite: "smoke",
    title: "图片编辑与附件回传",
    purpose: "验证 edit_image、长命令等待和最终图片附件投递。",
    requiredAssets: ["image_primary"],
    steps: [
      {
        id: "edit",
        prompt: "在花朵中央添加一只写实的大黄蜂，保持原图光线和构图，并把编辑后的图片发给我。",
        attachment: "image_primary",
        expect: { artifact: "image/", textRequired: true },
        maxWaitMs: 360_000,
        review: [
          "最终收到可打开的图片附件",
          "大黄蜂位于目标花朵上，背景仍来自原图",
          "回复不是编辑说明或本地文件路径",
        ],
      },
    ],
  },
  {
    id: "audio_sfx",
    suite: "smoke",
    title: "非语音音效理解与可靠转写",
    purpose: "验证普通音频描述走媒体理解，显式转写不会把音效臆测成句子。",
    requiredAssets: ["audio_sfx"],
    steps: [
      {
        id: "describe_sfx",
        prompt: "描述这段音频的内容，并说明是否能听到清晰人声。",
        attachment: "audio_sfx",
        expect: { textRequired: true },
        review: [
          "描述为音效、蜂鸣、敲击或环境声",
          "明确说明没有清晰可辨的人声",
        ],
      },
      {
        id: "transcribe_sfx",
        prompt: "请逐字转写我刚才发送的那段音频；如果没有可靠语音，请如实说明。",
        expect: {
          textRequired: true,
          textNone: [
            "I've decided to cancel the project",
            "The final score was 10-0",
            "Welcome.",
          ],
        },
        review: [
          "没有把音效断言为具体句子",
          "低可信候选被表达为不确定或无可靠语音",
        ],
      },
    ],
  },
  {
    id: "speech_roundtrip",
    suite: "smoke",
    title: "语音转写与语音合成串联",
    purpose: "验证可靠 ASR 结果可以被后续 TTS 任务引用。",
    requiredAssets: ["audio_speech"],
    steps: [
      {
        id: "transcribe",
        prompt: "逐字转写这段清晰语音。",
        attachment: "audio_speech",
        expect: { textRequired: true, textAny: ["四八二七", "4827"] },
        review: ["转写包含测试编号四八二七或 4827"],
      },
      {
        id: "tts_from_history",
        prompt: "把刚才识别出的完整句子转换成语音并把音频发给我。",
        expect: { artifact: "audio/", textRequired: true },
        maxWaitMs: 360_000,
        review: [
          "收到可播放的音频附件",
          "合成内容来自上一条转写，而不是重新编造无关句子",
        ],
      },
    ],
  },
  {
    id: "image_to_video",
    suite: "smoke",
    title: "图生视频、长任务与中文终态",
    purpose: "验证 img2video 超过普通 shell 时限后仍能交付视频，终态沿用中文。",
    requiredAssets: ["image_primary"],
    steps: [
      {
        id: "generate_video",
        prompt: "用这张图生成一段 4 秒短视频：镜头缓慢推进，主体有轻微自然运动。完成后直接把视频发给我。",
        attachment: "image_primary",
        expect: { artifact: "video/", textRequired: true },
        maxWaitMs: 720_000,
        review: [
          "等待超过 60 秒时任务仍继续而不是重复提交",
          "最终收到可播放视频",
          "视频内容来自本条消息的图片",
          "最终用户回复使用中文",
        ],
      },
    ],
  },
  {
    id: "edit_then_animate",
    suite: "linked",
    title: "历史图片引用：编辑后再动画化",
    purpose: "验证后续消息优先引用上一轮生成物，而不是回退到原始附件。",
    requiredAssets: ["image_primary"],
    steps: [
      {
        id: "introduce",
        prompt: "这是本任务的原始花朵素材。先用一句话确认你看到的主体，暂时不要编辑。",
        attachment: "image_primary",
        expect: { textRequired: true },
        review: ["正确识别原始花朵素材"],
      },
      {
        id: "edit_from_history",
        prompt: "现在在刚才那朵花上添加一只写实的大黄蜂，并把编辑后的图片发给我。",
        expect: { artifact: "image/", textRequired: true },
        maxWaitMs: 360_000,
        review: ["编辑结果来自上一条原始花朵图片并包含大黄蜂"],
      },
      {
        id: "animate_output",
        prompt: "把你刚生成的那张带大黄蜂的图片制作成 4 秒短视频，让蜜蜂轻微振翅。完成后发视频。",
        expect: { artifact: "video/", textRequired: true },
        maxWaitMs: 720_000,
        review: [
          "输入是上一轮带大黄蜂的编辑结果",
          "视频中仍有大黄蜂，而不是只动画化最初的花朵图片",
        ],
      },
    ],
  },
  {
    id: "rapid_attachment_binding",
    suite: "linked",
    title: "连续消息附件归属",
    purpose: "验证两条快速连续消息仍保留各自正文和附件边界。",
    requiredAssets: ["image_primary", "image_secondary"],
    steps: [
      {
        id: "first_asset",
        prompt: "这是素材 A，只需记住它，稍后不要把它当成素材 B。",
        attachment: "image_primary",
        expect: { textRequired: true },
        sendWithoutWaiting: true,
        delayAfterSendMs: 300,
        review: [],
      },
      {
        id: "second_asset_video",
        prompt: "这是素材 B。请只使用本条消息附带的素材 B 生成 4 秒短视频，完成后把视频发给我。",
        attachment: "image_secondary",
        expect: { artifact: "video/", textRequired: true },
        maxWaitMs: 720_000,
        review: [
          "生成视频来自第二张图片",
          "第一条消息的附件没有被错误绑定到第二条指令",
        ],
      },
    ],
  },
  {
    id: "correction_without_attachment",
    suite: "linked",
    title: "纠错消息复用正确历史附件",
    purpose: "验证用户纠错时无需重复上传，Agent 可以回到被明确引用的历史素材。",
    requiredAssets: ["image_primary", "image_secondary"],
    steps: [
      {
        id: "primary",
        prompt: "将这张花朵图片记为素材 A。只确认，不生成内容。",
        attachment: "image_primary",
        expect: { textRequired: true },
        review: [],
      },
      {
        id: "secondary",
        prompt: "将这张完全不同的图片记为素材 B。只确认，不生成内容。",
        attachment: "image_secondary",
        expect: { textRequired: true },
        review: [],
      },
      {
        id: "ambiguous_request",
        prompt: "用最近发送的素材生成一段短视频。",
        expect: { artifact: "video/", textRequired: true },
        maxWaitMs: 720_000,
        review: ["第一次结果应使用最近发送的素材 B"],
      },
      {
        id: "correction",
        prompt: "我指的是更早那张花朵素材 A。请改用素材 A 重新生成，不需要我再次上传。",
        replyToStep: "primary",
        expect: { artifact: "video/", textRequired: true },
        maxWaitMs: 720_000,
        review: [
          "纠错消息通过 reply_to 明确引用素材 A 的原消息",
          "纠错后的结果使用素材 A，且用户没有重新上传图片",
        ],
      },
    ],
  },
  {
    id: "generated_video_extension",
    suite: "linked",
    title: "生成视频的连续续写",
    purpose: "验证生成视频与 Provider continuation 状态之间的持久映射。",
    requiredAssets: ["image_primary"],
    steps: [
      {
        id: "generate",
        prompt: "用这张图生成 4 秒短视频并发给我，动作保持自然。",
        attachment: "image_primary",
        expect: { artifact: "video/", textRequired: true },
        maxWaitMs: 720_000,
        review: ["收到第一段生成视频"],
      },
      {
        id: "extend",
        prompt: "从刚才视频的结尾继续延长 4 秒，保持同一场景和运动方向，并把完整结果发给我。",
        expect: { artifact: "video/", textRequired: true },
        maxWaitMs: 720_000,
        review: [
          "支持原生续写时，后续片段与上一段连续",
          "原生续写不可用时，先用用户可理解的语言解释并说明替代方案",
        ],
      },
    ],
  },
  {
    id: "fresh_video_extension",
    suite: "linked",
    title: "用户上传视频的续写降级",
    purpose: "验证全新上传视频缺少 Provider 原生续写状态时的可理解处理。",
    requiredAssets: ["video_fresh"],
    steps: [
      {
        id: "extend_fresh",
        prompt: "把这个视频从结尾延长 4 秒，尽量保持原场景连续。",
        attachment: "video_fresh",
        expect: { textRequired: true },
        maxWaitMs: 720_000,
        review: [
          "不会声称任意上传视频一定支持 Provider 原生续写",
          "如采用末帧生成并拼接等替代方案，会先解释过渡可能不完全连续",
          "用户回复不包含 continuation_handle 等内部术语",
        ],
      },
    ],
  },
  {
    id: "document_read_and_export",
    suite: "smoke",
    title: "文档读取与结果文档",
    purpose: "验证文档入站、固定事实提取以及结果文档出站。",
    requiredAssets: ["document_facts"],
    steps: [
      {
        id: "read",
        prompt: "读取附件，回复其中的事实码、项目负责人和预算数字。",
        attachment: "document_facts",
        expect: { textRequired: true, textAny: ["AICC-DOC-7319"] },
        review: ["事实码、负责人和预算均来自附件，未臆造字段"],
      },
      {
        id: "export",
        prompt: "把刚才提取的事实生成一份 Markdown 或 PDF 报告并作为附件发送。",
        expect: { textRequired: true, artifact: "text/", attachmentCount: { min: 1, max: 1 } },
        review: ["结果附件可读并包含 AICC-DOC-7319"],
      },
    ],
  },
  {
    id: "archive_process_and_repack",
    suite: "smoke",
    title: "压缩包安全处理与重新打包",
    purpose: "验证 ZIP 入站、内部文件处理和 ZIP 出站。",
    requiredAssets: ["archive_mixed"],
    steps: [
      {
        id: "inspect",
        prompt: "安全解压附件，列出内部文件并回复事实码 AICC-ZIP-8642 所在文件。",
        attachment: "archive_mixed",
        expect: { textRequired: true, textAny: ["AICC-ZIP-8642"] },
        review: ["文件清单包含中英文文件名、CSV、图片和空目录"],
      },
      {
        id: "repack",
        prompt: "生成 summary.md 后与原始数据一起重新打包成 ZIP 并发送。",
        expect: { textRequired: true, artifact: "application/zip", attachmentCount: { min: 1, max: 1 } },
        review: ["输出 ZIP 可解压且包含 summary.md"],
      },
    ],
  },
  {
    id: "archive_valid_shapes",
    suite: "smoke",
    title: "单文档、多文档与嵌套归档",
    purpose: "验证三种合法 ZIP 结构均被安全解包，且内部事实与文件清单不混淆。",
    requiredAssets: ["archive_single_document", "archive_multiple_documents", "archive_nested"],
    steps: [
      {
        id: "single",
        prompt: "安全解包该单文档 ZIP，回复内部文件名和事实码。",
        attachment: "archive_single_document",
        expect: { textRequired: true, textAny: ["AICC-FIXTURE-7319", "facts.txt"] },
        review: ["只报告该归档中的单个文档"],
      },
      {
        id: "multiple",
        prompt: "安全解包该多文档 ZIP，列出全部相对路径、空目录和每份文档的事实码。",
        attachment: "archive_multiple_documents",
        expect: { textRequired: true, textAny: ["AICC-FIXTURE-7319", "中文/说明.md"] },
        review: ["文件清单包含 a、b、中文目录和空目录，未串入上一步文件"],
      },
      {
        id: "nested",
        prompt: "在安全嵌套深度内解包外层和内层 ZIP，回复最终文档的事实码。",
        attachment: "archive_nested",
        expect: { textRequired: true, textAny: ["AICC-FIXTURE-7319", "inner.zip"] },
        review: ["只在允许的嵌套深度内处理，最终文档可读"],
      },
    ],
  },
  archiveBoundaryScenario({
    id: "archive_empty_rejected",
    title: "空 ZIP",
    asset: "archive_empty",
    prompt: "检查该 ZIP；如果没有可处理文件，明确说明并停止，不要生成结果附件。",
    evidence: ["空", "empty", "没有文件", "no files"],
  }),
  archiveBoundaryScenario({
    id: "archive_corrupt_rejected",
    title: "损坏 ZIP",
    asset: "archive_corrupt",
    prompt: "安全检查该归档。损坏时必须明确拒绝，不能猜测或继续处理。",
    evidence: ["损坏", "corrupt", "无效", "invalid"],
  }),
  archiveBoundaryScenario({
    id: "archive_encrypted_rejected",
    title: "加密 ZIP",
    asset: "archive_encrypted",
    prompt: "安全检查该归档。加密且没有密码时必须明确拒绝，不能绕过加密。",
    evidence: ["加密", "encrypted", "密码", "password"],
  }),
  archiveBoundaryScenario({
    id: "archive_path_traversal_rejected",
    title: "路径穿越 ZIP",
    asset: "archive_path_traversal",
    prompt: "安全检查该归档。发现路径穿越条目时必须拒绝整个归档，不得写出解压目录。",
    evidence: ["路径", "path", "穿越", "traversal", "安全", "unsafe"],
  }),
  archiveBoundaryScenario({
    id: "archive_many_files_rejected",
    title: "超文件数 ZIP",
    asset: "archive_many_files",
    prompt: "安全检查该归档。文件数超过限制时明确拒绝，并说明触发的是文件数量限制。",
    evidence: ["文件数", "数量", "too many", "file count", "限制", "limit"],
  }),
  archiveBoundaryScenario({
    id: "archive_large_expansion_rejected",
    title: "超总解压量 ZIP",
    asset: "archive_large_expansion",
    prompt: "安全检查该高压缩比归档。预计总解压量超过限制时必须在完整展开前拒绝。",
    evidence: ["解压", "大小", "expansion", "size", "限制", "limit"],
  }),
  archiveBoundaryScenario({
    id: "archive_deep_nesting_rejected",
    title: "超嵌套深度 ZIP",
    asset: "archive_deep_nesting",
    prompt: "安全检查该归档。嵌套目录或归档深度超过限制时明确拒绝，不得无限递归。",
    evidence: ["嵌套", "深度", "nest", "depth", "限制", "limit"],
  }),
  {
    id: "multi_attachment_current_and_history",
    suite: "linked",
    title: "当前与历史多附件绑定",
    purpose: "验证同一消息多附件的数量、顺序和历史 reply_to 引用。",
    requiredAssets: ["image_primary", "image_secondary", "document_facts"],
    steps: [
      {
        id: "bundle",
        prompt: "按附件顺序记录两张图片和事实文档，并回复事实码。",
        attachments: ["image_primary", "image_secondary", "document_facts"],
        expect: { textRequired: true, textAny: ["AICC-DOC-7319"] },
        review: ["三项附件均可见，顺序与发送顺序一致"],
      },
      {
        id: "refer_bundle",
        prompt: "只根据我引用的那条多附件消息，对比两张图，并用文档事实码命名结果。",
        attachment: "document_facts",
        replyToStep: "bundle",
        expect: { textRequired: true, artifact: "image/" },
        review: ["同时使用当前消息的文档和被引用历史消息中的两图，没有串入其他附件"],
      },
    ],
  },
  {
    id: "structured_text_output",
    suite: "smoke",
    title: "结构化文本输出",
    purpose: "验证 Jarvis 能将普通文本任务约束为可解析 JSON，并保持固定字段。",
    requiredAssets: [],
    steps: [{
      id: "json",
      prompt: "只返回一个 JSON 对象，字段必须为 summary、keywords、marker；marker 必须是 JARVIS-JSON-4827，不要使用 Markdown 代码块。",
      expect: { textRequired: true, textAny: ["JARVIS-JSON-4827"] },
      review: ["回复是可解析 JSON，包含 summary、keywords 和 marker 三个字段"],
    }],
  },
  {
    id: "duplicate_inbound_idempotency",
    suite: "linked",
    title: "重复入站幂等",
    purpose: "验证 msg-center 收到相同幂等键和相同消息两次时只保存、处理和回复一次。",
    requiredAssets: [],
    steps: [{
      id: "duplicate",
      prompt: "只回复一次 JARVIS-IDEMPOTENCY-4827。",
      duplicateInbound: true,
      expect: { textRequired: true, textAny: ["JARVIS-IDEMPOTENCY-4827"] },
      review: [],
    }],
  },
  {
    id: "outbound_delivery_idempotency",
    suite: "linked",
    title: "出站投递幂等",
    purpose: "验证同一 Jarvis 最终回复只形成一个可见出站消息和一个消息 ID。",
    requiredAssets: [],
    steps: [{
      id: "unique_reply",
      prompt: "只回复一次 JARVIS-OUTBOUND-IDEMPOTENCY-4827。",
      assertUniqueOutbound: true,
      expect: { textRequired: true, textAny: ["JARVIS-OUTBOUND-IDEMPOTENCY-4827"] },
      review: [],
    }],
  },
  {
    id: "group_message_semantics",
    suite: "linked",
    title: "群消息收发语义",
    purpose: "验证 from 保持实际发送者、to 指向 group DID，Jarvis 回复仍回到群会话。",
    requiredAssets: [],
    requiresGroup: true,
    steps: [{
      id: "group_reply",
      prompt: "这是群消息测试。请在群会话中只回复 JARVIS-GROUP-4827。",
      messageKind: "group_msg",
      expect: { textRequired: true, textAny: ["JARVIS-GROUP-4827"] },
      review: ["回复仍位于同一 group DID 会话，没有错误地私聊发送者"],
    }],
  },
  {
    id: "forwarded_message_source",
    suite: "linked",
    title: "转发消息来源语义",
    purpose: "验证 MsgObject.source 中的原始来源不会被当前转发者 from 覆盖。",
    requiredAssets: [],
    steps: [{
      id: "forward",
      prompt: "这是一条转发消息，正文事实码是 JARVIS-FORWARD-4827。请回复事实码并说明它来自转发内容。",
      sourceDid: "did:bns:aicc-forward-origin",
      expect: { textRequired: true, textAny: ["JARVIS-FORWARD-4827"] },
      review: ["Jarvis 将内容识别为转发内容，且没有把原始 source 与当前 from 混淆"],
    }],
  },
  {
    id: "delivery_retry_final_state",
    suite: "linked",
    title: "出站重试与最终投递状态",
    purpose: "验证临时投递失败后的重试次数、最终状态和用户可见失败回退。",
    requiredAssets: [],
    coverage: {
      status: "platform_limitation",
      reason: "当前公开 msg-center/MessageHub DV 接口没有按单条消息注入临时出站失败的能力；用例已登记，需故障注入入口后执行。",
    },
    steps: [{
      id: "retry_then_deliver",
      prompt: "触发一次可重试的临时投递失败，然后验证最终只投递一次。",
      expect: { textRequired: true },
      review: [],
    }],
  },
  {
    id: "jarvis_restart_recovery",
    suite: "linked",
    title: "Jarvis 重启恢复",
    purpose: "验证消息处理过程中 Jarvis 重启后任务恢复、去重和最终投递。",
    requiredAssets: [],
    coverage: {
      status: "platform_limitation",
      reason: "DV runner 默认没有 Jarvis 进程重启授权；用例保留为显式 gated 场景，不能用 reload 代替。",
    },
    steps: [{
      id: "restart_mid_task",
      prompt: "启动长任务，在处理中重启 Jarvis，并验证恢复后只有一个最终回复。",
      expect: { textRequired: true },
      review: [],
    }],
  },
  {
    id: "multi_attachment_audio_image",
    suite: "linked",
    title: "音频与图片组合入站",
    purpose: "验证同一消息中的音频和图片均被读取，顺序和归属不丢失。",
    requiredAssets: ["audio_speech", "image_primary"],
    steps: [{
      id: "analyze",
      prompt: "先转写第一个附件中的测试编号，再描述第二个附件的主体；按附件顺序分两点回答。",
      attachments: ["audio_speech", "image_primary"],
      expect: { textRequired: true, textAny: ["四八二七", "4827"] },
      review: ["两项附件都被使用，且没有交换音频与图片的顺序"],
    }],
  },
  {
    id: "multi_attachment_video_subtitle",
    suite: "linked",
    title: "视频与字幕文档组合入站",
    purpose: "验证视频和 WebVTT 字幕可在同一消息中关联处理。",
    requiredAssets: ["video_fresh", "video_subtitle"],
    steps: [{
      id: "verify",
      prompt: "核对视频与字幕内容，回复字幕中的验证事实码，并说明字幕描述是否与视频主体一致。",
      attachments: ["video_fresh", "video_subtitle"],
      expect: { textRequired: true, textAny: ["AICC-SUBTITLE-2468"] },
      review: ["事实码来自字幕，视频判断来自视频内容"],
    }],
  },
  {
    id: "multi_attachment_zip_document",
    suite: "linked",
    title: "ZIP 与补充文档组合入站",
    purpose: "验证压缩包和独立补充说明文档不会被串包或静默丢弃。",
    requiredAssets: ["archive_mixed", "document_facts"],
    steps: [{
      id: "compare",
      prompt: "安全检查 ZIP，再读取补充文档；分别回复 ZIP 事实码和文档事实码，并标明来源。",
      attachments: ["archive_mixed", "document_facts"],
      expect: { textRequired: true, textAll: ["AICC-ZIP-8642", "AICC-DOC-7319"] },
      review: ["两个事实码均出现且来源标注正确"],
    }],
  },
  {
    id: "multi_output_images",
    suite: "smoke",
    title: "多图片出站",
    purpose: "验证一次任务可以回传两张有序图片，而不是只保留最后一个附件。",
    requiredAssets: [],
    steps: [{
      id: "generate_pair",
      prompt: "生成两张图片并在同一最终回复中发送：第一张是蓝色方块，第二张是绿色圆形；保持这个顺序。",
      expect: { textRequired: true, artifacts: ["image/"], attachmentCount: { min: 2, max: 2 } },
      maxWaitMs: 360_000,
      review: ["恰好两张图片，顺序为蓝色方块、绿色圆形"],
    }],
  },
  {
    id: "multi_output_image_audio",
    suite: "smoke",
    title: "图片与语音讲解出站",
    purpose: "验证一次回复同时包含图片和对应音频讲解。",
    requiredAssets: ["image_primary"],
    steps: [{
      id: "package",
      prompt: "根据附件生成一张暖色编辑图，并同时生成不超过 15 秒的中文语音讲解；在同一最终回复中发送图片和音频。",
      attachment: "image_primary",
      expect: { textRequired: true, artifacts: ["image/", "audio/"], attachmentCount: { min: 2, max: 2 } },
      maxWaitMs: 480_000,
      review: ["图片和音频均可打开，音频讲解与图片一致"],
    }],
  },
  {
    id: "multi_output_video_subtitle_cover",
    suite: "smoke",
    title: "视频、字幕与封面图出站",
    purpose: "验证长任务完成后主动回传视频、字幕文档和封面图三种附件。",
    requiredAssets: ["video_fresh"],
    steps: [{
      id: "package",
      prompt: "处理该视频并在同一最终回复中发送三项结果：视频、WebVTT 字幕和一张封面图。",
      attachment: "video_fresh",
      expect: { textRequired: true, artifacts: ["video/", "text/", "image/"], attachmentCount: { min: 3, max: 3 } },
      maxWaitMs: 720_000,
      review: ["三项附件类型正确、均可读取，字幕与视频时间线相关"],
    }],
  },
  {
    id: "multi_output_document_zip",
    suite: "smoke",
    title: "结果文档与 ZIP 出站",
    purpose: "验证一次最终回复同时回传可读结果文档和包含该文档的 ZIP。",
    requiredAssets: ["archive_mixed"],
    steps: [{
      id: "package",
      prompt: "安全处理该 ZIP，生成 summary.md，并在同一最终回复中发送两项附件：独立 summary.md 和包含 summary.md 的新 ZIP。",
      attachment: "archive_mixed",
      expect: {
        textRequired: true,
        artifacts: ["text/", "application/zip"],
        attachmentCount: { min: 2, max: 2 },
      },
      review: ["两项附件均可读，ZIP 内的 summary.md 与独立文档内容一致"],
    }],
  },
  {
    id: "generated_image_edit_history",
    suite: "linked",
    title: "Provider 生成图片后继续编辑",
    purpose: "验证上一轮生成物、source task、exact model 和 Provider 引用能被历史恢复。",
    requiredAssets: [],
    steps: [
      {
        id: "generate",
        prompt: "生成并发送一张写实图片：白色杯子放在木桌上。",
        expect: { textRequired: true, artifact: "image/" },
        maxWaitMs: 360_000,
        review: ["收到白色杯子图片"],
      },
      {
        id: "edit",
        prompt: "直接编辑你刚生成的图片，把杯子改成蓝色，不要让我重新上传，并发送编辑结果。",
        expect: { textRequired: true, artifact: "image/" },
        maxWaitMs: 360_000,
        review: ["编辑使用上一轮生成图片，构图保留且杯子变蓝"],
      },
    ],
  },
  {
    id: "document_vector_retrieval",
    suite: "smoke",
    title: "文档向量检索链路",
    purpose: "记录 Jarvis 文档 chunk、embedding、向量检索和 rerank 链路的当前覆盖状态。",
    requiredAssets: ["document_facts"],
    coverage: {
      status: "not_applicable",
      reason: "当前 Jarvis 实现未配置可观测的 embedding.text + vector retrieval + rerank 文档检索链路；不得用普通 LLM 附件阅读伪造通过。",
    },
    steps: [{
      id: "retrieve",
      prompt: "通过文档向量检索和 rerank 回答事实码。",
      attachment: "document_facts",
      expect: { textRequired: true },
      review: [],
    }],
  },
  {
    id: "multimodal_image_retrieval",
    suite: "smoke",
    title: "图文同空间检索链路",
    purpose: "记录 Jarvis embedding.multimodal 图文检索能力的当前覆盖状态。",
    requiredAssets: ["image_primary", "image_secondary"],
    coverage: {
      status: "not_applicable",
      reason: "当前 Jarvis 实现未配置可观测的 embedding.multimodal 同空间图文索引；不得以普通视觉理解代替该链路。",
    },
    steps: [{
      id: "retrieve",
      prompt: "在图文同空间索引中返回与粉色花朵最相似的图片。",
      attachments: ["image_primary", "image_secondary"],
      expect: { textRequired: true },
      review: [],
    }],
  },
  ...conversionMatrixScenarios(),
];
