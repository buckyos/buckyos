import { CheckCircleOutlineRounded, ContentCopyRounded, LaunchRounded } from "@mui/icons-material";
import { Alert, Button, Paper, Snackbar, Stack, Typography } from "@mui/material";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { WizardData } from "../../types";
import { WEB3_BASE_HOST } from "../../../active_lib";
import { copyTextToClipboard } from "../../utils/clipboard";

type Props = {
  wizardData: WizardData;
  targetUrl: string;
};

const SuccessStep = ({ wizardData, targetUrl }: Props) => {
  const { t } = useTranslation();
  const [copyFeedback, setCopyFeedback] = useState<{
    open: boolean;
    severity: "success" | "error";
    message: string;
  }>({
    open: false,
    severity: "success",
    message: "",
  });
  const url =
    targetUrl ||
    (wizardData.use_self_domain
      ? `https://${wizardData.self_domain}`
      : `https://${wizardData.sn_user_name}.${WEB3_BASE_HOST}`);

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
        {url && (
          <Typography variant="h6" sx={{ wordBreak: "break-all" }}>
            {url}
          </Typography>
        )}
        <Typography variant="body2" color="text.secondary">
          {t("default_credentials")}
        </Typography>
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
