import { AtSign, Copy, Globe } from "lucide-react";
import {
  Alert,
  Box,
  Button,
  CircularProgress,
  IconButton,
  Stack,
  TextField,
  Tooltip,
  Typography,
} from "@mui/material";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { WEB3_BASE_HOST, bindUserDomain, isValidDomain } from "../../../active_lib";
import { WizardData } from "../../types";
import { copyTextToClipboard } from "../../utils/clipboard";
import { ChoiceCard, StatusLine, StepActions, mono, plainLabel } from "../ui";

type Props = {
  wizardData: WizardData;
  onUpdate: (data: Partial<WizardData>) => void;
  onNext: () => void;
  onBack: () => void;
};

const DomainStep = ({ wizardData, onUpdate, onNext, onBack }: Props) => {
  const { t } = useTranslation();
  const [mode, setMode] = useState<"bucky" | "self">(
    wizardData.use_self_domain ? "self" : "bucky",
  );
  const [domain, setDomain] = useState(wizardData.self_domain || "");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const username = wizardData.sn_user_name?.trim().toLowerCase() || "";
  const previewDomain = useMemo(
    () => (username ? `https://${username}.${WEB3_BASE_HOST}` : t("domain_format")),
    [t, username],
  );
  const binding = wizardData.domain_binding;

  const chooseMode = (next: "bucky" | "self") => {
    setMode(next);
    setError("");
    onUpdate({
      use_self_domain: next === "self",
      self_domain: next === "self" ? domain.trim().toLowerCase() : "",
      domain_binding: { state: "unused" },
      prepared_documents: null,
      signed_documents: null,
    });
  };

  const continueDefault = () => {
    if (!username) return setError(t("username_placeholder"));
    onUpdate({
      use_self_domain: false,
      self_domain: "",
      domain_binding: { state: "unused" },
    });
    onNext();
  };

  const verifyDomain = async () => {
    const normalized = domain.trim().toLowerCase();
    setError("");
    if (!isValidDomain(normalized)) {
      setError(t("error_domain_format"));
      return;
    }
    setLoading(true);
    onUpdate({
      use_self_domain: true,
      self_domain: normalized,
      domain_binding: { state: "checking", domain: normalized },
      prepared_documents: null,
      signed_documents: null,
    });
    try {
      const result = await bindUserDomain(
        { ...wizardData, use_self_domain: true, self_domain: normalized },
        normalized,
      );
      onUpdate({
        use_self_domain: true,
        self_domain: normalized,
        domain_binding: result.binding,
        sn_access_token: result.accessToken,
        sn_refresh_token: result.refreshToken,
      });
      if (result.binding.state === "verified") onNext();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
      onUpdate({ domain_binding: { state: "unused" } });
    } finally {
      setLoading(false);
    }
  };

  const iconProps = { size: 20, strokeWidth: 1.75 };
  const challengeShown = binding.state === "challenge" && binding.domain === domain.trim().toLowerCase();

  return (
    <Stack spacing={3}>
      <Box
        role="radiogroup"
        aria-label={t("title_step_3").replace(/^\d+\.\s*/, "")}
        sx={{ display: "grid", gridTemplateColumns: { xs: "1fr", sm: "repeat(2, minmax(0, 1fr))" }, gap: 1.5 }}
      >
        <ChoiceCard
          selected={mode === "bucky"}
          onSelect={() => chooseMode("bucky")}
          icon={<AtSign {...iconProps} />}
          title={t("use_buckyos_domain")}
          description={<Box component="span" sx={{ ...mono, fontSize: 13, overflowWrap: "anywhere" }}>{previewDomain}</Box>}
        />
        <ChoiceCard
          selected={mode === "self"}
          onSelect={() => chooseMode("self")}
          icon={<Globe {...iconProps} />}
          title={t("use_own_domain")}
          description={plainLabel(t("domain_provider_setup"))}
        />
      </Box>

      {mode === "bucky" ? null : (
        <Stack spacing={2}>
          <TextField
            label={t("domain_placeholder")}
            value={domain}
            autoComplete="off"
            inputProps={{ autoCapitalize: "none", spellCheck: false }}
            onChange={(event) => {
              const value = event.target.value;
              setDomain(value);
              onUpdate({
                self_domain: value.trim().toLowerCase(),
                domain_binding: { state: "unused" },
                prepared_documents: null,
                signed_documents: null,
              });
            }}
            placeholder="home.example.com"
          />
          {challengeShown && binding.state === "challenge" && (
            <Box sx={{ p: 2, borderRadius: "12px", border: "1px solid var(--cp-border)", bgcolor: "var(--cp-surface-2)" }}>
              <Stack spacing={2}>
                <Stack direction="row" spacing={1} alignItems="center">
                  <Box component="span" sx={{ width: 6, height: 6, borderRadius: "50%", bgcolor: "var(--cp-warning)", flexShrink: 0 }} />
                  <Typography variant="subtitle2">
                    {t("domain_txt_challenge", "Add this TXT record, then verify again")}
                  </Typography>
                </Stack>
                {[
                  { label: t("dns_record_name", "Record name"), value: binding.record_name },
                  { label: t("dns_record_value", "TXT value"), value: binding.value },
                ].map((row) => (
                  <Box key={row.label}>
                    <Typography variant="caption" color="text.secondary" component="div" sx={{ mb: 0.5 }}>
                      {row.label}
                    </Typography>
                    <Stack
                      direction="row"
                      alignItems="center"
                      spacing={1}
                      sx={{ pl: 1.5, pr: 0.5, minHeight: 40, borderRadius: "10px", border: "1px solid var(--cp-border)", bgcolor: "var(--cp-surface)" }}
                    >
                      <Typography sx={{ ...mono, fontSize: 13, flex: 1, minWidth: 0, py: 1, overflowWrap: "anywhere", userSelect: "all" }}>
                        {row.value}
                      </Typography>
                      <Tooltip title={t("copy_button", "Copy value").replace(/^\((.*)\)$/, "$1")}>
                        <IconButton
                          size="small"
                          aria-label={t("copy_button", "Copy value").replace(/^\((.*)\)$/, "$1")}
                          onClick={() => copyTextToClipboard(row.value)}
                        >
                          <Copy size={16} strokeWidth={1.75} />
                        </IconButton>
                      </Tooltip>
                    </Stack>
                  </Box>
                ))}
                {binding.reason ? (
                  <Typography variant="body2" color="text.secondary">
                    {binding.reason}
                  </Typography>
                ) : null}
                <Typography variant="caption" color="text.secondary">
                  {t("dns_propagation_hint", "DNS propagation may take several minutes.")}
                </Typography>
              </Stack>
            </Box>
          )}
          {binding.state === "verified" && (
            <StatusLine tone="success">{t("domain_verified", "Domain ownership verified")}</StatusLine>
          )}
        </Stack>
      )}

      {error && <Alert severity="error">{error}</Alert>}
      <StepActions back={<Button variant="text" onClick={onBack}>{t("back_button")}</Button>}>
        {mode === "bucky" ? (
          <Button variant="contained" size="large" onClick={continueDefault} sx={{ minWidth: 120 }}>
            {t("next_button")}
          </Button>
        ) : (
          <Button
            variant="contained"
            size="large"
            onClick={verifyDomain}
            disabled={loading}
            startIcon={loading ? <CircularProgress size={16} thickness={5} color="inherit" /> : undefined}
            sx={{ minWidth: 120 }}
          >
            {binding.state === "challenge"
              ? t("verify_again", "Verify again")
              : t("verify_domain", "Verify domain")}
          </Button>
        )}
      </StepActions>
    </Stack>
  );
};

export default DomainStep;
