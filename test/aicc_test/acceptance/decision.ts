import { readFileSync } from "node:fs";

export const decisionFixture = JSON.parse(readFileSync(new URL("./fixtures/typesafe-systemone.json", import.meta.url), "utf8")) as {
  canonical_request: { state: unknown; questions: Array<Record<string, unknown>> };
  wire_request: Record<string, unknown>;
  wire_response: Record<string, unknown>;
};

export function decisionInput(): Record<string, unknown> {
  return structuredClone({ state: decisionFixture.canonical_request.state, questions: decisionFixture.canonical_request.questions });
}

export function decisionRouteRequirements(): Record<string, unknown> {
  const { state, questions } = decisionFixture.canonical_request;
  const bytes = (v: unknown) => new TextEncoder().encode(JSON.stringify(v)).length;
  return { decision: { question_types: ["choice", "score", "boolean"], structured_state: true, structured_rules: true,
    question_count: 3, max_options: 2, max_levels: 3, input_bytes: bytes(state) + bytes(questions),
    max_state_question_bytes: bytes(state) + Math.max(...questions.map(bytes)) } };
}

export function assertDecisionResult(value: unknown): void {
  const result = value as Record<string, unknown>;
  const answers = result?.answers as Array<Record<string, unknown>>;
  if (!Array.isArray(answers) || answers.length !== 3 || new Set(answers.map(a => a.id)).size !== 3) throw new Error("decision must return three distinct answers");
  const byId = Object.fromEntries(answers.map(answer => [String(answer.id), answer]));
  const probability = (v: unknown) => typeof v === "number" && Number.isFinite(v) && v >= 0 && v <= 1;
  for (const answer of answers) {
    if (answer.confidence !== undefined && !probability(answer.confidence)) throw new Error("invalid decision confidence");
  }
  for (const [id, keys] of [["marker", ["absent", "present"]], ["priority", ["0", "1", "2"]]] as const) {
    const probabilities = byId[id]?.probabilities as Record<string, number>;
    if (!probabilities || Object.keys(probabilities).sort().join() !== [...keys].sort().join() || Object.values(probabilities).some(p => !probability(p)) || Math.abs(Object.values(probabilities).reduce((a,b) => a+b, 0) - 1) > 1e-4) throw new Error("invalid decision distribution");
  }
  if (byId.marker?.type !== "choice" || byId.marker.selected !== "present") throw new Error("decision did not find the known marker");
  const score = byId.priority?.score;
  const probabilities = byId.priority?.probabilities as Record<string, number>;
  const expected = probabilities["1"] + 2 * probabilities["2"];
  if (byId.priority?.type !== "score" || typeof score !== "number" || !Number.isFinite(score) || score < 0 || score > 2 || Math.abs(score - expected) > 2e-4 || Math.abs(score - 1) > 0.5 || JSON.stringify(byId.priority.levels) !== JSON.stringify(["Routine", "Urgent", "Critical"])) throw new Error("decision score disagrees with its scale or stated priority");
  if (byId.contains?.type !== "boolean" || !probability(byId.contains.probability_true) || Number(byId.contains.probability_true) < 0.5) throw new Error("decision boolean did not preserve the true probability");
}

export const openrouterDecisionFixture = JSON.parse(readFileSync(new URL("./fixtures/openrouter-decisions.json", import.meta.url), "utf8")) as typeof decisionFixture;
