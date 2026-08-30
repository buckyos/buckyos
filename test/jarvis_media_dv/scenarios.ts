export type AssetKey =
  | "image_primary"
  | "image_secondary"
  | "image_ocr"
  | "audio_sfx"
  | "audio_speech"
  | "video_fresh";

export type ArtifactExpectation = "image/" | "audio/" | "video/";

export interface StepExpectation {
  artifact?: ArtifactExpectation;
  textRequired?: boolean;
  textAny?: string[];
  textNone?: string[];
}

export interface ScenarioStep {
  id: string;
  prompt: string;
  attachment?: AssetKey;
  replyToStep?: string;
  expect: StepExpectation;
  maxWaitMs?: number;
  sendWithoutWaiting?: boolean;
  delayAfterSendMs?: number;
  review: string[];
}

export interface Scenario {
  id: string;
  suite: "smoke" | "linked" | "matrix";
  title: string;
  purpose: string;
  requiredAssets: AssetKey[];
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

export const ASSET_ENV: Record<AssetKey, string> = {
  image_primary: "JARVIS_DV_IMAGE_PRIMARY_ID",
  image_secondary: "JARVIS_DV_IMAGE_SECONDARY_ID",
  image_ocr: "JARVIS_DV_IMAGE_OCR_ID",
  audio_sfx: "JARVIS_DV_AUDIO_SFX_ID",
  audio_speech: "JARVIS_DV_AUDIO_SPEECH_ID",
  video_fresh: "JARVIS_DV_VIDEO_FRESH_ID",
};

export const ASSET_FILE: Record<AssetKey, string> = {
  image_primary: "assets/image_primary.png",
  image_secondary: "assets/image_secondary.png",
  image_ocr: "assets/image_ocr.png",
  audio_sfx: "assets/audio_sfx.wav",
  audio_speech: "assets/audio_speech.wav",
  video_fresh: "assets/video_fresh.mp4",
};

export const ASSET_LABEL: Record<AssetKey, string> = {
  image_primary: "image/png",
  image_secondary: "image/png",
  image_ocr: "image/png",
  audio_sfx: "audio/wav",
  audio_speech: "audio/wav",
  video_fresh: "video/mp4",
};

export const ASSET_DESCRIPTION: Record<AssetKey, string> = {
  image_primary: "主图片：粉色花朵、岩石和绿叶组成的原创插图，用于描述、编辑和图生视频",
  image_secondary: "第二张明显不同的原创山地公路插图，用于附件归属测试",
  image_ocr: "包含唯一文字 BUCKYOS-DV-4827 的图片",
  audio_sfx: "只有蜂鸣、敲击或环境音效且完全没有人声的短音频",
  audio_speech: "清晰朗读“今天的测试编号是四八二七”的短音频",
  video_fresh: "用户上传的 CC0 花朵 MP4，并非当前 Provider 生成的视频",
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
  ...conversionMatrixScenarios(),
];
