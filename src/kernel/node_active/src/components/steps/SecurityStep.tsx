import { LockRounded, PeopleRounded } from "@mui/icons-material";
import {
  Alert,
  Box,
  Button,
  CircularProgress,
  FormControlLabel,
  Stack,
  Switch,
  TextField,
  Typography,
} from "@mui/material";
import { buckyos } from "buckyos";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { WizardData } from "../../types";

type Props = {
  wizardData: WizardData;
  onUpdate: (data: Partial<WizardData>) => void;
  onNext: () => void;
  onBack: () => void;
};

const SecurityStep = ({ wizardData, onUpdate, onNext, onBack }: Props) => {
  const { t } = useTranslation();
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [friendCode, setFriendCode] = useState(wizardData.friend_passcode || "");
  const [guestAccess, setGuestAccess] = useState(wizardData.enable_guest_access);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);

  const handleNext = async () => {
    setError("");
    if (password.length < 8) {
      setError(t("error_password_too_short") || "");
      return;
    }
    if (password !== confirm) {
      setError(t("error_password_mismatch") || "");
      return;
    }
    if (friendCode && friendCode.length < 6) {
      setError(t("error_friend_code_too_short") || "");
      return;
    }
    setLoading(true);
    try {
      const hash = await buckyos.hashPassword(wizardData.sn_user_name || "", password);
      onUpdate({
        admin_password_hash: hash,
        friend_passcode: friendCode,
        enable_guest_access: guestAccess,
      });
      onNext();
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setError(`${t("error_activation_failed") || "Failed"} ${msg}`);
    } finally {
      setLoading(false);
    }
  };

  return (
    <Stack spacing={3}>

      <Alert icon={<LockRounded />} severity="info">
        {t("set_admin_password")}
      </Alert>
      <Stack spacing={2}>
        <TextField
          type="password"
          label={t("admin_password_placeholder")}
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          required
        />
        <TextField
          type="password"
          label={t("confirm_password_placeholder")}
          value={confirm}
          onChange={(e) => setConfirm(e.target.value)}
          required
        />
        <TextField
          label={t("friend_code_placeholder")}
          value={friendCode}
          onChange={(e) => setFriendCode(e.target.value)}
          helperText={t("friend_code_desc")}
        />
        <FormControlLabel
          control={<Switch checked={guestAccess} onChange={(e) => setGuestAccess(e.target.checked)} />}
          label={
            <Box>
              <Typography fontWeight={600}>{t("enable_guest_mode")}</Typography>
              <Typography variant="body2" color="text.secondary">
                {t("guest_mode_desc")}
              </Typography>
            </Box>
          }
        />
      </Stack>

      {error && <Alert severity="error">{error}</Alert>}
      <Stack direction="row" justifyContent="space-between" spacing={1.5} flexWrap="wrap" alignItems="center">
        <Button variant="text" onClick={onBack}>
          {t("back_button")}
        </Button>
        <Button
          variant="contained"
          onClick={handleNext}
          disabled={loading}
          startIcon={!loading ? <PeopleRounded /> : undefined}
          size="large"
          sx={{ py: 1.15, minWidth: 160 }}
        >
          {loading ? <CircularProgress size={18} /> : t("next_button")}
        </Button>
      </Stack>
    </Stack>
  );
};

export default SecurityStep;
