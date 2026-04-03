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
  bind_owner_key,
  check_bucky_username,
  check_sn_active_code,
  generate_zone_txt_records,
  login_by_password_and_activecode,
  register_sn_user,
  SN_HOST,
  WEB3_BASE_HOST,
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
      if (nameStatus === "invalid" || nameStatus === "tooShort") {
        setError(t("error_name_invalid") || "Invalid name");
        return;
      }
    }

    const normalizedCode = snCode.trim();
    if (!isWalletRuntime && (!normalizedCode || normalizedCode.length < 8)) {
      setError(t("error_invite_code_too_short") || "");
      return;
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
      if (!isWalletRuntime) {
        const activeCodeReady = await check_sn_active_code(normalizedCode);
        if (!activeCodeReady) {
          setError(t("error_invite_code_invalid") || "");
          return;
        }

        const isUsernameAvailable = await check_bucky_username(normalizedUsername);
        if (isUsernameAvailable) {
          if (!wizardData.owner_private_key) {
            setError(t("error_private_key_not_ready") || "Private key missing");
            return;
          }

          const tempZoneRecords = await generate_zone_txt_records(
            SN_HOST,
            wizardData.owner_public_key,
            wizardData.owner_private_key,
            wizardData.device_public_key,
            null,
            wizardData.rtcp_port,
            false,
          );
          const tempZoneConfigJwt =
            tempZoneRecords && typeof tempZoneRecords["BOOT"] === "string"
              ? tempZoneRecords["BOOT"]
              : null;
          if (!tempZoneConfigJwt) {
            setError(
              t("error_generate_txt_records_failed") || "Failed to prepare SN registration config",
            );
            return;
          }

          const registerOk = await register_sn_user(
            normalizedUsername,
            normalizedCode,
            JSON.stringify(wizardData.owner_public_key),
            tempZoneConfigJwt,
            null,
          );
          if (!registerOk) {
            setError(
              t("error_activation_failed") || "Failed to register SN user",
            );
            return;
          }
        } else {
          const loginResult = await login_by_password_and_activecode(
            normalizedUsername,
            hash,
            normalizedCode,
          );
          const bindOwnerResult = await bind_owner_key(
            loginResult.access_token,
            wizardData.owner_public_key,
          );
          if (bindOwnerResult.code !== 0) {
            setError(
              t("error_activation_failed") || "Failed to bind owner key",
            );
            return;
          }
        }
      }

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
                {t("create_or_login_sn_user_title", { defaultValue: "创建或登录 SN 用户" })}
              </Typography>
              <Typography variant="body2" color="text.secondary">
                {t("create_or_login_sn_user_desc", {
                  defaultValue: "第一步会自动判断用户名是否已存在：不存在就注册，已存在就直接登录并绑定 owner key。",
                })}
              </Typography>
            </Box>
            {!isWalletRuntime ? (
              <Chip
                color="info"
                icon={<LoginRounded />}
                label={t("auto_login_when_username_exists", {
                  defaultValue: "用户名已存在时自动登录",
                })}
              />
            ) : null}
          </Stack>

          {!isWalletRuntime ? (
            <Typography variant="caption" color="text.secondary">
              {t("login_existing_sn_account_hint", {
                defaultValue: "点 Next 时会直接尝试注册或登录，失败则不能进入下一步。",
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
