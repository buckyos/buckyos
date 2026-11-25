import type {
  ActiveWizzardData,
  CertificateInspection,
  CheckItem,
  CheckStatus,
  DomainCheck,
  DomainMode,
  DomainSetup,
  DomainState,
  GatewayTopology,
  PlannedRecord,
  RecordConflict,
} from "./types";

export const DOMAIN_MODES: DomainMode[] = ["sn", "dns_api", "manual"];

export function emptyDomainState(): DomainState {
  return {
    inspection: null,
    inspection_key: "",
    mode: null,
    certificate: "acme",
    provider: "",
    credential_id: null,
    certificate_id: null,
    certificate_inspection: null,
    accept_warnings: false,
    check: null,
    check_key: "",
  };
}

export function inspectionKey(domain: string, netId: string, ownerName: string): string {
  return JSON.stringify([domain, netId, ownerName]);
}

/** Manual hosting has no ACME challenge record, so its certificate is always uploaded. */
export function certificateSource(state: DomainState) {
  return state.mode === "manual" ? "upload" : state.certificate;
}

export function buildDomainSetup(data: ActiveWizzardData, topology: GatewayTopology): DomainSetup | null {
  const { domain } = data;
  if (!data.use_self_domain || !data.self_domain || !domain.mode || !data.owner_document) return null;
  return {
    domain: data.self_domain,
    mode: domain.mode,
    certificate: certificateSource(domain),
    net_id: topology.net_id,
    public_ips: topology.public_ips,
    owner_name: data.owner_document.name,
  };
}

export function checkKey(setup: DomainSetup, state: DomainState): string {
  return JSON.stringify([
    setup,
    setup.mode === "dns_api" ? state.credential_id : null,
    setup.certificate === "upload" ? state.certificate_id : null,
  ]);
}

/** The last check, only while it still describes the current settings. */
export function currentCheck(setup: DomainSetup | null, state: DomainState): DomainCheck | null {
  return setup && state.check && state.check_key === checkKey(setup, state) ? state.check : null;
}

export function certificateAccepted(state: DomainState): boolean {
  const inspection = state.certificate_inspection;
  if (!state.certificate_id || !inspection) return false;
  return state.accept_warnings || !inspection.items.some((item) => item.level === "warn");
}

export function domainReady(data: ActiveWizzardData, setup: DomainSetup | null): boolean {
  if (!setup) return false;
  if (setup.mode === "dns_api" && !data.domain.credential_id) return false;
  if (setup.certificate === "upload" && !certificateAccepted(data.domain)) return false;
  return currentCheck(setup, data.domain)?.ready === true;
}

/** Nobody but clients that trust the certificate can connect (self-signed, private CA, broken chain). */
export function certificateUntrusted(data: ActiveWizzardData): boolean {
  return (
    data.use_self_domain &&
    certificateSource(data.domain) === "upload" &&
    data.domain.certificate_inspection?.publicly_trusted === false
  );
}

const FIXED_HOSTS = ["ood1", "sys", "public"];

function worst(statuses: CheckStatus[]): CheckStatus | null {
  for (const status of ["fail", "waiting", "skipped", "pass"] as const) {
    if (statuses.includes(status)) return status;
  }
  return null;
}

/** The check result that belongs to one record card. */
export function recordStatus(
  record: PlannedRecord,
  domain: string,
  items: CheckItem[],
): CheckStatus | null {
  const item = (id: CheckItem["id"]) => items.find((entry) => entry.id === id);
  if (record.id === "pkx") return item("ownership")?.status ?? null;
  if (record.id === "ns") return item("delegation")?.status ?? null;
  const entry = item("entry");
  if (!entry) return null;
  if (!entry.targets?.length) return entry.status;
  const own = entry.targets.filter((target) =>
    record.id === "entry"
      ? target.host === domain
      : target.host !== domain && !FIXED_HOSTS.some((label) => target.host === `${label}.${domain}`),
  );
  return worst(own.map((target) => target.status)) ?? "pass";
}

export function conflictOf(record: PlannedRecord, conflicts: RecordConflict[]): RecordConflict | null {
  return (
    conflicts.find((conflict) => conflict.name === record.name && conflict.planned.type === record.type) ?? null
  );
}

/** What most DNS consoles want in the name column; null when it is the full name anyway. */
export function relativeName(name: string, zoneApex: string): string | null {
  if (name === zoneApex) return "@";
  return name.endsWith(`.${zoneApex}`) ? name.slice(0, -zoneApex.length - 1) : null;
}

export function isPublicIPv4(value: string): boolean {
  const parts = value.split(".");
  if (parts.length !== 4 || !parts.every((part) => /^(0|[1-9]\d{0,2})$/.test(part) && Number(part) <= 255)) {
    return false;
  }
  const [a, b] = parts.map(Number);
  return !(
    a === 0 ||
    a === 10 ||
    a === 127 ||
    a >= 224 ||
    (a === 100 && b >= 64 && b <= 127) ||
    (a === 169 && b === 254) ||
    (a === 172 && b >= 16 && b <= 31) ||
    (a === 192 && b === 168)
  );
}

export function isPublicIPv6(value: string): boolean {
  if (!/^[0-9a-f:]+$/i.test(value) || !value.includes(":")) return false;
  try {
    new URL(`http://[${value}]/`);
  } catch {
    return false;
  }
  return /^[23][0-9a-f]{3}:/i.test(value);
}

export function rejectedItems(inspection: CertificateInspection) {
  return inspection.items.filter((item) => item.level === "reject");
}

/** `[active:<code>] detail` from node_daemon, wherever the RPC layer put its own prefix. */
export function activeErrorOf(error: unknown): { code: string; detail: string } | null {
  const message = error instanceof Error ? error.message : String(error);
  const match = /\[active:([a-z0-9_]+)\]\s*([\s\S]*)$/.exec(message);
  return match ? { code: match[1], detail: match[2] } : null;
}

/** The failing items `commit_active` attaches to `domain_not_ready` and `bind_<name>`. */
export function checkItemsOf(detail: string): CheckItem[] {
  const json = /[[{][\s\S]*[\]}]/.exec(detail)?.[0] ?? "";
  try {
    const parsed = JSON.parse(json) as unknown;
    const items = Array.isArray(parsed) ? parsed : (parsed as { items?: unknown } | null)?.items;
    return Array.isArray(items) ? (items as CheckItem[]) : [];
  } catch {
    return [];
  }
}
