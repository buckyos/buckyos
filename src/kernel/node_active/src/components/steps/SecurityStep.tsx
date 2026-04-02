import {
  CheckCircleRounded,
  LockRounded,
  LoginRounded,
  PeopleRounded,
  PersonRounded,
  VerifiedRounded,
} from "@mui/icons-material";
import {
  Alert,
  Box,
  Button,
  Chip,
  CircularProgress,
  Paper,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import { buckyos } from "buckyos";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  WEB3_BASE_HOST,
  check_bucky_username,
  check_sn_active_code,
  validate_bucky_username,
} from "../../../active_lib";
import { WalletUser, WizardData } from "../../types";

type Props = {
  wizardData: WizardData;
  onUpdate: (data: Partial<WizardData>) => void;
  onNext: () => void;
  isWalletRuntime: boolean;
  walletUser?: WalletUser;
};

type NameStatus = "idle" | "checking" | "ok" | "taken" | "tooShort" | "invalid";

const SecurityStep = ({
  wizardData,
  onUpdate,
  onNext,
  isWalletRuntime,
  walletUser,
}: Props) => {
  const { t } = useTranslation();
  const [username, setUsername] = useState(
    wizardData.sn_user_name || walletUser?.sn_username || walletUser?.user_name || "",
  );
  const [snCode, setSnCode] = useState(wizardData.sn_active_code || "");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [nameStatus, setNameStatus] = useState<NameStatus>(isWalletRuntime ? "ok" : "idle");
  const [checkingSnCode, setCheckingSnCode] = useState(false);
  const [snCodeValid, setSnCodeValid] = useState<boolean | null>(isWalletRuntime ? true : null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (!isWalletRuntime) {
      return;
    }

    setUsername(wizardData.sn_user_name || walletUser?.sn_username || walletUser?.user_name || "");
    setNameStatus("ok");
    setSnCodeValid(true);
  }, [isWalletRuntime, walletUser, wizardData.sn_user_name]);

  useEffect(() => {
    if (isWalletRuntime) {
      return;
    }

    const trimmedUsername = username.trim().toLowerCase();
    if (trimmedUsername.length <= 4) {
      setNameStatus(trimmedUsername.length > 0 ? "tooShort" : "idle");
      return;
    }

    const validation = validate_bucky_username(trimmedUsername);
    if (!validation.valid) {
      setNameStatus("invalid");
      return;
    }

    let cancelled = false;
    setNameStatus("checking");
    check_bucky_username(trimmedUsername)
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
  }, [isWalletRuntime, username]);

  useEffect(() => {
    if (isWalletRuntime) {
      return;
    }

    const trimmedCode = snCode.trim();
    if (trimmedCode.length < 7) {
      setSnCodeValid(null);
      return;
    }

    let cancelled = false;
    setCheckingSnCode(true);
    check_sn_active_code(trimmedCode)
      .then((ok) => {
        if (!cancelled) {
          setSnCodeValid(ok);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setSnCodeValid(false);
        }
      })
      .finally(() => {
        if (!cancelled) {
          setCheckingSnCode(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [isWalletRuntime, snCode]);

  const renderStatusChip = () => {
    if (nameStatus === "checking") {
      return (
        <Chip
          size="small"
          label={t("username_checking")}
          icon={<CircularProgress size={14} />}
        />
      );
    }
    if (nameStatus === "ok") {
      return (
        <Chip
          size="small"
          color="success"
          label={t("username_available")}
          icon={<VerifiedRounded />}
        />
      );
    }
    if (nameStatus === "taken") {
      return <Chip size="small" color="error" label={t("error_name_taken")} />;
    }
    if (nameStatus === "tooShort") {
      return <Chip size="small" color="warning" label={t("error_name_too_short")} />;
    }
    if (nameStatus === "invalid") {
      return (
        <Chip
          size="small"
          color="error"
          label={t("error_name_invalid") || "Invalid name"}
        />
      );
    }
    return null;
  };

  const handleNext = async () => {
    setError("");

    const normalizedUsername = (
      isWalletRuntime
        ? wizardData.sn_user_name || walletUser?.sn_username || walletUser?.user_name || username
        : username
    )
      .trim()
      .toLowerCase();

    if (!normalizedUsername || normalizedUsername.length <= 4) {
      setError(t("error_name_too_short") || "");
      return;
    }

    const usernameValidation = validate_bucky_username(normalizedUsername);
    if (!usernameValidation.valid) {
      setError(
        t("error_name_invalid") || "Only lowercase letters and numbers are supported.",
      );
      return;
    }

    if (!isWalletRuntime) {
      if (nameStatus === "checking") {
        setError(t("username_checking") || "Checking availability…");
        return;
      }
      if (nameStatus !== "ok") {
        setError(t("error_name_taken") || "");
        return;
      }
    }

    const normalizedCode = snCode.trim();
    if (!isWalletRuntime) {
      if (!normalizedCode || normalizedCode.length < 8) {
        setError(t("error_invite_code_too_short") || "");
        return;
      }
      if (checkingSnCode) {
        setError(t("invite_checking") || "Checking invitation code…");
        return;
      }
      if (snCodeValid !== true) {
        setError(t("error_invite_code_invalid") || "");
        return;
      }
    }

    if (password.length < 8) {
      setError(t("error_password_too_short") || "");
      return;
    }
    if (password !== confirm) {
      setError(t("error_password_mismatch") || "");
      return;
    }

    setLoading(true);
    try {
      const hash = await buckyos.hashPassword(normalizedUsername, password);
      onUpdate({
        sn_user_name: normalizedUsername,
        owner_user_name: isWalletRuntime ? wizardData.owner_user_name : normalizedUsername,
        sn_active_code: isWalletRuntime ? wizardData.sn_active_code : normalizedCode,
        admin_password_hash: hash,
        friend_passcode: "",
        enable_guest_access: false,
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
      <Alert icon={<PersonRounded />} severity="info">
        {t("create_sn_user_intro", {
          defaultValue: "先创建 SN 用户，再继续设置当前设备的访问方式和系统权限。",
        })}
      </Alert>

      <Paper variant="outlined" sx={{ p: 2.5, borderRadius: 3 }}>
        <Stack spacing={2}>
          <Stack
            direction={{ xs: "column", sm: "row" }}
            spacing={1.5}
            justifyContent="space-between"
            alignItems={{ xs: "flex-start", sm: "center" }}
          >
            <Box>
              <Typography fontWeight={700}>
                {t("create_sn_user_title", { defaultValue: "创建 SN 用户" })}
              </Typography>
              <Typography variant="body2" color="text.secondary">
                {t("create_sn_user_desc", {
                  defaultValue: "第一步先创建一个新的 SN 用户。后续步骤不再重复输入用户名和激活码。",
                })}
              </Typography>
            </Box>
            {!isWalletRuntime ? (
              <Button
                variant="outlined"
                startIcon={<LoginRounded />}
                disabled
                sx={{ minHeight: 44 }}
              >
                {t("login_existing_sn_account", { defaultValue: "登录已有SN账号" })}
              </Button>
            ) : null}
          </Stack>

          {!isWalletRuntime ? (
            <Typography variant="caption" color="text.secondary">
              {t("login_existing_sn_account_hint", {
                defaultValue: "该入口预留在这里，当前版本暂未实现。",
              })}
            </Typography>
          ) : null}

          <TextField
            label={t("username_placeholder")}
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            helperText={
              isWalletRuntime
                ? t("wallet_bound_username", {
                    defaultValue: "当前钱包已经绑定 SN 用户名。",
                  })
                : username.trim()
                ? `https://${username.trim().toLowerCase()}.${WEB3_BASE_HOST}`
                : t("domain_format")
            }
            required
            InputProps={{
              readOnly: isWalletRuntime,
              endAdornment: renderStatusChip() ? (
                <Box sx={{ pr: 1 }}>{renderStatusChip()}</Box>
              ) : undefined,
            }}
          />

          {!isWalletRuntime ? (
            <TextField
              label={t("invite_code_placeholder")}
              value={snCode}
              onChange={(e) => setSnCode(e.target.value)}
              error={snCodeValid === false}
              helperText={
                checkingSnCode
                  ? t("invite_checking")
                  : snCodeValid === false
                  ? t("error_invite_code_invalid")
                  : snCodeValid === true
                  ? t("invite_valid")
                  : t("invite_code_required")
              }
              fullWidth
              required
            />
          ) : (
            <Alert icon={<CheckCircleRounded />} severity="success">
              {t("wallet_bound_username", {
                defaultValue: "当前钱包已经绑定 SN 用户名。",
              })}
            </Alert>
          )}
        </Stack>
      </Paper>

      <Paper variant="outlined" sx={{ p: 2.5, borderRadius: 3 }}>
        <Stack spacing={2}>
          <Alert icon={<LockRounded />} severity="info">
            {t("set_admin_password")}
          </Alert>
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
        </Stack>
      </Paper>

      {error && <Alert severity="error">{error}</Alert>}
      <Stack direction="row" justifyContent="flex-end" spacing={1.5} flexWrap="wrap" alignItems="center">
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
