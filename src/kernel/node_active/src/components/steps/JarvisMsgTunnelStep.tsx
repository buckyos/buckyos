import {
  Alert,
  Box,
  Button,
  IconButton,
  InputAdornment,
  Link,
  Paper,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import {
  LaunchRounded,
  SendRounded,
  VisibilityOffRounded,
  VisibilityRounded,
} from "@mui/icons-material";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { WizardData } from "../../types";
import {
  TELEGRAM_ACCOUNT_ID_TUTORIAL_URL,
  TELEGRAM_BOT_API_TOKEN_TUTORIAL_URL,
} from "../../../active_lib";
import { buckyos, RuntimeType } from "buckyos";

type Props = {
  wizardData: WizardData;
  onUpdate: (data: Partial<WizardData>) => void;
  onNext: () => void;
  onBack: () => void;
};

const JarvisMsgTunnelStep = ({
  wizardData,
  onUpdate,
  onNext,
  onBack,
}: Props) => {
  const { t } = useTranslation();
  const [showBotToken, setShowBotToken] = useState(false);

  const config = wizardData.jarvis_msg_tunnel_config;
  const hasBotToken = config.telegram_bot_api_token.trim().length > 0;
  const hasAccountId = config.telegram_account_id.trim().length > 0;
  const isEmpty = !hasBotToken && !hasAccountId;
  const isComplete = hasBotToken && hasAccountId;
  const isPartial = !isEmpty && !isComplete;
  const primaryLabel = isEmpty ? t("skip_button") : t("next_button");

  const errorMessage = useMemo(() => {
    if (!isPartial) {
      return "";
    }
    return t("error_telegram_tunnel_incomplete") || "";
  }, [isPartial, t]);

  const openExternal = (url: string) => {
    if (!url) {
      return;
    }
    if (buckyos.getRuntimeType?.() === RuntimeType.AppRuntime) {
      buckyos.openExternal?.(url);
      return;
    }
    window.open(url, "_blank", "noopener,noreferrer");
  };

  const updateField = (
    field: "telegram_bot_api_token" | "telegram_account_id",
    value: string
  ) => {
    onUpdate({
      jarvis_msg_tunnel_config: {
        ...config,
        [field]: value,
      },
    });
  };

  const handleNext = () => {
    const normalizedConfig = {
      telegram_bot_api_token: config.telegram_bot_api_token.trim(),
      telegram_account_id: config.telegram_account_id.trim(),
    };

    onUpdate({
      jarvis_msg_tunnel_config: normalizedConfig,
    });

    if (normalizedConfig.telegram_bot_api_token && normalizedConfig.telegram_account_id) {
      onNext();
      return;
    }

    if (!normalizedConfig.telegram_bot_api_token && !normalizedConfig.telegram_account_id) {
      onNext();
    }
  };

  return (
    <Stack spacing={3}>
      <Stack spacing={1.5}>
        <Alert icon={<SendRounded />} severity="info">
          {t("jarvis_msg_tunnel_description")}
        </Alert>
        <Stack direction={{ xs: "column", sm: "row" }} spacing={1.5}>
          <Button
            variant="outlined"
            endIcon={<LaunchRounded />}
            onClick={() => openExternal(TELEGRAM_BOT_API_TOKEN_TUTORIAL_URL)}
            sx={{ minHeight: 44 }}
          >
            {t("jarvis_bot_token_tutorial_link")}
          </Button>
          <Button
            variant="outlined"
            endIcon={<LaunchRounded />}
            onClick={() => openExternal(TELEGRAM_ACCOUNT_ID_TUTORIAL_URL)}
            sx={{ minHeight: 44 }}
          >
            {t("jarvis_account_id_tutorial_link")}
          </Button>
        </Stack>
      </Stack>

      <Paper
        variant="outlined"
        sx={{
          p: 2,
          borderRadius: 3,
        }}
      >
        <Stack spacing={2}>
          <Box>
            <Typography fontWeight={700}>
              {t("jarvis_telegram_card_title")}
            </Typography>
            <Typography variant="body2" color="text.secondary">
              {t("jarvis_telegram_card_desc")}
            </Typography>
          </Box>

          <TextField
            label={t("jarvis_bot_api_token_label")}
            type={showBotToken ? "text" : "password"}
            value={config.telegram_bot_api_token}
            onChange={(event) =>
              updateField("telegram_bot_api_token", event.target.value)
            }
            InputProps={{
              endAdornment: (
                <InputAdornment position="end">
                  <IconButton
                    onClick={() => setShowBotToken((prev) => !prev)}
                    edge="end"
                    aria-label={t("toggle_secret_visibility")}
                  >
                    {showBotToken ? (
                      <VisibilityOffRounded />
                    ) : (
                      <VisibilityRounded />
                    )}
                  </IconButton>
                </InputAdornment>
              ),
            }}
            fullWidth
          />

          <TextField
            label={t("jarvis_telegram_account_id_label")}
            value={config.telegram_account_id}
            onChange={(event) =>
              updateField("telegram_account_id", event.target.value)
            }
            helperText={
              <Link
                component="button"
                type="button"
                underline="hover"
                onClick={() => openExternal(TELEGRAM_ACCOUNT_ID_TUTORIAL_URL)}
              >
                {t("jarvis_account_id_inline_link")}
              </Link>
            }
            fullWidth
          />
        </Stack>
      </Paper>

      {errorMessage ? <Alert severity="warning">{errorMessage}</Alert> : null}

      <Alert severity="info">{t("jarvis_msg_tunnel_footer_note")}</Alert>

      <Stack
        direction="row"
        justifyContent="space-between"
        spacing={1.5}
        flexWrap="wrap"
        alignItems="center"
      >
        <Button variant="text" onClick={onBack}>
          {t("back_button")}
        </Button>
        <Button
          variant="contained"
          onClick={handleNext}
          disabled={isPartial}
          sx={{ py: 1.15, minWidth: 160 }}
        >
          {primaryLabel}
        </Button>
      </Stack>
    </Stack>
  );
};

export default JarvisMsgTunnelStep;
