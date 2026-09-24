import type { ProviderBaseline } from "./types.ts";

export const CANONICAL_API_METHODS = {
  llm: ["chat.completions.create"],
  "embedding.text": ["embedding.text"],
  "embedding.multimodal": ["embedding.multimodal"],
  rerank: ["rerank"],
  "image.txt2img": ["images.generate"],
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
  sourceAssociations: ReadonlyMap<string, readonly string[]>;
  baseline: ProviderBaseline;
}): void {
  const expected = new Set(CANONICAL_API_TYPES);
  const source = new Set(args.sourceAssociations.keys());
  const baseline = new Set(args.baseline.canonical_api_types);
  const errors: string[] = [];
  for (const apiType of expected) {
    if (!source.has(apiType)) errors.push(`protocol missing ${apiType}`);
    if (!baseline.has(apiType)) errors.push(`baseline missing ${apiType}`);
    const expectedMethods = methodsForApiType(apiType);
    const sourceMethods = args.sourceAssociations.get(apiType) ?? [];
    if (expectedMethods.length !== sourceMethods.length ||
      expectedMethods.some((method) => !sourceMethods.includes(method))) {
      errors.push(
        `requirements association mismatch for ${apiType}: expected ${expectedMethods.join(", ")}, ` +
        `found ${sourceMethods.join(", ")}`,
      );
    }
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

export function parseCanonicalAssociationsFromRequirements(
  source: string,
): ReadonlyMap<string, readonly string[]> {
  const table = /当前 AICC canonical API type[\s\S]*?\n\| namespace \| canonical api_type \| typed method \|\n\|[-| ]+\|\n((?:\|[^\n]+\|\n?)+)/
    .exec(source)?.[1];
  if (!table) throw new Error("cannot find canonical API type table in requirements");
  const associations = new Map<string, readonly string[]>();
  for (const line of table.split("\n")) {
    const columns = line.split("|").map((value) => value.trim());
    if (columns.length < 5) continue;
    const apiTypes = [...columns[2].matchAll(/`([^`]+)`/g)].map((match) => match[1]);
    const explicitMethods = [...columns[3].matchAll(/`([^`]+)`/g)].map((match) => match[1]);
    const methods = columns[3].includes("同名 typed method") ? apiTypes : explicitMethods;
    if (apiTypes.length === 0 || methods.length !== apiTypes.length) {
      throw new Error(`invalid canonical API association row: ${line}`);
    }
    for (const [index, apiType] of apiTypes.entries()) {
      if (associations.has(apiType)) throw new Error(`duplicate canonical api_type ${apiType}`);
      associations.set(apiType, [methods[index]]);
    }
  }
  return associations;
}

export function parseCanonicalApiTypesFromRequirements(source: string): string[] {
  return [...parseCanonicalAssociationsFromRequirements(source).keys()];
}
