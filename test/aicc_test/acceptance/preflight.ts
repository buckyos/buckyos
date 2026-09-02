import { readFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  assertCanonicalCompleteness,
  parseCanonicalApiTypesFromRequirements,
} from "./canonical.ts";
import { buildStaticManifest } from "./cases.ts";
import {
  assertTaxonomyConstants,
  validateCaseManifest,
  validateProviderBaseline,
} from "./manifest.ts";
import {
  buildT15Manifest,
  loadProviderProtocolCatalog,
  protocolContracts,
  REQUIRED_T15_PROVIDER_DRIVERS,
} from "./provider_protocol_contracts.ts";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, "../../..");

export type PreflightResult = {
  baseline_revision: string;
  canonical_api_types: number;
  static_cases: number;
  t15_cases: number;
  provider_drivers: string[];
};

function canonicalCheckoutBytes(bytes: Buffer): Buffer {
  const normalized: number[] = [];
  for (let index = 0; index < bytes.length; index += 1) {
    if (bytes[index] === 0x0d && bytes[index + 1] === 0x0a) continue;
    normalized.push(bytes[index]);
  }
  return Buffer.from(normalized);
}

export async function runPreflight(): Promise<PreflightResult> {
  const baselineRaw = JSON.parse(
    await readFile(join(here, "provider_capability_baseline.json"), "utf8"),
  );
  const baseline = validateProviderBaseline(baselineRaw);
  const requirementsSource = await readFile(
    join(repoRoot, "doc/aicc/aicc_e2e_test_requirements.md"),
    "utf8",
  );
  const sourceApiTypes = parseCanonicalApiTypesFromRequirements(requirementsSource);
  assertCanonicalCompleteness({ sourceApiTypes, baseline });
  const cases = validateCaseManifest(buildStaticManifest());
  const coveredT1ApiTypes = new Set(cases.filter((testCase) => testCase.layer === "T1" &&
    testCase.tags.includes("api_type")).flatMap((testCase) => testCase.api_type ? [testCase.api_type] : []));
  const missingT1ApiTypes = sourceApiTypes.filter((apiType) => !coveredT1ApiTypes.has(apiType));
  if (missingT1ApiTypes.length > 0) {
    throw new Error(`T1 routing manifest missing canonical API types: ${missingT1ApiTypes.join(", ")}`);
  }
  const protocolCatalog = await loadProviderProtocolCatalog();
  const t15Cases = validateCaseManifest(buildT15Manifest(protocolCatalog));
  const coveredT15ApiTypes = new Set(protocolContracts(protocolCatalog).flatMap((contract) => contract.api_types));
  const applicableApiTypes = new Set(baseline.providers.flatMap((provider) =>
    provider.rules.flatMap((rule) => rule.api_types)
  ));
  const missingT15ApiTypes = [...applicableApiTypes].filter((apiType) => !coveredT15ApiTypes.has(apiType));
  if (missingT15ApiTypes.length > 0) {
    throw new Error(`T1.5 protocol contracts missing applicable API types: ${missingT15ApiTypes.join(", ")}`);
  }
  const contractsByDriver = new Map(protocolCatalog.providers.map((provider) => [
    provider.provider_driver,
    new Set(provider.contracts.flatMap((contract) => contract.api_types)),
  ]));
  for (const provider of baseline.providers.filter((candidate) => !candidate.capability_source_provider)) {
    const requiredApiTypes = new Set(provider.rules.flatMap((rule) => rule.api_types));
    const covered = contractsByDriver.get(provider.provider_driver) ?? new Set<string>();
    const missing = [...requiredApiTypes].filter((apiType) => !covered.has(apiType));
    if (missing.length > 0) {
      throw new Error(`${provider.provider_driver} T1.5 contracts missing API types: ${missing.join(", ")}`);
    }
  }
  const contractIds = new Set(protocolContracts(protocolCatalog).map((contract) => contract.id));
  for (const contract of protocolContracts(protocolCatalog)) {
    if (contract.base_contract_id && !contractIds.has(contract.base_contract_id)) {
      throw new Error(`${contract.id} references missing base contract ${contract.base_contract_id}`);
    }
  }
  assertTaxonomyConstants();

  const fixtureManifest = JSON.parse(
    await readFile(join(here, "fixture_manifest.json"), "utf8"),
  ) as { fixtures?: unknown[] };
  if (!Array.isArray(fixtureManifest.fixtures) || fixtureManifest.fixtures.length === 0) {
    throw new Error("fixture manifest is empty");
  }
  const fixtureIds = new Set<string>();
  for (const raw of fixtureManifest.fixtures) {
    if (!raw || typeof raw !== "object") throw new Error("invalid fixture record");
    const fixture = raw as Record<string, unknown>;
    for (const field of ["id", "path", "mime", "size", "sha256", "facts", "cases", "source"]) {
      if (fixture[field] === undefined) throw new Error(`fixture missing ${field}`);
    }
    const id = String(fixture.id);
    if (fixtureIds.has(id)) throw new Error(`duplicate fixture id: ${id}`);
    fixtureIds.add(id);
    if (!Array.isArray(fixture.facts) || fixture.facts.length === 0 ||
        !Array.isArray(fixture.cases) || fixture.cases.length === 0) {
      throw new Error(`fixture facts/cases are empty: ${id}`);
    }
    if (typeof fixture.mime !== "string" || !fixture.mime.includes("/")) {
      throw new Error(`fixture MIME is invalid: ${id}`);
    }
    const bytes = await readFile(resolve(here, String(fixture.path)));
    const rawDigest = createHash("sha256").update(bytes).digest("hex");
    const rawMatches = bytes.byteLength === fixture.size && rawDigest === fixture.sha256;
    const canonicalBytes = rawMatches ? bytes : canonicalCheckoutBytes(bytes);
    const digest = createHash("sha256").update(canonicalBytes).digest("hex");
    if (!rawMatches && (canonicalBytes.byteLength !== fixture.size || digest !== fixture.sha256)) {
      throw new Error(`fixture integrity mismatch: ${id}`);
    }
    const legacyOfficeStream = new Map([
      ["application/msword", "WordDocument"],
      ["application/vnd.ms-excel", "Workbook"],
      ["application/vnd.ms-powerpoint", "PowerPoint Document"],
    ]).get(String(fixture.mime));
    if (legacyOfficeStream &&
      (!bytes.subarray(0, 8).equals(Buffer.from([0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1])) ||
        !bytes.includes(Buffer.from(legacyOfficeStream, "utf16le")))) {
      throw new Error(`fixture is not a genuine legacy Office ${legacyOfficeStream} container: ${id}`);
    }
  }
  const requiredFixtures = [
    "facts-txt", "facts-md", "facts-pdf", "facts-doc", "facts-docx", "facts-xls",
    "facts-xlsx", "facts-ppt", "facts-pptx",
    "facts-html", "facts-csv", "transparent-png", "mask-png", "marker-jpg",
    "speech-8khz-stereo-wav", "audio-sfx-wav", "audio-speech-wav", "video-fresh-mp4",
    "zip-single-document-zip", "zip-multiple-documents-zip", "archive-mixed-zip",
    "zip-corrupt-zip", "zip-encrypted-flag-zip", "zip-path-traversal-zip",
    "zip-deep-nesting-zip", "zip-many-files-zip", "zip-large-expansion-zip",
    "empty-bin", "mime-mismatch-png", "prompt-injection-md",
  ];
  const missingFixtures = requiredFixtures.filter((id) => !fixtureIds.has(id));
  if (missingFixtures.length > 0) {
    throw new Error(`fixture manifest missing required coverage: ${missingFixtures.join(", ")}`);
  }

  const baselineDrivers = new Set(
    baseline.providers.map((provider) => provider.provider_driver),
  );
  const missing = REQUIRED_T15_PROVIDER_DRIVERS.filter((driver) => !baselineDrivers.has(driver));
  if (missing.length > 0) {
    throw new Error(`provider baseline missing required drivers: ${missing.join(", ")}`);
  }

  return {
    baseline_revision: baseline.baseline_revision,
    canonical_api_types: sourceApiTypes.length,
    static_cases: cases.length,
    t15_cases: t15Cases.length,
    provider_drivers: [...baselineDrivers].sort(),
  };
}

if (
  process.argv[1] &&
  resolve(fileURLToPath(import.meta.url)) === resolve(process.argv[1])
) {
  runPreflight().then((result) => {
    process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
  }).catch((error) => {
    process.stderr.write(`AICC acceptance preflight failed: ${String(error)}\n`);
    process.exitCode = 1;
  });
}
