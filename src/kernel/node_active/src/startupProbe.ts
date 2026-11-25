import type { StartupCause } from "./startup";

export const PROBE_TIMEOUT_MS = 8_000;
const DIAGNOSE_TIMEOUT_MS = 5_000;
const DOH_URLS = ["https://dns.google/resolve", "https://cloudflare-dns.com/dns-query"];

async function fetchWithin(url: string, timeoutMs: number, init: RequestInit = {}): Promise<Response> {
  const abort = new AbortController();
  const timer = setTimeout(() => abort.abort(), timeoutMs);
  try {
    return await fetch(url, {
      cache: "no-store",
      mode: "cors",
      credentials: "omit",
      ...init,
      signal: abort.signal,
    });
  } finally {
    clearTimeout(timer);
  }
}

export async function probeReadme(accessHostname: string): Promise<boolean> {
  const response = await fetchWithin(`https://public.${accessHostname}/readme`, PROBE_TIMEOUT_MS, {
    redirect: "error",
  }).catch(() => null);
  return response?.status === 200;
}

function documentId(value: unknown): string | null {
  if (typeof value === "string") {
    try {
      const payload = value.split(".")[1].replace(/-/g, "+").replace(/_/g, "/");
      return documentId(JSON.parse(atob(payload)));
    } catch {
      return null;
    }
  }
  const id = (value as { id?: unknown } | null)?.id;
  return typeof id === "string" ? id : null;
}

/** `self` is the answering node's own Zone, so another BuckyOS node behind this address does not match. */
async function nodeAnswers(pageHostname: string, zoneDid: string): Promise<boolean> {
  try {
    const response = await fetchWithin(`http://${pageHostname}/1.0/identifiers/self`, DIAGNOSE_TIMEOUT_MS);
    if (response.status !== 200) return false;
    const answer = (await response.json()) as { didDocument?: unknown };
    return documentId(answer.didDocument ?? answer) === zoneDid;
  } catch {
    return false;
  }
}

/** null: no resolver could be asked, which says nothing about the domain. */
async function resolves(hostname: string): Promise<boolean | null> {
  for (const resolver of DOH_URLS) {
    try {
      for (const type of ["A", "AAAA"]) {
        const response = await fetchWithin(
          `${resolver}?name=${encodeURIComponent(hostname)}&type=${type}`,
          DIAGNOSE_TIMEOUT_MS,
          { headers: { accept: "application/dns-json" } },
        );
        if (!response.ok) throw new Error(`DoH status ${response.status}`);
        const answer = (await response.json()) as { Answer?: Array<{ type: number }> };
        if (answer.Answer?.some((record) => record.type === 1 || record.type === 28)) return true;
      }
      return false;
    } catch {
      continue;
    }
  }
  return null;
}

export async function diagnoseStartup(
  pageHostname: string,
  zoneDid: string,
  accessHostname: string,
): Promise<StartupCause> {
  if (!(await nodeAnswers(pageHostname, zoneDid))) return "node";
  if ((await resolves(`public.${accessHostname}`)) === false) return "dns";
  return "service";
}
