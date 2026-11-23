import { Cloud, KeyRound, ListChecks } from "lucide-react";
import {
  Alert,
  Box,
  Button,
  CircularProgress,
  FormControlLabel,
  Radio,
  RadioGroup,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  authorizeWalletDomainSession,
  deriveDomainSetup,
  deriveGatewayTopology,
  ensureSnSession,
  inspectDomain,
  isValidDomain,
  planDomainRecords,
} from "../../../active_lib";
import {
  DOMAIN_MODES,
  certificateAccepted,
  certificateSource,
  conflictOf,
  currentCheck,
  emptyDomainState,
  inspectionKey,
  recordStatus,
} from "../../domain";
import { errorText, type ErrorText } from "../../errors";
import type { CheckItem, DomainMode, DomainState, PlannedRecord, RecordPlan, WizardData } from "../../types";
import type { UpdateWizard } from "../ActiveWizard";
import { ChoiceCard, ErrorAlert, Section, StatusLine } from "../ui";
import CertificatePanel from "./CertificatePanel";
import CheckList from "./CheckList";
import DnsApiPanel, { type DnsFields } from "./DnsApiPanel";
import RecordCard from "./RecordCard";
import { useDomainCheck } from "./useDomainCheck";

type Props = {
  wizardData: WizardData;
  onUpdate: UpdateWizard;
  /** True while the domain in the field is not the one that was inspected. */
  onEditing: (editing: boolean) => void;
};

const CustomDomain = ({ wizardData, onUpdate, onEditing }: Props) => {
  const { t } = useTranslation();
  const [input, setInput] = useState(wizardData.self_domain);
  const [inspecting, setInspecting] = useState(false);
  const [inspectError, setInspectError] = useState<ErrorText | null>(null);
  const [authorizing, setAuthorizing] = useState(false);
  const [plan, setPlan] = useState<RecordPlan | null>(null);
  const [planError, setPlanError] = useState<ErrorText | null>(null);
  const [dnsFields, setDnsFields] = useState<DnsFields>({});

  const state = wizardData.domain;
  const ownerName = wizardData.owner_document?.name ?? "";
  const topology = deriveGatewayTopology(wizardData);
  const domain = input.trim().toLowerCase().replace(/\.$/, "");
  const editing = domain !== wizardData.self_domain;
  const wantedKey = inspectionKey(domain, topology.net_id, ownerName);
  const inspection = !editing && state.inspection_key === wantedKey ? state.inspection : null;
  const usable = inspection !== null && state.mode !== null && !inspection.blocked[state.mode];
  const setup = usable ? deriveDomainSetup(wizardData) : null;
  const check = currentCheck(setup, state);
  const checkable = setup !== null && (setup.mode !== "dns_api" || state.credential_id !== null);
  const { error: checkError, recheck } = useDomainCheck(wizardData, checkable, onUpdate);

  const patch = (partial: Partial<DomainState>) =>
    onUpdate((previous) => ({ domain: { ...previous.domain, ...partial } }));

  useEffect(() => {
    onEditing(editing || !usable);
  }, [editing, usable]);

  const inspect = async (askWallet: boolean) => {
    setInspectError(null);
    if (!isValidDomain(domain)) return setInspectError({ message: t("error_domain_format") });
    setInspecting(true);
    if (editing) {
      onUpdate({ use_self_domain: true, self_domain: domain, domain: emptyDomainState() });
    }
    try {
      let session = await ensureSnSession(wizardData);
      if (!session && askWallet && wizardData.is_wallet_runtime) {
        session = await authorizeWalletDomainSession(wizardData, domain).catch(() => null);
      }
      if (session) onUpdate(session);
      const result = await inspectDomain({
        domain,
        net_id: topology.net_id,
        owner_name: ownerName,
        ...(session ? { access_token: session.sn_access_token } : {}),
      });
      onUpdate((previous) => {
        const kept = previous.domain.mode;
        const mode =
          kept && !result.blocked[kept] ? kept : result.blocked[result.recommended] ? null : result.recommended;
        return {
          domain: {
            ...previous.domain,
            inspection: result,
            inspection_key: wantedKey,
            mode,
            provider: previous.domain.provider || result.provider || "",
          },
        };
      });
    } catch (cause) {
      setInspectError(errorText(t, cause));
    } finally {
      setInspecting(false);
    }
  };

  useEffect(() => {
    if (wizardData.self_domain && state.inspection_key !== wantedKey) void inspect(false);
  }, []);

  const planKey = setup ? JSON.stringify({ ...setup, certificate: null }) : "";
  const planRefresh = plan && plan.conflicts.length > 0 ? check?.checked_at ?? 0 : 0;
  useEffect(() => {
    if (!setup) {
      setPlan(null);
      return;
    }
    let cancelled = false;
    planDomainRecords(setup)
      .then((next) => {
        if (cancelled) return;
        setPlan(next);
        setPlanError(null);
      })
      .catch((cause) => {
        if (cancelled) return;
        setPlan(null);
        setPlanError(errorText(t, cause));
      });
    return () => {
      cancelled = true;
    };
  }, [planKey, planRefresh]);

  const authorize = async () => {
    setAuthorizing(true);
    try {
      onUpdate(await authorizeWalletDomainSession(wizardData, wizardData.self_domain));
      recheck();
    } catch (cause) {
      setInspectError(errorText(t, cause));
    } finally {
      setAuthorizing(false);
    }
  };

  const itemAction = (item: CheckItem) =>
    wizardData.is_wallet_runtime && item.status !== "pass" && item.code === "no_session" ? (
      <Button variant="outlined" size="small" onClick={authorize} disabled={authorizing} sx={{ mt: 1 }}>
        {t("wallet_authorize_button")}
      </Button>
    ) : null;

  const items = check?.items ?? [];
  const ownership = items.find((item) => item.id === "ownership");
  const ownershipPassed = ownership?.status === "pass";
  // SN answers the domain already but does not know whose it is, and nobody else can show it the TXT record.
  const hiddenBySn =
    ownership !== undefined &&
    !ownershipPassed &&
    (ownership.detail === "delegated_to_sn" ||
      items.some((item) => item.id === "delegation" && item.status === "pass"));
  const card = (record: PlannedRecord) => (
    <RecordCard
      key={`${record.name} ${record.type}`}
      record={record}
      zoneApex={inspection?.zone_apex ?? ""}
      status={setup ? recordStatus(record, setup.domain, items) : null}
      conflict={conflictOf(record, plan?.conflicts ?? [])}
    />
  );
  const ownershipRecords = plan?.records.filter((record) => record.phase === "ownership") ?? [];
  const routingRecords = plan?.records.filter((record) => record.phase === "routing") ?? [];

  const iconProps = { size: 20, strokeWidth: 1.75 };
  const modeCards: Record<DomainMode, { icon: JSX.Element; title: string; description: string }> = {
    sn: { icon: <Cloud {...iconProps} />, title: t("mode_sn"), description: t("mode_sn_desc") },
    dns_api: { icon: <KeyRound {...iconProps} />, title: t("mode_dns_api"), description: t("mode_dns_api_desc") },
    manual: { icon: <ListChecks {...iconProps} />, title: t("mode_manual"), description: t("mode_manual_desc") },
  };
  const checkNotes = check?.ready
    ? [setup?.certificate === "upload" && !certificateAccepted(state) ? "cert_confirm_pending" : "check_all_passed"]
    : [
        "check_auto",
        ...(items.some((item) => item.status === "waiting") ? ["check_wait_hint"] : []),
        ...(wizardData.is_wallet_runtime ? [] : ["check_leave_hint"]),
      ];
  const everyModeBlocked = inspection !== null && DOMAIN_MODES.every((mode) => inspection.blocked[mode]);
  const caaDenied = inspection !== null && !inspection.caa.allowed && setup?.certificate === "acme";

  return (
    <Stack spacing={4}>
      <Stack
        component="form"
        direction={{ xs: "column", sm: "row" }}
        spacing={1.5}
        alignItems={{ xs: "stretch", sm: "flex-start" }}
        onSubmit={(event) => {
          event.preventDefault();
          void inspect(true);
        }}
      >
        <TextField
          label={t("domain_placeholder")}
          value={input}
          autoComplete="off"
          placeholder="home.example.com"
          inputProps={{ autoCapitalize: "none", spellCheck: false, inputMode: "url" }}
          onChange={(event) => setInput(event.target.value)}
        />
        <Button
          type="submit"
          variant={inspection ? "outlined" : "contained"}
          disabled={inspecting || !domain}
          startIcon={inspecting ? <CircularProgress size={16} thickness={5} color="inherit" /> : undefined}
          sx={{ height: 56, flexShrink: 0, minWidth: 120 }}
        >
          {t("domain_inspect_button")}
        </Button>
      </Stack>

      {inspectError ? <ErrorAlert {...inspectError} /> : null}

      {inspection ? (
        <Stack spacing={1}>
          {inspection.delegated_to_sn ? (
            <StatusLine tone="success">{t("domain_delegated_to_sn")}</StatusLine>
          ) : inspection.ns.length > 0 ? (
            <StatusLine tone={inspection.provider ? "success" : "muted"}>
              {t(inspection.provider ? "domain_dns_known" : "domain_dns_other", { servers: inspection.ns.join(", ") })}
            </StatusLine>
          ) : null}
          {inspection.bound ? <StatusLine tone="success">{t("domain_already_bound")}</StatusLine> : null}
        </Stack>
      ) : null}

      {everyModeBlocked ? (
        <Alert
          severity="warning"
          action={
            <Button size="small" variant="text" onClick={() => setInput(`home.${domain}`)}>
              {t("domain_use_subdomain", { domain: `home.${domain}` })}
            </Button>
          }
        >
          {t("blocked_apex_needs_subdomain")}
        </Alert>
      ) : null}

      {inspection ? (
        <Section title={t("domain_mode_title")}>
          <Box
            role="radiogroup"
            aria-label={t("domain_mode_title")}
            sx={{ display: "grid", gridTemplateColumns: { xs: "1fr", md: "repeat(3, minmax(0, 1fr))" }, gap: 1.5 }}
          >
            {DOMAIN_MODES.map((mode) => {
              const blocked = inspection.blocked[mode];
              return (
                <ChoiceCard
                  key={mode}
                  selected={state.mode === mode}
                  disabled={blocked !== null}
                  onSelect={() => patch({ mode })}
                  icon={modeCards[mode].icon}
                  title={modeCards[mode].title}
                  description={blocked ? t([`blocked_${blocked}`, "code_unknown"]) : modeCards[mode].description}
                />
              );
            })}
          </Box>
        </Section>
      ) : null}

      {setup && setup.mode !== "manual" ? (
        <Section title={t("cert_source_title")}>
          <RadioGroup
            value={certificateSource(state)}
            onChange={(event) => patch({ certificate: event.target.value === "upload" ? "upload" : "acme" })}
          >
            <FormControlLabel value="acme" control={<Radio size="small" />} label={<Typography variant="body2">{t("cert_source_acme")}</Typography>} />
            <FormControlLabel value="upload" control={<Radio size="small" />} label={<Typography variant="body2">{t("cert_source_upload")}</Typography>} />
          </RadioGroup>
          {caaDenied ? <Alert severity="warning">{t("code_caa_denied")}</Alert> : null}
        </Section>
      ) : null}

      {planError ? <ErrorAlert {...planError} /> : null}

      {setup?.mode === "sn" && plan ? (
        <Section title={t("sn_records_title")}>
          <Typography variant="body2" color="text.secondary">
            {t("sn_step_txt")}
          </Typography>
          {ownershipRecords.map(card)}
          {hiddenBySn ? <Alert severity="info">{t("sn_withdraw_ns_hint")}</Alert> : null}
          <Typography variant="body2" color="text.secondary">
            {t(ownershipPassed ? "sn_step_ns" : "sn_step_ns_locked")}
          </Typography>
          {ownershipPassed ? routingRecords.map(card) : null}
        </Section>
      ) : null}

      {setup?.mode === "manual" && plan ? (
        <Section title={t("manual_records_title")}>
          <Typography variant="body2" color="text.secondary">
            {t("manual_records_intro", { zone: inspection?.zone_apex ?? "" })}
          </Typography>
          {plan.records.map(card)}
        </Section>
      ) : null}

      {setup?.mode === "dns_api" ? (
        <DnsApiPanel
          setup={setup}
          state={state}
          plan={plan}
          fields={dnsFields}
          onFields={setDnsFields}
          onState={patch}
          onApplied={recheck}
        />
      ) : null}

      {setup?.certificate === "upload" ? (
        <CertificatePanel domain={setup.domain} state={state} onState={patch} />
      ) : null}

      {checkable ? (
        <Section title={t("check_title")}>
          {items.length > 0 ? (
            <CheckList items={items} action={itemAction} />
          ) : (
            <StatusLine tone="accent" live>
              {t("check_running")}
            </StatusLine>
          )}
          {checkError ? <ErrorAlert {...errorText(t, checkError)} /> : null}
          <Stack spacing={0.5}>
            {checkNotes.map((key) => (
              <Typography key={key} variant="caption" color="text.secondary">
                {t(key)}
              </Typography>
            ))}
          </Stack>
        </Section>
      ) : null}
    </Stack>
  );
};

export default CustomDomain;
