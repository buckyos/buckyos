import { parseToml } from "./config.ts";
import { SCENARIOS } from "./scenarios.ts";

function check(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

const config = parseToml(`
[common]
transports = ["msg-center", "telegram"]
cases = ["image_edit", "audio_sfx"]
interactive_review = true

[telegram]
api_id = 123456
bot_username = '@jarvis_test_bot'
`);
check(
  Array.isArray(config["common.transports"]) && config["common.transports"].length === 2,
  "TOML section parsing failed",
);
check(config["telegram.api_id"] === 123456, "TOML integer parsing failed");
check(config["common.interactive_review"] === true, "TOML boolean parsing failed");
check(
  Array.isArray(config["common.cases"]) && config["common.cases"].length === 2,
  "TOML array parsing failed",
);

const scenarioIds = SCENARIOS.map((scenario) => scenario.id);
check(scenarioIds.length === new Set(scenarioIds).size, "scenario ids must be unique");
check(
  SCENARIOS.filter((scenario) => scenario.suite === "matrix").length === 16,
  "matrix suite must contain 16 scenarios",
);
for (const scenario of SCENARIOS) {
  const seen = new Set<string>();
  for (const step of scenario.steps) {
    check(!seen.has(step.id), `${scenario.id} contains duplicate step ${step.id}`);
    if (step.replyToStep) {
      check(
        seen.has(step.replyToStep),
        `${scenario.id}/${step.id} references a step that has not been sent`,
      );
    }
    seen.add(step.id);
  }
}

console.log(
  `jarvis_media_dv self-check passed: ${SCENARIOS.length} scenarios, 16 matrix cases`,
);
