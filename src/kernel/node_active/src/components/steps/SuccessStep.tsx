import { CheckCircleOutlineRounded, ContentCopyRounded, LaunchRounded } from "@mui/icons-material";
import { Alert, Box, Button, LinearProgress, Paper, Snackbar, Stack, Typography } from "@mui/material";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { WizardData } from "../../types";
import { WEB3_BASE_HOST } from "../../../active_lib";
import { copyTextToClipboard } from "../../utils/clipboard";

type Props = {
  wizardData: WizardData;
  targetUrl: string;
};

const ACTIVATION_COUNTDOWN_SECONDS = 120;

const SuccessStep = ({ wizardData, targetUrl }: Props) => {
  const { t } = useTranslation();
  const [remainingSeconds, setRemainingSeconds] = useState(ACTIVATION_COUNTDOWN_SECONDS);
  const [copyFeedback, setCopyFeedback] = useState<{
    open: boolean;
    severity: "success" | "error";
    message: string;
  }>({
    open: false,
    severity: "success",
    message: "",
  });
  const url = useMemo(() => {
    if (targetUrl) {
      return targetUrl;
    }

    if (wizardData.use_self_domain) {
      return wizardData.self_domain ? `https://${wizardData.self_domain}` : "";
    }

    return wizardData.sn_user_name
      ? `https://${wizardData.sn_user_name}.${WEB3_BASE_HOST}`
      : "";
  }, [targetUrl, wizardData.self_domain, wizardData.sn_user_name, wizardData.use_self_domain]);
  const loginUsername = wizardData.owner_user_name?.trim() || wizardData.sn_user_name?.trim() || "";
  const countdownLabel = useMemo(() => {
    const minutes = String(Math.floor(remainingSeconds / 60)).padStart(2, "0");
    const seconds = String(remainingSeconds % 60).padStart(2, "0");
    return `${minutes}:${seconds}`;
  }, [remainingSeconds]);
  const countdownProgress =
    ((ACTIVATION_COUNTDOWN_SECONDS - remainingSeconds) / ACTIVATION_COUNTDOWN_SECONDS) * 100;

  useEffect(() => {
    setRemainingSeconds(ACTIVATION_COUNTDOWN_SECONDS);
    const startedAt = Date.now();
    const timer = window.setInterval(() => {
      const elapsedSeconds = Math.floor((Date.now() - startedAt) / 1000);
      const nextRemaining = Math.max(ACTIVATION_COUNTDOWN_SECONDS - elapsedSeconds, 0);
      setRemainingSeconds(nextRemaining);
      if (nextRemaining === 0) {
        window.clearInterval(timer);
      }
    }, 1000);

    return () => {
      window.clearInterval(timer);
    };
  }, []);

  const openUrl = () => {
    if (url) {
      window.location.href = url;
    }
  };

  const copyUrl = async () => {
    if (!url) {
      return;
    }

    const copied = await copyTextToClipboard(url);
    setCopyFeedback({
      open: true,
      severity: copied ? "success" : "error",
      message: copied
        ? t("success_copied")
        : t("error_copy_failed", "Copy failed. Please copy it manually."),
    });
  };

  return (
    <Paper
      variant="outlined"
      sx={{
        p: { xs: 3, md: 4 },
        borderRadius: 4,
        textAlign: "center",
      }}
    >
      <Stack spacing={2} alignItems="center">
        <CheckCircleOutlineRounded color="success" sx={{ fontSize: 56 }} />
        <Typography variant="h5">{t("activation_success")}</Typography>
        <Typography color="text.secondary">{t("activation_success_desc")}</Typography>
        <Stack spacing={1} sx={{ width: "100%", maxWidth: 420 }}>
          <Box>
            <Typography variant="h4" fontWeight={700}>
              {countdownLabel}
            </Typography>
            <Typography variant="body2" color="text.secondary">
              {t("refresh_note", {
                time: countdownLabel,
                defaultValue:
                  "Activation is still completing. It should be ready in about {{time}}. If the page shows errors, refresh a few more times.",
              })}
            </Typography>
          </Box>
          <LinearProgress
            variant="determinate"
            value={countdownProgress}
            sx={{ height: 8, borderRadius: 999 }}
          />
        </Stack>
        {url && (
          <Typography variant="h6" sx={{ wordBreak: "break-all" }}>
            {url}
          </Typography>
        )}
        {loginUsername && (
          <Typography variant="body2" color="text.secondary">
            {t("default_credentials", { username: loginUsername })}
          </Typography>
        )}
        <Stack direction={{ xs: "column", sm: "row" }} spacing={1.5}>
          <Button variant="contained" endIcon={<LaunchRounded />} onClick={openUrl}>
            {t("close_and_redirect")}
          </Button>
          <Button variant="outlined" startIcon={<ContentCopyRounded />} onClick={copyUrl}>
            {t("copy_link")}
          </Button>
        </Stack>
      </Stack>
      <Snackbar
        open={copyFeedback.open}
        autoHideDuration={2500}
        onClose={() => setCopyFeedback((prev) => ({ ...prev, open: false }))}
        anchorOrigin={{ vertical: "bottom", horizontal: "center" }}
      >
        <Alert
          onClose={() => setCopyFeedback((prev) => ({ ...prev, open: false }))}
          severity={copyFeedback.severity}
          variant="filled"
          sx={{ width: "100%" }}
        >
          {copyFeedback.message}
        </Alert>
      </Snackbar>
    </Paper>
  );
};

export default SuccessStep;
