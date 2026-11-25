import { useEffect, useRef, useState } from "react";
import { SN_API_URL, waitForRegionProbe } from "../../active_lib";
import type { NetId, RegionProbeStatus } from "../types";

const REGION_PREFERENCE_STORAGE_KEY = "buckyos.node_active.region_preference";

/** The region decides which relay the account gets, so only these access routes show it. */
export const regionMatters = (netId: NetId) => netId === "nat" || netId === "portmap";

function unavailable(): RegionProbeStatus {
  return {
    phase: "unavailable",
    region: null,
    source: "none",
    config_version: null,
    confidence: "none",
    measured_at: null,
    expires_at: null,
    available_regions: [],
    regions: [],
  };
}

function loadPreference(): string {
  try {
    const value = JSON.parse(localStorage.getItem(REGION_PREFERENCE_STORAGE_KEY) || "null") as {
      sn_url?: unknown;
      region?: unknown;
    } | null;
    if (
      value?.sn_url === SN_API_URL &&
      typeof value.region === "string" &&
      /^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(value.region)
    ) {
      return value.region;
    }
  } catch {
    return "auto";
  }
  return "auto";
}

function savePreference(region: string, configVersion: string | null) {
  try {
    if (region === "auto") {
      localStorage.removeItem(REGION_PREFERENCE_STORAGE_KEY);
      return;
    }
    localStorage.setItem(
      REGION_PREFERENCE_STORAGE_KEY,
      JSON.stringify({ sn_url: SN_API_URL, region, config_version: configVersion }),
    );
  } catch {
    return;
  }
}

export type RegionProbe = {
  status: RegionProbeStatus | null;
  preference: string;
  choose: (preference: string) => void;
  retry: () => void;
  /** The region to register with; null leaves it to the server. */
  resolve: (netId: NetId) => Promise<string | null>;
};

export function useRegionProbe(enabled: boolean): RegionProbe {
  const [status, setStatus] = useState<RegionProbeStatus | null>(null);
  const [preference, setPreference] = useState(loadPreference);
  const inFlight = useRef<Promise<RegionProbeStatus> | null>(null);

  const run = (force: boolean): Promise<RegionProbeStatus> => {
    if (!force && inFlight.current) return inFlight.current;
    if (force) setStatus((previous) => (previous ? { ...previous, phase: "running" } : previous));
    const promise = waitForRegionProbe(force)
      .catch(unavailable)
      .then((next) => {
        if (inFlight.current === promise) setStatus(next);
        return next;
      })
      .finally(() => {
        if (inFlight.current === promise) inFlight.current = null;
      });
    inFlight.current = promise;
    return promise;
  };

  useEffect(() => {
    if (enabled) void run(false);
  }, [enabled]);

  useEffect(() => {
    if (
      preference !== "auto" &&
      status?.config_version &&
      !status.available_regions.some((region) => region.region_id === preference)
    ) {
      setPreference("auto");
      savePreference("auto", null);
    }
  }, [preference, status]);

  return {
    status,
    preference,
    choose: (next) => {
      setPreference(next);
      savePreference(next, status?.config_version ?? null);
    },
    retry: () => void run(true),
    resolve: async (netId) => {
      if (regionMatters(netId) && preference !== "auto") return preference;
      const finished = status && status.phase !== "running" && status.phase !== "idle" ? status : await run(false);
      return finished.region;
    },
  };
}
