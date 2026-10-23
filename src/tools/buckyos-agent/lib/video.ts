// Shared pieces of the video commands: repeatable condition flags, the
// `output` spec flags, and picking the one generated video from a result.
// Protocol: doc/aicc/video_generation.md.

import type { buckyos } from "buckyos";

import { ArgError, flagFloat, flagInt, requireString } from "./cli.ts";
import { resolveInputResource } from "./io.ts";
import { aiResponseArtifacts } from "./types.ts";
import type { AiArtifact, AiccInferenceResponse } from "./types.ts";

type SdkAiccClient = ReturnType<typeof buckyos.getAiccClient>;
type VideoGenerateRequest = Parameters<SdkAiccClient["videoGenerate"]>[0];
export type VideoCondition = NonNullable<
  VideoGenerateRequest["conditions"]
>[number];
export type VideoOutputSpec = NonNullable<VideoGenerateRequest["output"]>;

export const VIDEO_CONDITION_FLAGS = [
  "ref-image",
  "ref-video",
  "ref-audio",
  "clip",
  "with-audio",
] as const;

export const VIDEO_OUTPUT_HELP = `Output:
  --duration <seconds>
  --aspect <W:H>                  e.g. 16:9, 9:16, 21:9
  --short-edge <px>               e.g. 720, 1080; or
  --size <WxH>                    an exact size such as 1280x720
  --fps <n>
  --audio <auto|generate|none>    audio track; none is refused by models that always add one
  --format <mp4|webm|mov>`;

// Removes every `--<name> <value>` of `names` from argv, keeping the order
// the values were given in; the ordinary parser keeps only the last one.
export function takeRepeatedFlags(
  argv: string[],
  names: readonly string[],
): { rest: string[]; values: Map<string, string[]> } {
  const values = new Map<string, string[]>(names.map((name) => [name, []]));
  const rest: string[] = [];
  for (let index = 0; index < argv.length; index += 1) {
    const name = argv[index].startsWith("--") ? argv[index].slice(2) : "";
    if (!values.has(name)) {
      rest.push(argv[index]);
      continue;
    }
    const value = argv[index + 1];
    if (value === undefined || value.startsWith("--")) {
      throw new ArgError(`--${name} requires a value`);
    }
    values.get(name)!.push(value);
    index += 1;
  }
  return { rest, values };
}

function splitAssignment(flag: string, value: string): [string, string] {
  const at = value.indexOf("=");
  if (at <= 0 || at === value.length - 1) {
    throw new ArgError(`--${flag} expects <id>=<value>, got ${value}`);
  }
  return [value.slice(0, at), value.slice(at + 1)];
}

// Builds `conditions[]` from the repeatable reference flags plus the
// keyframes, in the order: keyframes, then references as given.
export async function videoConditions(
  values: Map<string, string[]>,
  keyframes: { start?: string; end?: string },
): Promise<VideoCondition[]> {
  const conditions: VideoCondition[] = [];
  for (const [position, id] of [["start", "first"], ["end", "last"]] as const) {
    const source = keyframes[position];
    if (source) {
      conditions.push({
        type: "keyframe",
        id,
        position,
        resource: await resolveInputResource(source, "image/*"),
      });
    }
  }
  const kinds = [
    ["ref-image", "reference_image", "image/*"],
    ["ref-video", "reference_video", "video/*"],
    ["ref-audio", "reference_audio", "audio/*"],
  ] as const;
  for (const [flag, type, mime] of kinds) {
    for (const value of values.get(flag) ?? []) {
      const [id, source] = splitAssignment(flag, value);
      conditions.push(
        { type, id, resource: await resolveInputResource(source, mime) } as
          VideoCondition,
      );
    }
  }
  const find = (flag: string, id: string) => {
    const condition = conditions.find((condition) => condition.id === id);
    if (!condition) throw new ArgError(`--${flag} names unknown condition ${id}`);
    return condition;
  };
  for (const value of values.get("clip") ?? []) {
    const [id, range] = splitAssignment("clip", value);
    const match = /^([0-9.]+)-([0-9.]+)$/.exec(range);
    const condition = find("clip", id);
    if (
      !match ||
      (condition.type !== "reference_video" &&
        condition.type !== "reference_audio")
    ) {
      throw new ArgError(
        `--clip expects <video_or_audio_id>=<start>-<end>, got ${value}`,
      );
    }
    condition.time_range = {
      start_seconds: Number(match[1]),
      end_seconds: Number(match[2]),
    };
  }
  for (const id of values.get("with-audio") ?? []) {
    const condition = find("with-audio", id);
    if (condition.type !== "reference_video") {
      throw new ArgError(`--with-audio applies to a --ref-video id, got ${id}`);
    }
    condition.include_audio = true;
  }
  return conditions;
}

const FORMATS: Record<string, string> = {
  mp4: "video/mp4",
  webm: "video/webm",
  mov: "video/quicktime",
};

// The `output` spec from the output flags, or undefined when none is given.
export function videoOutputSpec(
  flags: Map<string, string | true>,
): VideoOutputSpec | undefined {
  const output: VideoOutputSpec = {};
  const duration = flagFloat(flags, "duration");
  if (duration !== undefined) output.duration_seconds = duration;
  const aspect = requireString(flags, "aspect");
  if (aspect !== undefined) output.aspect_ratio = aspect;
  const shortEdge = flagInt(flags, "short-edge");
  const size = requireString(flags, "size");
  if (shortEdge !== undefined && size !== undefined) {
    throw new ArgError("--short-edge and --size are alternatives");
  }
  if (shortEdge !== undefined) output.size = { short_edge: shortEdge };
  if (size !== undefined) {
    const match = /^(\d+)x(\d+)$/.exec(size);
    if (!match) throw new ArgError(`--size expects <W>x<H>, got ${size}`);
    output.size = { width: Number(match[1]), height: Number(match[2]) };
  }
  const fps = flagFloat(flags, "fps");
  if (fps !== undefined) output.fps = fps;
  const audio = requireString(flags, "audio");
  if (audio !== undefined) {
    if (audio !== "auto" && audio !== "generate" && audio !== "none") {
      throw new ArgError(`--audio must be auto|generate|none, got ${audio}`);
    }
    output.audio = audio;
  }
  const format = requireString(flags, "format");
  if (format !== undefined) {
    if (!FORMATS[format]) throw new ArgError(`--format invalid: ${format}`);
    output.media_type = FORMATS[format];
  }
  return Object.keys(output).length ? output : undefined;
}

// The single generated video of a result. More than one is an error rather
// than being cut down to the first.
export function pickVideo(response: AiccInferenceResponse): AiArtifact | null {
  const videos = aiResponseArtifacts(response).filter((artifact) =>
    /^video_\d+$/.test(artifact.name)
  );
  if (videos.length > 1) {
    throw new Error(
      `the result holds ${videos.length} videos but one was requested`,
    );
  }
  return videos[0] ?? null;
}
