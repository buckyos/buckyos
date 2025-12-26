import {
  Alert,
  Box,
  Button,
  Chip,
  CircularProgress,
  IconButton,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import { CheckCircleRounded, ContentCopyRounded, LaunchRounded } from "@mui/icons-material";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { WizardData } from "../../types";
import { do_active, do_active_by_wallet } from "../../../active_lib";

type Props = {
  wizardData: WizardData;
  onUpdate: (data: Partial<WizardData>) => void;
  onActivated: (targetUrl: string) => void;
  onBack: () => void;
  isWalletRuntime: boolean;
};

const ReviewStep = ({ wizardData, onActivated, onBack, isWalletRuntime }: Props) => {
  const { t } = useTranslation();
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");

  const targetHost = wizardData.use_self_domain
    ? wizardData.self_domain
    : wizardData.sn_user_name && wizardData.web3_base_host
    ? `${wizardData.sn_user_name}.${wizardData.web3_base_host}`
    : "";
  const targetUrl = targetHost ? `https://${targetHost}` : "";

  const copyKey = () => {
    const key = wizardData.owner_private_key as string;
    if (key) {
      navigator.clipboard?.writeText(key);
    }
  };

  const handleActivate = async () => {
    setError("");
    setLoading(true);
    try {
      const ok = isWalletRuntime ? await do_active_by_wallet(wizardData) : await do_active(wizardData);
      if (ok) {
        onActivated(targetUrl);
      } else {
        setError(t("error_activation_failed") || "Activation failed");
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setError(`${t("error_activation_failed") || "Activation failed"} ${msg}`);
    } finally {
      setLoading(false);
    }
  };

  return (
    <Stack spacing={3}>

      <Alert icon={<CheckCircleRounded />} severity="info">
        {wizardData.use_self_domain ? (
          <span>
            {t("access_domain")}: <strong>{targetHost || t("domain_placeholder")}</strong>
          </span>
        ) : (
          <span>
            {t("access_domain")}: <strong>{targetHost || t("domain_format")}</strong>
          </span>
        )}
      </Alert>
      <Stack direction="row" spacing={1} flexWrap="wrap">
        <Chip label={wizardData.gatewy_type} color="primary" />
        <Chip
          label={wizardData.use_self_domain ? t("use_own_domain") : t("use_buckyos_domain")}
          color="secondary"
        />
        <Chip
          label={
            wizardData.enable_guest_access ? t("enable_guest_mode") : (t("guest_mode_desc") || "Private access")
          }
        />
      </Stack>
      {!wizardData.is_wallet_runtime && (
        <Stack spacing={1}>
          <Typography fontWeight={700}>{t("owner_private_key")}</Typography>
          <TextField
            value={wizardData.owner_private_key as string}
            multiline
            minRows={4}
            InputProps={{
              readOnly: true,
              endAdornment: (
                <IconButton onClick={copyKey} aria-label="copy private key">
                  <ContentCopyRounded />
                </IconButton>
              ),
            }}
          />
          <Alert severity="warning">
            <Typography variant="body2">{t("private_key_warning1")}</Typography>
            <Typography variant="body2">{t("private_key_warning2")}</Typography>
            <Typography variant="body2">{t("private_key_warning3")}</Typography>
          </Alert>
        </Stack>
      )}

      {error && <Alert severity="error">{error}</Alert>}

      <Stack direction="row" justifyContent="space-between" spacing={1.5} flexWrap="wrap" alignItems="center">
        <Button variant="text" onClick={onBack}>
          {t("back_button")}
        </Button>
        <Button
          variant="contained"
          size="large"
          onClick={handleActivate}
          endIcon={!loading ? <LaunchRounded /> : undefined}
          disabled={loading}
          sx={{ py: 1.15, minWidth: 160 }}
        >
          {loading ? <CircularProgress size={20} /> : t("activate_button")}
        </Button>
      </Stack>

    </Stack>
  );
};

export default ReviewStep;
