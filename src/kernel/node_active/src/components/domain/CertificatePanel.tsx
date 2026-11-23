import { Upload, X } from "lucide-react";
import {
  Alert,
  Box,
  Button,
  Checkbox,
  CircularProgress,
  FormControlLabel,
  IconButton,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { inspectCertificate } from "../../../active_lib";
import { codeText, errorText, type ErrorText } from "../../errors";
import type { CertificateCheckItem, CertificateInspection, DomainState } from "../../types";
import { ErrorAlert, Section, StatusDot, StatusLine, mono, type Tone } from "../ui";

const MAX_UPLOAD_BYTES = 1024 * 1024;

type PickedFile = { name: string; size: number; base64: string };

const levelTone: Record<CertificateCheckItem["level"], Tone> = {
  pass: "success",
  warn: "warning",
  reject: "danger",
};

function toBase64(bytes: Uint8Array): string {
  let binary = "";
  for (let offset = 0; offset < bytes.length; offset += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + 0x8000));
  }
  return btoa(binary);
}

/** The two coverage checks are shown as the name they are about. */
function coverageOf(item: CertificateCheckItem, inspection: CertificateInspection) {
  if (item.code !== "covers_domain" && item.code !== "covers_wildcard") return null;
  const wildcard = item.code === "covers_wildcard";
  return inspection.coverage.find((entry) => entry.name.startsWith("*.") === wildcard) ?? null;
}

/** Who can still reach the Zone with a certificate that is not publicly trusted. */
export const UntrustedImpact = () => {
  const { t } = useTranslation();
  return (
    <Box component="ul" sx={{ m: 0, pl: 2.5 }}>
      {["cert_untrusted_browser", "cert_untrusted_zone", "cert_untrusted_app", "cert_untrusted_check"].map((key) => (
        <li key={key}>{t(key)}</li>
      ))}
    </Box>
  );
};

type Props = {
  domain: string;
  state: DomainState;
  onState: (partial: Partial<DomainState>) => void;
};

const CertificatePanel = ({ domain, state, onState }: Props) => {
  const { t, i18n } = useTranslation();
  const picker = useRef<HTMLInputElement>(null);
  const [files, setFiles] = useState<PickedFile[]>([]);
  const [pasted, setPasted] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<ErrorText | null>(null);

  const inspection = state.certificate_inspection;
  const accepted = state.certificate_id !== null;
  const wantsPassword =
    files.some((file) => /\.(pfx|p12)$/i.test(file.name)) ||
    inspection?.items.some((item) => item.reason === "pfx_password_wrong") === true;

  const pick = async (list: FileList | null) => {
    setError(null);
    const picked = await Promise.all(
      [...(list ?? [])].map(async (file) => ({
        name: file.name,
        size: file.size,
        base64: file.size > MAX_UPLOAD_BYTES ? "" : toBase64(new Uint8Array(await file.arrayBuffer())),
      })),
    );
    setFiles((current) => [
      ...current.filter((file) => !picked.some((next) => next.name === file.name)),
      ...picked,
    ]);
    if (picker.current) picker.current.value = "";
  };

  const check = async () => {
    const text = pasted.trim();
    const textBytes = new TextEncoder().encode(text);
    const total = files.reduce((sum, file) => sum + file.size, textBytes.length);
    if (total > MAX_UPLOAD_BYTES) return setError({ message: codeText(t, "certificate_too_large") });
    setError(null);
    setBusy(true);
    try {
      const result = await inspectCertificate({
        domain,
        files: [...files.map((file) => file.base64), ...(text ? [toBase64(textBytes)] : [])],
        ...(password ? { pfx_password: password } : {}),
      });
      onState({
        certificate_id: result.certificate_id,
        certificate_inspection: result.inspection,
        accept_warnings: false,
      });
      if (result.certificate_id) {
        setFiles([]);
        setPasted("");
        setPassword("");
      }
    } catch (cause) {
      setError(errorText(t, cause));
    } finally {
      setBusy(false);
    }
  };

  const formatTime = (seconds: number) =>
    new Date(seconds * 1000).toLocaleString(i18n.language, { dateStyle: "long", timeStyle: "short" });
  const formatDate = (seconds: number) =>
    new Date(seconds * 1000).toLocaleDateString(i18n.language, { dateStyle: "long" });

  const reasonText = (item: CertificateCheckItem, result: CertificateInspection) =>
    t([`cert_reason_${item.reason}`, "cert_reason_other"], {
      domain,
      wildcard: `*.${domain}`,
      detail: item.detail ?? "",
      date: result.not_after ? formatDate(result.not_after) : "",
      time: formatTime(result.node_time),
    });

  const results = inspection ? (
    <Stack spacing={2}>
      <Box sx={{ borderRadius: "12px", border: "1px solid var(--cp-border)", overflow: "hidden" }}>
        {inspection.items.map((item, index) => {
          const coverage = coverageOf(item, inspection);
          return (
            <Stack
              key={item.code}
              data-cert-check={item.code}
              data-level={item.level}
              direction="row"
              spacing={1.5}
              sx={{ px: 2, py: 1.25, borderTop: index === 0 ? "none" : "1px solid var(--cp-border)", minWidth: 0 }}
            >
              <Box sx={{ display: "flex", alignItems: "center", height: 20 }}>
                <StatusDot tone={levelTone[item.level]} />
              </Box>
              <Box sx={{ flex: 1, minWidth: 0 }}>
                <Stack direction="row" spacing={2} justifyContent="space-between">
                  <Typography
                    variant="body2"
                    sx={{ ...(coverage ? { ...mono, fontSize: 13 } : {}), minWidth: 0, overflowWrap: "anywhere" }}
                  >
                    {coverage ? coverage.name : t(`cert_check_${item.code}`)}
                  </Typography>
                  {coverage ? (
                    <Typography variant="body2" color="text.secondary" sx={{ maxWidth: "55%", textAlign: "right", overflowWrap: "anywhere" }}>
                      {coverage.covered_by ? t("cert_covered_by", { name: coverage.covered_by }) : t("cert_not_covered")}
                    </Typography>
                  ) : null}
                </Stack>
                {item.reason ? (
                  <Typography variant="body2" color="text.secondary" sx={{ overflowWrap: "anywhere" }}>
                    {reasonText(item, inspection)}
                  </Typography>
                ) : null}
              </Box>
            </Stack>
          );
        })}
      </Box>
      {accepted && inspection.not_after ? (
        <Typography variant="body2" color="text.secondary">
          {t("cert_expires", { date: formatDate(inspection.not_after) })}
          {inspection.issuer ? ` ${t("cert_issuer", { issuer: inspection.issuer })}` : ""}
        </Typography>
      ) : null}
      {accepted && inspection.items.some((item) => item.level === "warn") ? (
        <Alert severity="warning">
          {inspection.publicly_trusted ? null : (
            <Box sx={{ mb: 1 }}>
              {t("cert_untrusted_title")}
              <UntrustedImpact />
            </Box>
          )}
          <FormControlLabel
            control={
              <Checkbox
                size="small"
                checked={state.accept_warnings}
                onChange={(event) => onState({ accept_warnings: event.target.checked })}
              />
            }
            label={<Typography variant="body2">{t("cert_accept_warnings")}</Typography>}
          />
        </Alert>
      ) : null}
    </Stack>
  ) : null;

  if (accepted) {
    return (
      <Section
        title={t("cert_title")}
        action={
          <Button
            variant="text"
            size="small"
            onClick={() => onState({ certificate_id: null, certificate_inspection: null, accept_warnings: false })}
          >
            {t("cert_replace")}
          </Button>
        }
      >
        <StatusLine tone="success">{t("cert_ready")}</StatusLine>
        {results}
      </Section>
    );
  }

  return (
    <Section title={t("cert_title")}>
      <Typography variant="body2" color="text.secondary">
        {t("cert_intro", { domain, wildcard: `*.${domain}` })}
      </Typography>
      <input ref={picker} type="file" multiple hidden onChange={(event) => void pick(event.target.files)} />
      <Button
        variant="outlined"
        startIcon={<Upload size={16} strokeWidth={1.75} />}
        onClick={() => picker.current?.click()}
        sx={{ alignSelf: "flex-start" }}
      >
        {t("cert_choose_files")}
      </Button>
      {files.length > 0 ? (
        <Stack spacing={1}>
          {files.map((file) => (
            <Stack
              key={file.name}
              direction="row"
              alignItems="center"
              spacing={1}
              sx={{ pl: 1.5, pr: 0.5, minHeight: 40, borderRadius: "10px", border: "1px solid var(--cp-border)" }}
            >
              <Typography sx={{ ...mono, fontSize: 13, flex: 1, minWidth: 0, overflowWrap: "anywhere" }}>{file.name}</Typography>
              <IconButton
                size="small"
                aria-label={t("cert_remove_file", { name: file.name })}
                onClick={() => setFiles((current) => current.filter((entry) => entry.name !== file.name))}
              >
                <X size={16} strokeWidth={1.75} />
              </IconButton>
            </Stack>
          ))}
        </Stack>
      ) : null}
      <TextField
        multiline
        minRows={3}
        maxRows={8}
        label={t("cert_paste_label")}
        placeholder="-----BEGIN CERTIFICATE-----"
        value={pasted}
        onChange={(event) => setPasted(event.target.value)}
        autoComplete="off"
        inputProps={{ spellCheck: false, autoCapitalize: "none", style: { ...mono, fontSize: 12 } }}
      />
      {wantsPassword ? (
        <TextField
          type="password"
          label={t("cert_pfx_password")}
          value={password}
          onChange={(event) => setPassword(event.target.value)}
          autoComplete="new-password"
        />
      ) : null}
      {error ? <ErrorAlert {...error} /> : null}
      <Button
        variant="contained"
        onClick={check}
        disabled={busy || (files.length === 0 && !pasted.trim())}
        startIcon={busy ? <CircularProgress size={16} thickness={5} color="inherit" /> : undefined}
        sx={{ alignSelf: "flex-start" }}
      >
        {t("cert_check_button")}
      </Button>
      {results}
      <Typography variant="caption" color="text.secondary">
        {t("cert_not_managed")}
      </Typography>
    </Section>
  );
};

export default CertificatePanel;
