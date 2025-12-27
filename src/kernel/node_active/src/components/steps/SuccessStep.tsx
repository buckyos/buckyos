import { CheckCircleOutlineRounded, ContentCopyRounded, LaunchRounded } from "@mui/icons-material";
import { Button, Paper, Stack, Typography } from "@mui/material";
import { useTranslation } from "react-i18next";
import { WizardData } from "../../types";
import { WEB3_BASE_HOST } from "../../../active_lib";

type Props = {
  wizardData: WizardData;
  targetUrl: string;
};

const SuccessStep = ({ wizardData, targetUrl }: Props) => {
  const { t } = useTranslation();
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

  const copyUrl = () => {
    if (url) {
      navigator.clipboard?.writeText(url);
    }
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
    </Paper>
  );
};

export default SuccessStep;
