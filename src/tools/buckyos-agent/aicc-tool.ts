// aicc-tool — single entry dispatcher for all AICC agent CLI commands.
//
// Two invocation styles, per doc §10.1:
//
//   aicc-tool <command> [args...]      # explicit dispatch
//   <command> [args...]                # if argv[0] basename matches a command
//                                      # (works for both `<command> a b c` and
//                                      # `deno run aicc-tool.ts a b c` after
//                                      # being symlinked).

import { HelpRequested } from "./lib/cli.ts";

import * as gen_image from "./commands/gen_image.ts";
import * as edit_image from "./commands/edit_image.ts";
import * as inpaint_image from "./commands/inpaint_image.ts";
import * as upscale_image from "./commands/upscale_image.ts";
import * as remove_bg from "./commands/remove_bg.ts";
import * as ocr_image from "./commands/ocr_image.ts";
import * as caption_image from "./commands/caption_image.ts";
import * as detect_image from "./commands/detect_image.ts";
import * as segment_image from "./commands/segment_image.ts";
import * as text_to_speech from "./commands/text_to_speech.ts";
import * as speech_to_text from "./commands/speech_to_text.ts";
import * as gen_music from "./commands/gen_music.ts";
import * as enhance_audio from "./commands/enhance_audio.ts";
import * as gen_video from "./commands/gen_video.ts";
import * as img2video from "./commands/img2video.ts";
import * as video2video from "./commands/video2video.ts";
import * as extend_video from "./commands/extend_video.ts";
import * as upscale_video from "./commands/upscale_video.ts";
import * as ai_provider from "./commands/ai_provider.ts";
import * as ai_quota from "./commands/ai_quota.ts";

interface Command {
  run: (argv: string[]) => Promise<never>;
  HELP: string;
}

const COMMANDS: Record<string, Command> = {
  gen_image,
  edit_image,
  inpaint_image,
  upscale_image,
  remove_bg,
  ocr_image,
  caption_image,
  detect_image,
  segment_image,
  text_to_speech,
  speech_to_text,
  gen_music,
  enhance_audio,
  gen_video,
  img2video,
  video2video,
  extend_video,
  upscale_video,
  ai_provider,
  ai_quota,
};

function topLevelHelp(): string {
  const names = Object.keys(COMMANDS).sort();
  return [
    "Usage: aicc-tool <command> [args...]",
    "",
    "Available commands:",
    ...names.map((n) => `  ${n}`),
    "",
    "Run `aicc-tool <command> --help` for command-specific options.",
  ].join("\n");
}

function basename(p: string): string {
  const i = p.lastIndexOf("/");
  return (i >= 0 ? p.slice(i + 1) : p).replace(/\.[tj]s$/i, "");
}

async function main(): Promise<void> {
  const argv = [...Deno.args];

  // If invoked via a symlinked binary whose name matches a command, treat
  // that name as the subcommand. Otherwise take argv[0] as the subcommand.
  const launcher = basename(Deno.mainModule);
  let subcommand: string | undefined;
  if (launcher in COMMANDS) {
    subcommand = launcher;
  } else {
    subcommand = argv.shift();
  }

  if (!subcommand || subcommand === "-h" || subcommand === "--help" || subcommand === "help") {
    console.error(topLevelHelp());
    Deno.exit(subcommand ? 0 : 1);
  }

  const cmd = COMMANDS[subcommand];
  if (!cmd) {
    console.error(`unknown command: ${subcommand}\n\n${topLevelHelp()}`);
    Deno.exit(1);
  }

  try {
    await cmd.run(argv);
  } catch (err) {
    if (err instanceof HelpRequested) {
      console.error(cmd.HELP);
      Deno.exit(0);
    }
    const msg = err instanceof Error ? err.stack ?? err.message : String(err);
    console.error(`aicc-tool ${subcommand} crashed: ${msg}`);
    Deno.exit(1);
  }
}

await main();
