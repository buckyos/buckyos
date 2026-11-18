import { Check, Copy, ExternalLink } from "lucide-react";
import { Alert, Box, Button, Snackbar, Stack, Typography } from "@mui/material";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { WizardData } from "../../types";
import { SN_BASE_HOST, WEB3_BASE_HOST } from "../../../active_lib";
import { copyTextToClipboard } from "../../utils/clipboard";
import { Note, StatusDot, mono } from "../ui";

type Props = {
  wizardData: WizardData;
  targetUrl: string;
};

const ACTIVATION_COUNTDOWN_SECONDS = 120;

const SuccessStep = ({ wizardData, targetUrl }: Props) => {
  const { t } = useTranslation();
  const [remainingSeconds, setRemainingSeconds] = useState(ACTIVATION_COUNTDOWN_SECONDS);
  const [opening, setOpening] = useState(false);
  const [feedback, setFeedback] = useState<{
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
  const loginUsername = wizardData.owner_document?.name || wizardData.sn_user_name?.trim() || "";
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

  const openUrl = async () => {
    if (!url || opening) return;

    const buckyApi = (window as Window & {
      BuckyApi?: {
        openExternalUrl?: (url: string) => Promise<{ code: number }>;
      };
    }).BuckyApi;
    if (!buckyApi) {
      window.location.href = url;
      return;
    }

    setOpening(true);
    try {
      const result = await buckyApi.openExternalUrl?.(url);
      if (result?.code !== 0) throw new Error("open_external_url_failed");
    } catch {
      setFeedback({
        open: true,
        severity: "error",
        message: t("error_open_failed", "Could not open the browser. Please copy the link and open it manually."),
      });
    } finally {
      setOpening(false);
    }
  };

  const copyUrl = async () => {
    if (!url) {
      return;
    }

    const copied = await copyTextToClipboard(url);
    setFeedback({
      open: true,
      severity: copied ? "success" : "error",
      message: copied
        ? t("success_copied")
        : t("error_copy_failed", "Copy failed. Please copy it manually."),
    });
  };

  const stillCompleting = remainingSeconds > 0;

  return (
    <Stack spacing={4} alignItems="center" sx={{ textAlign: "center", py: { xs: 1, sm: 2 } }}>
      <Stack spacing={1.5} alignItems="center">
        <Box
          sx={{
            width: 40,
            height: 40,
            borderRadius: "50%",
            display: "grid",
            placeItems: "center",
            color: "var(--cp-success)",
            bgcolor: "color-mix(in srgb, var(--cp-success) 14%, var(--cp-surface))",
          }}
        >
          <Check size={20} strokeWidth={2} />
        </Box>
        <Typography variant="h5" component="h2">
          {t("activation_success")}
        </Typography>
        <Typography variant="body2" color="text.secondary" sx={{ maxWidth: 480 }}>
          {t("activation_success_desc")}
        </Typography>
      </Stack>

      {wizardData.use_self_domain && (
        <Box sx={{ width: "100%", textAlign: "left" }}>
          <Note title={t("dns_ns_record", { sn_host_base: SN_BASE_HOST })}>
            {t(
              "ns_after_activation_hint",
              "Activation is complete. Delegate NS now to make the custom domain resolve publicly.",
            )}
          </Note>
        </Box>
      )}

      <Stack spacing={1} alignItems="center" sx={{ width: "100%", maxWidth: 480 }}>
        <Stack direction="row" spacing={1} alignItems="center">
          <StatusDot tone={stillCompleting ? "accent" : "success"} live={stillCompleting} />
          <Typography sx={{ ...mono, fontSize: 20, fontWeight: 500, lineHeight: 1.4 }}>{countdownLabel}</Typography>
        </Stack>
        <Typography variant="body2" color="text.secondary">
          {t("refresh_note", {
            time: countdownLabel,
            defaultValue:
              "Activation is still completing. It should be ready in about {{time}}. If the page shows errors, refresh a few more times.",
          })}
        </Typography>
        <Box
          role="progressbar"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={Math.round(countdownProgress)}
          sx={{ width: "100%", height: 4, mt: 1, borderRadius: 2, bgcolor: "var(--cp-border)", overflow: "hidden" }}
        >
          <Box
            sx={{
              height: "100%",
              width: "100%",
              bgcolor: "var(--cp-accent)",
              transformOrigin: "left center",
              transform: `scaleX(${countdownProgress / 100})`,
              transition: "transform 1000ms linear",
            }}
          />
        </Box>
      </Stack>

      {url && (
        <Box
          sx={{
            width: "100%",
            maxWidth: 480,
            px: 2,
            py: 2,
            borderRadius: "12px",
            border: "1px solid var(--cp-border)",
            bgcolor: "var(--cp-surface-2)",
          }}
        >
          <Typography sx={{ ...mono, fontSize: 17, fontWeight: 500, overflowWrap: "anywhere", userSelect: "all" }}>
            {url}
          </Typography>
          {loginUsername && (
            <Typography variant="body2" color="text.secondary" sx={{ mt: 0.5 }}>
              {t("default_credentials", { username: loginUsername })}
            </Typography>
          )}
        </Box>
      )}

      <Stack direction={{ xs: "column", sm: "row" }} spacing={1.5} sx={{ width: { xs: "100%", sm: "auto" } }}>
        <Button
          variant="contained"
          size="large"
          endIcon={<ExternalLink size={16} strokeWidth={1.75} />}
          onClick={openUrl}
          disabled={opening || !url}
        >
          {t("close_and_redirect")}
        </Button>
        <Button
          variant="outlined"
          size="large"
          startIcon={<Copy size={16} strokeWidth={1.75} />}
          onClick={copyUrl}
          disabled={!url}
        >
          {t("copy_link")}
        </Button>
      </Stack>
      <Snackbar
        open={feedback.open}
        autoHideDuration={2500}
        onClose={() => setFeedback((prev) => ({ ...prev, open: false }))}
        anchorOrigin={{ vertical: "bottom", horizontal: "center" }}
      >
        <Alert
          onClose={() => setFeedback((prev) => ({ ...prev, open: false }))}
          severity={feedback.severity}
          sx={{ width: "100%", boxShadow: "var(--cp-panel-shadow)" }}
        >
          {feedback.message}
        </Alert>
      </Snackbar>
    </Stack>
  );
};

export default SuccessStep;
