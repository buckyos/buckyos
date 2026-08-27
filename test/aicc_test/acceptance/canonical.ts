import type { ProviderBaseline } from "./types.ts";

export const CANONICAL_API_METHODS = {
  llm: ["llm.chat"],
  "embedding.text": ["embedding.text"],
  "embedding.multimodal": ["embedding.multimodal"],
  rerank: ["rerank"],
  "image.txt2img": ["image.txt2img"],
  "image.img2img": ["image.img2img"],
  "image.inpaint": ["image.inpaint"],
  "image.upscale": ["image.upscale"],
  "image.bg_remove": ["image.bg_remove"],
  "vision.ocr": ["vision.ocr"],
  "vision.caption": ["vision.caption"],
  "vision.detect": ["vision.detect"],
  "vision.segment": ["vision.segment"],
  "audio.tts": ["audio.tts"],
  "audio.asr": ["audio.asr"],
  "audio.music": ["audio.music"],
  "audio.enhance": ["audio.enhance"],
  "video.txt2video": ["video.txt2video"],
  "video.img2video": ["video.img2video"],
  "video.video2video": ["video.video2video"],
  "video.extend": ["video.extend"],
  "video.upscale": ["video.upscale"],
  "agent.computer_use": ["agent.computer_use"],
} as const;

export type CanonicalApiType = keyof typeof CANONICAL_API_METHODS;

export const CANONICAL_API_TYPES = Object.freeze(
  Object.keys(CANONICAL_API_METHODS) as CanonicalApiType[],
);

export function methodsForApiType(apiType: string): readonly string[] {
  return CANONICAL_API_METHODS[apiType as CanonicalApiType] ?? [];
}

export function assertCanonicalCompleteness(args: {
  sourceApiTypes: Iterable<string>;
  baseline: ProviderBaseline;
}): void {
  const expected = new Set(CANONICAL_API_TYPES);
  const source = new Set(args.sourceApiTypes);
  const baseline = new Set(args.baseline.canonical_api_types);
  const errors: string[] = [];
  for (const apiType of expected) {
    if (!source.has(apiType)) errors.push(`protocol missing ${apiType}`);
    if (!baseline.has(apiType)) errors.push(`baseline missing ${apiType}`);
  }
  for (const apiType of source) {
    if (!expected.has(apiType as CanonicalApiType)) {
      errors.push(`requirements missing protocol api_type ${apiType}`);
    }
  }
  for (const apiType of baseline) {
    if (!expected.has(apiType as CanonicalApiType)) {
      errors.push(`requirements missing baseline api_type ${apiType}`);
    }
  }
  if (errors.length > 0) throw new Error(errors.join("; "));
}

export function parseCanonicalApiTypesFromRust(source: string): string[] {
  const enumMatch = /pub enum ApiType\s*\{([\s\S]*?)\n\}/m.exec(source);
  if (!enumMatch) throw new Error("cannot find ApiType enum in model_types.rs");
  return Array.from(
    enumMatch[1].matchAll(/#\[serde\(rename\s*=\s*"([^"]+)"\)\]/g),
    (match) => match[1],
  );
}
