import {
  Alert,
  Box,
  Button,
  Chip,
  CircularProgress,
  Paper,
  Grid,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import {
  AutoAwesomeRounded,
  DnsRounded,
  PublicRounded,
  VerifiedRounded,
} from "@mui/icons-material";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { GatewayType, WalletUser, WizardData } from "../../types";
import {
  check_bucky_username,
  generate_key_pair,
  isValidDomain,
} from "../../../active_lib";

type Props = {
  wizardData: WizardData;
  onUpdate: (data: Partial<WizardData>) => void;
  onNext: () => void;
  onBack: () => void;
  isWalletRuntime: boolean;
  walletUser?: WalletUser;
};

type NameStatus = "idle" | "checking" | "ok" | "taken" | "tooShort";

const DomainStep = ({ wizardData, onUpdate, onNext, onBack, isWalletRuntime, walletUser }: Props) => {
  const { t } = useTranslation();
  const [mode, setMode] = useState<"bucky" | "self">(wizardData.use_self_domain ? "self" : "bucky");
  const [username, setUsername] = useState(wizardData.sn_user_name || walletUser?.user_name || "");
  const [domain, setDomain] = useState(wizardData.self_domain || "");
  const [snCode, setSnCode] = useState(wizardData.sn_active_code || "");
  const [nameStatus, setNameStatus] = useState<NameStatus>("idle");
  const [generatingKeys, setGeneratingKeys] = useState(false);
  const [formError, setFormError] = useState("");

  useEffect(() => {
    if (isWalletRuntime && walletUser?.user_name) {
      setUsername(walletUser.user_name);
    }
  }, [isWalletRuntime, walletUser]);

  useEffect(() => {
    if (isWalletRuntime) {
      return;
    }
    if (wizardData.owner_private_key) {
      return;
    }
    setGeneratingKeys(true);
    generate_key_pair()
      .then(([pub, priv]) => {
        onUpdate({ owner_public_key: pub, owner_private_key: priv });
      })
      .finally(() => setGeneratingKeys(false));
  }, [onUpdate, wizardData.owner_private_key]);

  useEffect(() => {
    if (isWalletRuntime) {
      setNameStatus("ok");
      return;
    }
    if (mode !== "bucky" || username.trim().length <= 4) {
      setNameStatus(username.trim().length > 0 && username.trim().length <= 4 ? "tooShort" : "idle");
      return;
    }
    let cancelled = false;
    setNameStatus("checking");
    check_bucky_username(username.trim())
      .then((available) => {
        if (!cancelled) {
          setNameStatus(available ? "ok" : "taken");
        }
      })
      .catch(() => {
        if (!cancelled) {
          setNameStatus("idle");
        }
      });
    return () => {
      cancelled = true;
    };
  }, [mode, username]);

  useEffect(() => {
    if (mode !== "bucky" || !wizardData.owner_private_key || username.trim().length <= 4) {
      return;
    }
    if (wizardData.zone_config_jwt) {
      return;
    }
    let cancelled = false;
    const snHost = (() => {
      try {
        return new URL(wizardData.sn_url || "").hostname;
      } catch {
        return "sn.buckyos.ai";
      }
    })();
   
    return () => {
      cancelled = true;
    };
  }, [mode, username, wizardData.owner_private_key, wizardData.sn_url, wizardData.zone_config_jwt]);

  const handleNext = async () => {
    setFormError("");
    if (mode === "bucky") {
      if (username.trim().length <= 4) {
        setFormError(t("error_name_too_short") || "");
        return;
      }
      if (!isWalletRuntime && nameStatus === "taken") {
        setFormError(t("error_name_taken") || "");
        return;
      }
      if (!snCode || snCode.length < 8) {
        setFormError(t("error_invite_code_too_short") || "");
        return;
      }
      onUpdate({
        use_self_domain: false,
        sn_user_name: username.trim(),
        self_domain: "",
        sn_active_code: snCode,
      });
      onNext();
      return;
    }

    if (!domain.trim() || !isValidDomain(domain.trim())) {
      setFormError(t("error_domain_format") || "");
      return;
    }
    if (!wizardData.owner_private_key) {
      setFormError(t("error_private_key_not_ready") || "");
      return;
    }
    onUpdate({
      use_self_domain: true,
      self_domain: domain.trim(),
      sn_user_name: username.trim() || wizardData.sn_user_name,
      sn_active_code: snCode,
    });
    onNext();
  };

  const previewDomain = wizardData.web3_base_host
    ? `https://${username || "your-name"}.${wizardData.web3_base_host}`
    : "";

  const renderStatusChip = () => {
    if (nameStatus === "checking") {
      return <Chip size="small" label={t("username_checking")} icon={<CircularProgress size={14} />} />;
    }
    if (nameStatus === "ok") {
      return <Chip size="small" color="success" label={t("username_available")} icon={<VerifiedRounded />} />;
    }
    if (nameStatus === "taken") {
      return <Chip size="small" color="error" label={t("error_name_taken")} />;
    }
    if (nameStatus === "tooShort") {
      return <Chip size="small" color="warning" label={t("error_name_too_short")} />;
    }
    return null;
  };

  return (
    <Stack spacing={3}>

      <Grid container spacing={2}>
        <Grid item xs={12} md={6}>
          <Paper
            onClick={() => setMode("bucky")}
            sx={{
              p: 2,
              borderRadius: 3,
              cursor: "pointer",
              border: `1px solid ${mode === "bucky" ? "transparent" : "divider"}`,
              background: mode === "bucky" ? "linear-gradient(120deg, rgba(79,70,229,0.12), rgba(14,165,233,0.12))" : undefined,
              backgroundColor: mode === "bucky" ? undefined : "background.paper",
              boxShadow: mode === "bucky" ? "0 10px 40px rgba(79,70,229,0.18)" : "none",
              transition: "all 0.25s ease",
            }}
          >
            <Stack direction="row" spacing={1.5} alignItems="center">
              <Box
                sx={{
                  width: 44,
                  height: 44,
                  borderRadius: "12px",
                  display: "grid",
                  placeItems: "center",
                  backgroundColor: mode === "bucky" ? "primary.main" : "action.hover",
                  color: mode === "bucky" ? "primary.contrastText" : "text.secondary",
                }}
              >
                <AutoAwesomeRounded />
              </Box>
              <Box>
                <Typography fontWeight={700}>{t("use_buckyos_domain")}</Typography>
                <Typography variant="body2" color="text.secondary">
                  {t("domain_access_desc")}
                </Typography>
              </Box>
              {mode === "bucky" && renderStatusChip()}
            </Stack>
          </Paper>
        </Grid>
        <Grid item xs={12} md={6}>
          <Paper
            onClick={() => setMode("self")}
            sx={{
              p: 2,
              borderRadius: 3,
              cursor: "pointer",
              border: `1px solid ${mode === "self" ? "transparent" : "divider"}`,
              background: mode === "self" ? "linear-gradient(120deg, rgba(16,185,129,0.14), rgba(14,165,233,0.12))" : undefined,
              backgroundColor: mode === "self" ? undefined : "background.paper",
              boxShadow: mode === "self" ? "0 10px 40px rgba(16,185,129,0.2)" : "none",
              transition: "all 0.25s ease",
            }}
          >
            <Stack direction="row" spacing={1.5} alignItems="center">
              <Box
                sx={{
                  width: 44,
                  height: 44,
                  borderRadius: "12px",
                  display: "grid",
                  placeItems: "center",
                  backgroundColor: mode === "self" ? "success.main" : "action.hover",
                  color: mode === "self" ? "success.contrastText" : "text.secondary",
                }}
              >
                <PublicRounded />
              </Box>
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
        <Stack spacing={2}>
          <TextField
            label={t("username_placeholder")}
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            helperText={isWalletRuntime ? t("wallet_bound_username", { defaultValue: "Wallet 已绑定用户名" }) || previewDomain : previewDomain}
            required
            InputProps={{
              readOnly: isWalletRuntime,
              endAdornment: renderStatusChip() ? <Box sx={{ pr: 1 }}>{renderStatusChip()}</Box> : undefined,
            }}
          />
          <TextField
            label={t("invite_code_placeholder")}
            value={snCode}
            onChange={(e) => setSnCode(e.target.value)}
            helperText={t("invite_code_required")}
            required
          />
          {generatingKeys && (
            <Alert icon={<CircularProgress size={16} />} severity="info">
              {t("generate_keys_progress")}
            </Alert>
          )}
        </Stack>
      ) : (
        <Stack spacing={2}>
          <TextField
            label={t("domain_placeholder")}
            value={domain}
            onChange={(e) => setDomain(e.target.value)}
            placeholder="example.com"
            required
            InputProps={{
              endAdornment: wizardData.zone_config_jwt ? (
                <Chip size="small" color="success" label={t("zone_config_ready")} />
              ) : null,
            }}
          />
          <Alert icon={<DnsRounded fontSize="small" />} severity="info">
            {t("dns_ns_record", { sn_host_base: wizardData.web3_base_host || "web3.buckyos.ai" })}
          </Alert>
        </Stack>
      )}

      {formError && <Alert severity="error">{formError}</Alert>}
      <Stack direction="row" justifyContent="space-between" spacing={1.5} flexWrap="wrap" alignItems="center">
        <Button variant="text" onClick={onBack}>
          {t("back_button")}
        </Button>
        <Button
          variant="contained"
          onClick={handleNext}
          size="large"
          sx={{ py: 1.15, minWidth: 160 }}
        >
          {t("next_button")}
        </Button>
      </Stack>
    </Stack>
  );
};

export default DomainStep;
