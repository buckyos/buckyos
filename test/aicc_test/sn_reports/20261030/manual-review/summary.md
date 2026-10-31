# T2 内容复核（2026-10-30）

通过已认证 Gateway 的 TaskMgr `get_task` 回读原任务；无新增模型调用。原始运行报告保持不变。

OCR 对照输入完整标记 `BUCKYOS-DV-4827`；caption 对照实际输入图；ASR 对照 [OpenAI Whisper 官方 JFK 测试](https://github.com/openai/whisper/blob/main/tests/test_transcribe.py)。

| Case | 复核结果 | Task | 依据 |
|---|---|---|---|
| t2.sn-ai-provider.sn-global-open-20261030-t2.deepseek-v4.1-flash-1017.vision.caption | passed | t-6b5476e85fa548f8b296b19b8a49ee8c | 图中粉色花、黄色花心、绿色茎叶及棕色椭圆背景与描述相符。 |
| t2.sn-ai-provider.sn-global-open-20261030-t2.deepseek-v4.1-flash-1017.vision.ocr | passed | t-a60dfa6ee061483da3d11df81439410d | 预期 BUCKYOS-DV-4827；实际 BUCKYOS-DV-4827 |
| t2.sn-ai-provider.sn-global-open-20261030-t2.glm-5.3-flash.vision.caption | passed | t-29cccb2d3e764a9089f5b9bdcf766078 | 图中粉色花、黄色花心、绿色茎叶及棕色椭圆背景与描述相符。 |
| t2.sn-ai-provider.sn-global-open-20261030-t2.glm-5.3-flash.vision.ocr | passed | t-af59f44805ad40678a79c5f9a3a8d9dd | 预期 BUCKYOS-DV-4827；实际 **BUCKYOS-DV-4827**  The image contains a single line of text displayed in a pixelated, digital-style font reading "BUCKYOS-DV-4827". |
| t2.sn-ai-provider.sn-global-open-20261030-t2.kimi-k3.1-pro.vision.caption | passed | t-f6249d8e56f54edfbf2113def822d158 | 图中粉色花、黄色花心、绿色茎叶及棕色椭圆背景与描述相符。 |
| t2.sn-ai-provider.sn-global-open-20261030-t2.kimi-k3.vision.caption | passed | t-9b70da367f974b96b3559977db8cab9e | 图中粉色花、黄色花心、绿色茎叶及棕色椭圆背景与描述相符。 |
| t2.sn-ai-provider.sn-global-open-20261030-t2.kimi-k3.vision.ocr | failed | t-babc83b36ffb4a779ce725199a56f2b1 | 预期 BUCKYOS-DV-4827；实际 BUCKY05-OV-4827 |
| t2.sn-ai-provider.sn-global-open-20261030-t2.minimax-m3.1-flash.vision.caption | passed | t-9f8cf10290904cf9bd7411e44b6605c9 | 图中粉色花、黄色花心、绿色茎叶及棕色椭圆背景与描述相符。 |
| t2.sn-ai-provider.sn-global-open-20261030-t2.minimax-m3.1-flash.vision.ocr | failed | t-76b69744bdb54970a21dee561635df94 | 预期 BUCKYOS-DV-4827；实际 BUCKY05-04-4827 |
| t2.sn-ai-provider.sn-global-open-20261030-t2.qwen3.6-plus.vision.caption | passed | t-890afe45e8c54981a0fd69d1f1203218 | 图中粉色花、黄色花心、绿色茎叶及棕色椭圆背景与描述相符。 |
| t2.sn-ai-provider.sn-global-open-20261030-t2.qwen3.7-plus.vision.caption | passed | t-c9ff2ad29e4b48d3903a584639227c63 | 图中粉色花、黄色花心、绿色茎叶及棕色椭圆背景与描述相符。 |
| t2.sn-ai-provider.sn-global-open-20261030-t2.qwen3.8-flash.vision.caption | passed | t-c428d8a51cb14850a22d00b225c900ae | 图中粉色花、黄色花心、绿色茎叶及棕色椭圆背景与描述相符。 |
| t2.sn-ai-provider.sn-global-open-20261030-t2.qwen3.8-flash.vision.ocr | failed | t-a7547c25e3304fffabf58ef8b12fd50f | 预期 BUCKYOS-DV-4827；实际 BUCKYOS-DU-4827 |
| t2.sn-ai-provider.sn-global-open-20261030-t2.qwen3.8-max-0902.vision.caption | passed | t-1f0aa87672e340a1aece1a94d3bd6b8a | 图中粉色花、黄色花心、绿色茎叶及棕色椭圆背景与描述相符。 |
| t2.sn-ai-provider.sn-global-open-20261030-t2.qwen3.8-omni-flash.vision.caption | passed | t-5b64511067d046bead725f1a15e219cf | 图中粉色花、黄色花心、绿色茎叶及棕色椭圆背景与描述相符。 |
| t2.sn-ai-provider.sn-global-open-20261030-t2.ui-tars-1.5-7b.vision.caption | passed | t-73553a8811694302b96faf0d1c0bf9a1 | 图中粉色花、黄色花心、绿色茎叶及棕色椭圆背景与描述相符。 |
| t2.sn-ai-provider.sn-global-open-20261030-t2.ui-tars-1.5-7b.vision.ocr | failed | t-dd9d76af60de43ba8252d4f8a8f82e89 | 预期 BUCKYOS-DV-4827；实际 BricKM05 - CV - 4827 |
| t2.sn-ai-provider.sn-global-open-20261030-t2.voxtral-small-24b-2507.audio.asr | passed | t-f20f33619ac746e182078c7990f56a40 | 对照 OpenAI Whisper 官方 JFK 语音及测试断言：And so, my fellow Americans, ask not what your country can do for you, ask what you can do for your country. |
| t2.sn-ai-provider.sn-us-full-20261030-t2.claude-haiku-5.5.vision.caption | failed | t-5fe30273064c43f1a423d08c8450a477 | 主体正确，但将五个背景椭圆描述为四个。 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.claude-haiku-5.5.vision.ocr | passed | t-1425016fbe1b421ea2f75058dbbd6080 | 预期 BUCKYOS-DV-4827；实际 The visible text reads:  **BUCKYOS-DV-4827** |
| t2.sn-ai-provider.sn-us-full-20261030-t2.claude-opus-5.5.vision.caption | passed | t-e1b1a90ab3374c71a9959562ecaaa88c | 图中粉色花、黄色花心、绿色茎叶及棕色椭圆背景与描述相符。 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.claude-opus-5.5.vision.ocr | passed | t-e7c9dd507f9c4f4b8b574a7820c843ab | 预期 BUCKYOS-DV-4827；实际 The image contains this text:  **BUCKYOS-DV-4827**  Note: in this pixel-style font, the character after "Y" could be either the letter "O" or the digit "0". It reads most naturally as "BUCKYOS". |
| t2.sn-ai-provider.sn-us-full-20261030-t2.claude-sonnet-5.5.vision.caption | passed | t-33156e5e0e874b359d1be013acc8d46a | 图中粉色花、黄色花心、绿色茎叶及棕色椭圆背景与描述相符。 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.claude-sonnet-5.5.vision.ocr | passed | t-625c8563d52e4290b647fc7673d93046 | 预期 BUCKYOS-DV-4827；实际 BUCKYOS-DV-4827 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gemini-3.8-flash.audio.asr | passed | t-bca7c6f5a5b548bbbf8fb862242b9611 | 对照 OpenAI Whisper 官方 JFK 语音及测试断言：And so, my fellow Americans, ask not what your country can do for you, ask what you can do for your country. |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gemini-3.8-flash.vision.caption | passed | t-2b108227dd294f08928c4efbef746fb4 | 图中粉色花、黄色花心、绿色茎叶及棕色椭圆背景与描述相符。 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gemini-3.8-flash.vision.ocr | passed | t-9585f9a37ead40a190dc93f8079ce8ca | 预期 BUCKYOS-DV-4827；实际 BUCKYOS-DV-4827 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gemini-4-argon.audio.asr | passed | t-e306ba3a929a416384e4f39d0489a646 | 对照 OpenAI Whisper 官方 JFK 语音及测试断言：And so my fellow Americans, ask not what your country can do for you, ask what you can do for your country. |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gemini-4-argon.vision.caption | passed | t-b8b9d0e486c345f88b495a790dd90d8d | 图中粉色花、黄色花心、绿色茎叶及棕色椭圆背景与描述相符。 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gemini-4-argon.vision.ocr | passed | t-13d4e896e5cd4fb28a09d7aabf480637 | 预期 BUCKYOS-DV-4827；实际 BUCKYOS-DV-4827 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.glm-5.3-flash.vision.caption | passed | t-db9a89246c23492e9a1a0bbb9d278400 | 图中粉色花、黄色花心、绿色茎叶及棕色椭圆背景与描述相符。 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.glm-5.3-flash.vision.ocr | passed | t-81a57d9a559641eb97269ca750facaa7 | 预期 BUCKYOS-DV-4827；实际 BUCKYOS-DV-4827 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-4o-mini-transcribe.audio.asr | passed | t-ff4bd56df5ef4c80918b14ad1fc8231a | 对照 OpenAI Whisper 官方 JFK 语音及测试断言：And so, my fellow Americans: ask not what your country can do for you – ask what you can do for your country. |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-4o-transcribe-diarize.audio.asr | passed | t-e216daf2b00a4dfbaaa76f4f7e61d550 | 对照 OpenAI Whisper 官方 JFK 语音及测试断言：And so, my fellow Americans, ask not what your country can do for you, ask what you can do for your country. |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.3-codex.vision.caption | passed | t-030d4d0071284a748f8cf227b5054c87 | 图中粉色花、黄色花心、绿色茎叶及棕色椭圆背景与描述相符。 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.3-codex.vision.ocr | failed | t-c0562e3b662845d7ad9f6316681de658 | 预期 BUCKYOS-DV-4827；实际 6IICKY0S-0V-4827 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-mini.vision.caption | passed | t-67c87c1a83fa454386b5f7225b7c2f01 | 图中粉色花、黄色花心、绿色茎叶及棕色椭圆背景与描述相符。 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-mini.vision.ocr | failed | t-b299b553c11b4a349168c0667558c80d | 预期 BUCKYOS-DV-4827；实际 BUCKYNS-DV-4827 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-nano.vision.caption | passed | t-8600792cf7a040f49706eae941980f9e | 图中粉色花、黄色花心、绿色茎叶及棕色椭圆背景与描述相符。 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-nano.vision.ocr | failed | t-0d6fbbd4f83e40228d633fb98e65d9cf | 预期 BUCKYOS-DV-4827；实际 E I C K Y D T S E  O I H 4 E S F |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4.vision.caption | passed | t-522b5f10abce4f378f5187cd613e2414 | 图中粉色花、黄色花心、绿色茎叶及棕色椭圆背景与描述相符。 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4.vision.ocr | failed | t-632f911df913433691575b91cc5a119d | 预期 BUCKYOS-DV-4827；实际 6ICKYOS-0U-4827 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.5-pro.vision.caption | passed | t-2b6a5bdb07724f1389df3194558f55ed | 图中粉色花、黄色花心、绿色茎叶及棕色椭圆背景与描述相符。 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.5.vision.caption | failed | t-62fe4961b93b4f8bb99a8effaf608b47 | 主体正确，但将两片叶子描述为两对叶子。 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.5.vision.ocr | failed | t-158267333501404189c754a99f72c3fe | 预期 BUCKYOS-DV-4827；实际 BUCKYOS-0V-4827 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-luna.vision.caption | passed | t-8c4d43845f654c08b89140c36e3484ef | 图中粉色花、黄色花心、绿色茎叶及棕色椭圆背景与描述相符。 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-luna.vision.ocr | failed | t-11b7780603fa4217aa10197c815623c7 | 预期 BUCKYOS-DV-4827；实际 BLOCKY5 04-4827 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-sol.vision.caption | passed | t-7d94ccb0af1349c2912a407a77306e7d | 图中粉色花、黄色花心、绿色茎叶及棕色椭圆背景与描述相符。 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-sol.vision.ocr | failed | t-c492b1ebb9c74daea89432807cff7263 | 预期 BUCKYOS-DV-4827；实际 BUCKY06-04-4827 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-terra.vision.caption | passed | t-4ffbf62271ce4710bd46dd60bfa70f57 | 图中粉色花、黄色花心、绿色茎叶及棕色椭圆背景与描述相符。 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-terra.vision.ocr | failed | t-ec2e098f2ee8488a9deb7b778296845b | 预期 BUCKYOS-DV-4827；实际 FUCKTOS CV-4827 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-astra.vision.caption | passed | t-1276137a82b84f49ae7c1e218fd7df13 | 图中粉色花、黄色花心、绿色茎叶及棕色椭圆背景与描述相符。 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-astra.vision.ocr | passed | t-6d34076f12784eecb85c5cb4375236d8 | 预期 BUCKYOS-DV-4827；实际 BUCKYOS-DV-4827 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-luna.vision.caption | passed | t-81a89d6d4aa443ea995c83ac8e682e88 | 图中粉色花、黄色花心、绿色茎叶及棕色椭圆背景与描述相符。 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-luna.vision.ocr | failed | t-cd71e2eb9bd64ee2974c2db2f997c8d3 | 预期 BUCKYOS-DV-4827；实际 BINARYS-04-4827 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-sol.vision.caption | passed | t-a65c6b2dfc0c41718f70b640258856ed | 图中粉色花、黄色花心、绿色茎叶及棕色椭圆背景与描述相符。 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-sol.vision.ocr | failed | t-bfbbf85814334443abdb3791b23890b9 | 预期 BUCKYOS-DV-4827；实际 BICKYNS-04-4827 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6.1-sol.vision.caption | passed | t-cf7798de6b1448f99df39f60adf7eddb | 图中粉色花、黄色花心、绿色茎叶及棕色椭圆背景与描述相符。 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6.1-sol.vision.ocr | failed | t-0cc6e678b1814b248dc01394e953ca44 | 预期 BUCKYOS-DV-4827；实际 BUCKYOS-0V-4827 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6.1-terra.vision.caption | passed | t-4652450e019245958506b6cbb21d0e5e | 图中粉色花、黄色花心、绿色茎叶及棕色椭圆背景与描述相符。 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.grok-4.7.vision.caption | passed | t-3cd8a079cda64487bc91ce61c6aee042 | 图中粉色花、黄色花心、绿色茎叶及棕色椭圆背景与描述相符。 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.grok-4.7.vision.ocr | passed | t-43fd3548fec641fda5947f0fe4b11129 | 预期 BUCKYOS-DV-4827；实际 BUCKYOS-DV-4827 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.kimi-k3.1-pro.vision.caption | passed | t-a28b30a7090c4901844b3f5ccb52423a | 图中粉色花、黄色花心、绿色茎叶及棕色椭圆背景与描述相符。 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.kimi-k3.1-pro.vision.ocr | failed | t-ab948366a928421bbfae9da5c9623670 | 预期 BUCKYOS-DV-4827；实际 BUCKYOS-OV-4827 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.kimi-k3.vision.caption | passed | t-fd765f589ac34ba8bc4c68cd9fa31afb | 图中粉色花、黄色花心、绿色茎叶及棕色椭圆背景与描述相符。 |
| t2.sn-ai-provider.sn-us-full-20261030-t2.kimi-k3.vision.ocr | failed | t-35932c90d16344b09a7660290cbae55a | 预期 BUCKYOS-DV-4827；实际 BUCKY05-0V-4827 |
| t2.sn-ai-provider.sn-global-open-20261030-t2.kimi-k3.1-pro.vision.ocr | failed | t-1449a68e809a4697a1050ae487554d84 | 预期 BUCKYOS-DV-4827；实际 BUCKY05-0V-4827 |
| t2.sn-ai-provider.sn-global-open-20261030-t2.qwen3.7-plus.vision.ocr | failed | t-86b5052b175a4618b63fbbddf593c4b1 | 预期 BUCKYOS-DV-4827；实际 Based on the image provided, the visible text is:  **ELCFTIDE-DLU-48B27** |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-pro.vision.caption | passed | t-bb6e14a141d14fe1b9a510e2f83c5ce8 | 图中粉色花、黄色花心、绿色茎叶及棕色椭圆背景与描述相符。 |
