import { ExternalLink } from "lucide-react";
import {
  Alert,
  Box,
  Button,
  Checkbox,
  CircularProgress,
  FormControlLabel,
  MenuItem,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { applyDomainRecords, listDnsProviders } from "../../../active_lib";
import { conflictOf } from "../../domain";
import { codeText, errorText, type ErrorText } from "../../errors";
import type {
  DnsProviderInfo,
  DomainSetup,
  DomainState,
  RecordApplyResult,
  RecordPlan,
  RecordWriteResult,
} from "../../types";
import { openExternalUrl } from "../../utils/external";
import { ErrorAlert, Section, StatusDot, StatusLine, mono, type Tone } from "../ui";
import { formatRecord } from "./RecordCard";

const PROVIDER_GUIDE_URL = "https://github.com/buckyos/buckyos/blob/main/doc/user/dns-providers";

export type DnsFields = Record<string, Record<string, string>>;

const resultTone: Record<RecordWriteResult["status"], Tone> = {
  written: "success",
  unchanged: "success",
  needs_confirm: "warning",
  failed: "danger",
};

type Props = {
  setup: DomainSetup;
  state: DomainState;
  plan: RecordPlan | null;
  fields: DnsFields;
  onFields: (fields: DnsFields) => void;
  onState: (partial: Partial<DomainState>) => void;
  onApplied: () => void;
};

const DnsApiPanel = ({ setup, state, plan, fields, onFields, onState, onApplied }: Props) => {
  const { t } = useTranslation();
  const [providers, setProviders] = useState<DnsProviderInfo[] | null>(null);
  const [confirmed, setConfirmed] = useState<string[]>([]);
  const [replaceToken, setReplaceToken] = useState(false);
  const [applied, setApplied] = useState<RecordApplyResult | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<ErrorText | null>(null);

  useEffect(() => {
    let cancelled = false;
    listDnsProviders()
      .then((list) => !cancelled && setProviders(list))
      .catch((cause) => !cancelled && setError(errorText(t, cause)));
    return () => {
      cancelled = true;
    };
  }, []);

  const setupKey = JSON.stringify(setup);
  useEffect(() => {
    setApplied(null);
    setConfirmed([]);
  }, [setupKey]);

  const detected = state.inspection?.provider ?? null;
  const ordered = [...(providers ?? [])].sort((a, b) => Number(b.id === detected) - Number(a.id === detected));
  const provider = providers?.find((entry) => entry.id === state.provider) ?? null;
  const values = provider ? fields[provider.id] ?? {} : {};
  const hasCredential = state.credential_id !== null && !replaceToken;
  const tokenLost =
    state.credential_id === null && state.check?.items.some((item) => item.code === "credential_missing") === true;
  const formComplete =
    provider !== null && provider.fields.every((field) => field.optional || (values[field.id] ?? "").trim() !== "");
  const conflicts = (plan?.conflicts ?? []).filter(
    (conflict) =>
      !applied ||
      applied.conflicts.some((open) => open.name === conflict.name && open.planned.type === conflict.planned.type),
  );
  const conflictNames = [...new Set(conflicts.map((conflict) => conflict.name))];
  const steps = provider
    ? (t(`steps.${provider.id}`, { ns: "dns_providers", returnObjects: true, defaultValue: [] }) as unknown)
    : [];

  const chooseProvider = (id: string) => {
    onState({ provider: id, credential_id: null });
    setApplied(null);
    setReplaceToken(false);
    setError(null);
  };

  const apply = async () => {
    if (!provider) return;
    setBusy(true);
    setError(null);
    const confirm = confirmed.filter((name) => conflictNames.includes(name));
    try {
      const result = await applyDomainRecords(
        hasCredential
          ? { setup, credential_id: state.credential_id as string, confirm }
          : {
              setup,
              provider: provider.id,
              fields: Object.fromEntries(
                provider.fields
                  .map((field) => [field.id, (values[field.id] ?? "").trim()])
                  .filter(([, value]) => value !== ""),
              ),
              confirm,
            },
      );
      setApplied(result);
      if (result.credential_id) {
        onState({ credential_id: result.credential_id });
        onFields({ ...fields, [provider.id]: {} });
        setReplaceToken(false);
      } else {
        const code = result.results.find((entry) => entry.code)?.code ?? "credential_rejected";
        setError({ message: codeText(t, code) });
      }
      onApplied();
    } catch (cause) {
      setError(errorText(t, cause));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Stack spacing={3}>
      <Section title={t("dns_provider_title")}>
        <TextField
          select
          label={t("dns_provider_label")}
          value={provider ? provider.id : ""}
          onChange={(event) => chooseProvider(event.target.value)}
          disabled={!providers}
          SelectProps={{ renderValue: () => provider?.name ?? "" }}
        >
          {ordered.map((entry) => (
            <MenuItem key={entry.id} value={entry.id} sx={{ gap: 1.5 }}>
              <span>{entry.name}</span>
              <Typography component="span" variant="body2" color="text.secondary">
                {[
                  entry.id === detected ? t("dns_provider_detected") : "",
                  entry.tested ? "" : t("dns_provider_experimental"),
                ]
                  .filter(Boolean)
                  .join(" · ")}
              </Typography>
            </MenuItem>
          ))}
        </TextField>
        {!provider && providers ? (
          <Typography variant="body2" color="text.secondary">
            {t("dns_provider_other")}
          </Typography>
        ) : null}
        {provider && !provider.tested ? (
          <Typography variant="body2" color="text.secondary">
            {t("dns_provider_experimental_hint")}
          </Typography>
        ) : null}
        {provider?.token_scope === "account" ? <Alert severity="warning">{t("dns_provider_account_scope")}</Alert> : null}
      </Section>

      {provider && hasCredential ? (
        <Section title={t("dns_token_title")}>
          <Stack direction="row" spacing={2} alignItems="center" justifyContent="space-between">
            <StatusLine tone="success">{t("dns_token_kept", { provider: provider.name })}</StatusLine>
            <Button variant="text" size="small" onClick={() => setReplaceToken(true)} sx={{ flexShrink: 0 }}>
              {t("dns_token_replace")}
            </Button>
          </Stack>
        </Section>
      ) : null}

      {provider && !hasCredential ? (
        <Section title={t("dns_token_title")}>
          {tokenLost ? <Alert severity="warning">{t("code_credential_missing")}</Alert> : null}
          {Array.isArray(steps) && steps.length > 0 ? (
            <Box component="ol" sx={{ m: 0, pl: 2.5, color: "text.secondary", fontSize: 14, lineHeight: 1.6 }}>
              {(steps as string[]).map((step) => (
                <li key={step}>{step}</li>
              ))}
            </Box>
          ) : null}
          <Stack direction="row" spacing={1} flexWrap="wrap" useFlexGap>
            <Button
              variant="outlined"
              endIcon={<ExternalLink size={16} strokeWidth={1.75} />}
              onClick={() => void openExternalUrl(provider.token_url)}
            >
              {t("dns_token_open_page", { provider: provider.name })}
            </Button>
            <Button
              variant="text"
              endIcon={<ExternalLink size={16} strokeWidth={1.75} />}
              onClick={() => void openExternalUrl(`${PROVIDER_GUIDE_URL}/${provider.id}.md`)}
            >
              {t("dns_token_guide")}
            </Button>
          </Stack>
          {provider.fields.map((field) => (
            <TextField
              key={`${provider.id}.${field.id}`}
              select={field.options.length > 0}
              multiline={field.multiline}
              minRows={field.multiline ? 3 : undefined}
              maxRows={field.multiline ? 8 : undefined}
              type={field.secret && !field.multiline && field.options.length === 0 ? "password" : "text"}
              label={t(`fields.${provider.id}.${field.id}`, { ns: "dns_providers", defaultValue: field.label })}
              required={!field.optional}
              helperText={field.optional ? t("optional_field") : undefined}
              value={values[field.id] ?? ""}
              onChange={(event) =>
                onFields({ ...fields, [provider.id]: { ...values, [field.id]: event.target.value } })
              }
              autoComplete={field.secret ? "new-password" : "off"}
              inputProps={{
                autoCapitalize: "none",
                spellCheck: false,
                name: `dns-${field.id}`,
                ...(field.options.length > 0 ? {} : { style: { ...mono, fontSize: 14 } }),
              }}
            >
              {field.options.map(([value, label]) => (
                <MenuItem key={value} value={value}>
                  {label}
                </MenuItem>
              ))}
            </TextField>
          ))}
          <Typography variant="caption" color="text.secondary">
            {t("dns_token_storage")}
          </Typography>
        </Section>
      ) : null}

      {provider && plan ? (
        <Section title={t("dns_records_to_write")}>
          <Box sx={{ borderRadius: "12px", border: "1px solid var(--cp-border)", overflow: "hidden" }}>
            {plan.records.map((record, index) => {
              const conflict = conflictOf(record, conflicts);
              const result = applied?.results.find(
                (entry) => entry.name === record.name && entry.type === record.type,
              );
              const pendingConflict = conflict !== null && result?.status !== "written" && result?.status !== "unchanged";
              return (
                <Box
                  key={`${record.name} ${record.type}`}
                  data-write={record.id}
                  data-status={result?.status ?? ""}
                  sx={{ px: 2, py: 1.5, borderTop: index === 0 ? "none" : "1px solid var(--cp-border)" }}
                >
                  <Stack direction="row" spacing={2} justifyContent="space-between" alignItems="baseline">
                    <Typography sx={{ ...mono, fontSize: 13, fontWeight: 500, minWidth: 0, overflowWrap: "anywhere" }}>
                      {record.name}
                    </Typography>
                    {result ? (
                      <Stack direction="row" spacing={1} alignItems="center" sx={{ flexShrink: 0 }}>
                        <StatusDot tone={resultTone[result.status]} />
                        <Typography variant="body2" color="text.secondary">
                          {t(`write_status_${result.status}`)}
                        </Typography>
                      </Stack>
                    ) : null}
                  </Stack>
                  <Typography sx={{ ...mono, fontSize: 12, color: "text.secondary", overflowWrap: "anywhere" }}>
                    {formatRecord(record)}
                  </Typography>
                  {result?.code ? (
                    <Typography variant="body2" sx={{ mt: 0.5, color: "var(--cp-danger)" }}>
                      {codeText(t, result.code)}
                    </Typography>
                  ) : null}
                  {conflict && pendingConflict ? (
                    <>
                      <Typography variant="body2" sx={{ mt: 1, color: "var(--cp-danger)", overflowWrap: "anywhere" }}>
                        {t("dns_write_conflict", { existing: conflict.existing.map(formatRecord).join("; ") })}
                      </Typography>
                      <FormControlLabel
                        control={
                          <Checkbox
                            size="small"
                            checked={confirmed.includes(conflict.name)}
                            onChange={(event) =>
                              setConfirmed((current) =>
                                event.target.checked
                                  ? [...new Set([...current, conflict.name])]
                                  : current.filter((name) => name !== conflict.name),
                              )
                            }
                          />
                        }
                        label={<Typography variant="body2">{t("dns_write_confirm")}</Typography>}
                      />
                    </>
                  ) : null}
                </Box>
              );
            })}
          </Box>
          <Typography variant="caption" color="text.secondary">
            {t("dns_write_scope")}
          </Typography>
          {error ? <ErrorAlert {...error} /> : null}
          <Button
            variant="contained"
            onClick={apply}
            disabled={busy || (!hasCredential && !formComplete)}
            startIcon={busy ? <CircularProgress size={16} thickness={5} color="inherit" /> : undefined}
            sx={{ alignSelf: "flex-start" }}
          >
            {t(applied ? "dns_write_again" : "dns_write_button")}
          </Button>
        </Section>
      ) : error ? (
        <ErrorAlert {...error} />
      ) : null}
    </Stack>
  );
};

export default DnsApiPanel;
