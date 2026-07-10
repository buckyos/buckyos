import {
  AutoAwesomeRounded,
  ContentCopyRounded,
  PublicRounded,
  VerifiedRounded,
} from "@mui/icons-material";
import {
  Alert,
  Box,
  Button,
  CircularProgress,
  Grid,
  Paper,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { WEB3_BASE_HOST, bindUserDomain, isValidDomain } from "../../../active_lib";
import { WizardData } from "../../types";
import { copyTextToClipboard } from "../../utils/clipboard";

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

  return (
    <Stack spacing={3}>
      <Grid container spacing={2}>
        <Grid item xs={12} md={6}>
          <Paper
            onClick={() => chooseMode("bucky")}
            sx={{
              p: 2,
              borderRadius: 3,
              cursor: "pointer",
              border: `1px solid ${mode === "bucky" ? "transparent" : "divider"}`,
              bgcolor: mode === "bucky" ? "action.selected" : "background.paper",
            }}
          >
            <Stack direction="row" spacing={1.5} alignItems="center">
              <AutoAwesomeRounded color={mode === "bucky" ? "primary" : "disabled"} />
              <Box>
                <Typography fontWeight={700}>{t("use_buckyos_domain")}</Typography>
                <Typography variant="body2" color="text.secondary">
                  {previewDomain}
                </Typography>
              </Box>
            </Stack>
          </Paper>
        </Grid>
        <Grid item xs={12} md={6}>
          <Paper
            onClick={() => chooseMode("self")}
            sx={{
              p: 2,
              borderRadius: 3,
              cursor: "pointer",
              border: `1px solid ${mode === "self" ? "transparent" : "divider"}`,
              bgcolor: mode === "self" ? "action.selected" : "background.paper",
            }}
          >
            <Stack direction="row" spacing={1.5} alignItems="center">
              <PublicRounded color={mode === "self" ? "success" : "disabled"} />
              <Box>
                <Typography fontWeight={700}>{t("use_own_domain")}</Typography>
                <Typography variant="body2" color="text.secondary">
                  {t("domain_provider_setup")}
                </Typography>
              </Box>
            </Stack>
          </Paper>
        </Grid>
      </Grid>

      {mode === "bucky" ? (
        <Alert severity="info">{previewDomain}</Alert>
      ) : (
        <Stack spacing={2}>
          <TextField
            label={t("domain_placeholder")}
            value={domain}
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
          {binding.state === "challenge" && binding.domain === domain.trim().toLowerCase() && (
            <Alert severity="warning">
              <Stack spacing={1.5}>
                <Typography fontWeight={700}>
                  {t("domain_txt_challenge", "Add this TXT record, then verify again")}
                </Typography>
                <TextField
                  label={t("dns_record_name", "Record name")}
                  value={binding.record_name}
                  InputProps={{ readOnly: true }}
                />
                <TextField
                  multiline
                  label={t("dns_record_value", "TXT value")}
                  value={binding.value}
                  InputProps={{ readOnly: true }}
                />
                <Button
                  startIcon={<ContentCopyRounded />}
                  onClick={() => copyTextToClipboard(binding.value)}
                >
                  {t("copy_button", "Copy value")}
                </Button>
                <Typography variant="body2">{binding.reason}</Typography>
                <Typography variant="caption">
                  {t("dns_propagation_hint", "DNS propagation may take several minutes.")}
                </Typography>
              </Stack>
            </Alert>
          )}
          {binding.state === "verified" && (
            <Alert severity="success" icon={<VerifiedRounded />}>
              {t("domain_verified", "Domain ownership verified")}
            </Alert>
          )}
        </Stack>
      )}

      {error && <Alert severity="error">{error}</Alert>}
      <Stack direction="row" justifyContent="space-between" spacing={2}>
        <Button onClick={onBack}>{t("back_button")}</Button>
        {mode === "bucky" ? (
          <Button variant="contained" size="large" onClick={continueDefault}>
            {t("next_button")}
          </Button>
        ) : (
          <Button
            variant="contained"
            size="large"
            onClick={verifyDomain}
            disabled={loading}
            startIcon={loading ? <CircularProgress size={18} /> : <VerifiedRounded />}
          >
            {binding.state === "challenge"
              ? t("verify_again", "Verify again")
              : t("verify_domain", "Verify domain")}
          </Button>
        )}
      </Stack>
    </Stack>
  );
};

export default DomainStep;
