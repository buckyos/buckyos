import type { AcceptanceCase, CaseReport } from "./types.ts";

export type CoverageStatus = "passed" | "failed" | "skipped" | "partial" | "unexecuted";

export type RequirementBranchCoverage = {
  branch_id: string;
  planned_case_ids: string[];
  executed_case_ids: string[];
  status: CoverageStatus;
};

export type T1Coverage = {
  total_branches: number;
  executed_branches: number;
  passed_branches: number;
  failed_branches: number;
  skipped_branches: number;
  coverage_rate: number;
  branches: RequirementBranchCoverage[];
  combination_groups: Array<{
    group_id: string;
    planned_cells: number;
    executed_cells: number;
    passed_cells: number;
    coverage_rate: number;
  }>;
};

function branchStatus(planned: AcceptanceCase[], reports: Map<string, CaseReport>): CoverageStatus {
  const executed = planned.filter((item) => reports.has(item.case_id));
  if (executed.length === 0) return "unexecuted";
  if (executed.length < planned.length) return "partial";
  const statuses = executed.map((item) => reports.get(item.case_id)?.status);
  if (statuses.every((status) => status === "passed")) return "passed";
  if (statuses.every((status) => status === "skipped")) return "skipped";
  return statuses.includes("failed") ? "failed" : "partial";
}

function addBranch(
  target: RequirementBranchCoverage[],
  branchId: string,
  planned: AcceptanceCase[],
  reports: Map<string, CaseReport>,
): void {
  if (planned.length === 0) return;
  target.push({
    branch_id: branchId,
    planned_case_ids: planned.map((item) => item.case_id),
    executed_case_ids: planned.filter((item) => reports.has(item.case_id)).map((item) => item.case_id),
    status: branchStatus(planned, reports),
  });
}

export function buildT1Coverage(
  manifest: AcceptanceCase[],
  caseReports: CaseReport[],
): T1Coverage {
  const reports = new Map(caseReports.map((item) => [item.case_id, item]));
  const branches: RequirementBranchCoverage[] = [];

  for (const item of manifest.filter((entry) => entry.case_id.startsWith("t1.route."))) {
    addBranch(branches, item.case_id.slice(3), [item], reports);
  }
  for (const item of manifest.filter((entry) => entry.case_id.startsWith("t1.scheduler.profile."))) {
    addBranch(branches, item.case_id.slice(3), [item], reports);
  }
  for (const item of manifest.filter((entry) => entry.case_id.startsWith("t1.history."))) {
    addBranch(branches, item.case_id.slice(3), [item], reports);
  }

  const protocol = manifest.filter((entry) => entry.case_id.startsWith("t1.protocol."));
  for (const method of [...new Set(protocol.map((item) => `${item.api_type}/${item.method}`))]) {
    addBranch(
      branches,
      `protocol.method.${method}`,
      protocol.filter((item) => `${item.api_type}/${item.method}` === method),
      reports,
    );
  }
  for (const scenario of [...new Set(protocol.map((item) => item.mock_scenario ?? "none"))]) {
    addBranch(
      branches,
      `protocol.scenario.${scenario}`,
      protocol.filter((item) => (item.mock_scenario ?? "none") === scenario),
      reports,
    );
  }

  for (const item of manifest.filter((entry) =>
    entry.case_id.startsWith("t1.task.") || entry.case_id.startsWith("t1.usage.") ||
    entry.case_id.startsWith("t1.security.") || entry.case_id.startsWith("t1.config.") ||
    entry.case_id.startsWith("t1.observability.") || entry.case_id.startsWith("t1.embedding.")
  )) {
    addBranch(branches, item.case_id.slice(3), [item], reports);
  }

  const combinationDefinitions = [
    ["route_constraints", (item: AcceptanceCase) => item.case_id.startsWith("t1.route.")],
    ["scheduler_profiles", (item: AcceptanceCase) => item.case_id.startsWith("t1.scheduler.profile.")],
    ["history_constraints", (item: AcceptanceCase) => item.case_id.startsWith("t1.history.")],
    ["api_method_x_mock_scenario", (item: AcceptanceCase) => item.case_id.startsWith("t1.protocol.")],
    ["cross_cutting", (item: AcceptanceCase) =>
      /t1\.(task|usage|security|config|observability)\./.test(item.case_id)],
    ["embedding_boundaries", (item: AcceptanceCase) => item.case_id.startsWith("t1.embedding.")],
  ] as const;
  const combinationGroups = combinationDefinitions.map(([groupId, predicate]) => {
    const planned = manifest.filter(predicate);
    const executed = planned.filter((item) => reports.has(item.case_id));
    return {
      group_id: groupId,
      planned_cells: planned.length,
      executed_cells: executed.length,
      passed_cells: executed.filter((item) => reports.get(item.case_id)?.status === "passed").length,
      coverage_rate: planned.length === 0 ? 1 : executed.length / planned.length,
    };
  });
  const executedBranches = branches.filter((item) => item.status !== "unexecuted").length;
  return {
    total_branches: branches.length,
    executed_branches: executedBranches,
    passed_branches: branches.filter((item) => item.status === "passed").length,
    failed_branches: branches.filter((item) => item.status === "failed").length,
    skipped_branches: branches.filter((item) => item.status === "skipped").length,
    coverage_rate: branches.length === 0 ? 1 : executedBranches / branches.length,
    branches,
    combination_groups: combinationGroups,
  };
}
