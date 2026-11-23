import { useEffect, useRef, useState } from "react";
import { checkDomainRecords, deriveDomainSetup, ensureSnSession } from "../../../active_lib";
import { checkKey, currentCheck } from "../../domain";
import { createPoller } from "../../poller";
import type { DomainCheck, WizardData } from "../../types";
import type { UpdateWizard } from "../ActiveWizard";

const CHECK_INTERVAL_MS = 10_000;
const SESSION_CODES = ["no_session", "bind_auth_required", "bind_invalid_token"];

/** Keeps `check_domain_records` running while the settings are complete enough to be checked. */
export function useDomainCheck(data: WizardData, enabled: boolean, onUpdate: UpdateWizard) {
  const [error, setError] = useState<unknown>(null);
  const dataRef = useRef(data);
  dataRef.current = data;
  const setup = enabled ? deriveDomainSetup(data) : null;
  const key = setup ? checkKey(setup, data.domain) : "";

  const poller = useRef<ReturnType<typeof createPoller<{ key: string; check: DomainCheck }>> | null>(null);
  if (!poller.current) {
    poller.current = createPoller({
      intervalMs: CHECK_INTERVAL_MS,
      hidden: () => document.hidden,
      run: async () => {
        const current = dataRef.current;
        const target = deriveDomainSetup(current);
        if (!target) throw new Error("domain setup is incomplete");
        const session = await ensureSnSession(current);
        if (session) onUpdate(session);
        const { credential_id, certificate_id } = current.domain;
        const check = await checkDomainRecords({
          setup: target,
          ...(session ? { access_token: session.sn_access_token } : {}),
          ...(target.mode === "dns_api" && credential_id ? { credential_id } : {}),
          ...(target.certificate === "upload" && certificate_id ? { certificate_id } : {}),
        });
        return { key: checkKey(target, current.domain), check };
      },
      finished: ({ check }) => check.ready,
      onError: setError,
      onResult: ({ key: checkedKey, check }) => {
        setError(null);
        const codes = check.items.flatMap((item) => [item.code, ...(item.targets ?? []).map((target) => target.code)]);
        onUpdate((previous) => ({
          ...(codes.some((code) => code !== null && SESSION_CODES.includes(code)) ? { sn_access_token_at: null } : {}),
          domain: {
            ...previous.domain,
            check,
            check_key: checkedKey,
            ...(codes.includes("credential_missing") ? { credential_id: null } : {}),
            ...(codes.includes("certificate_missing")
              ? { certificate_id: null, certificate_inspection: null, accept_warnings: false }
              : {}),
          },
        }));
      },
    });
  }

  useEffect(() => {
    const current = poller.current;
    setError(null);
    if (!key || currentCheck(setup, dataRef.current.domain)?.ready) {
      current?.stop();
      return;
    }
    current?.restart();
    return () => current?.stop();
  }, [key]);

  useEffect(() => {
    const onVisible = () => poller.current?.visible();
    document.addEventListener("visibilitychange", onVisible);
    return () => document.removeEventListener("visibilitychange", onVisible);
  }, []);

  return { error, recheck: () => key && poller.current?.restart() };
}
