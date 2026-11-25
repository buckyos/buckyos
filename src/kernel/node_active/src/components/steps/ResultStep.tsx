import { Check, Copy, ExternalLink, RotateCw, TriangleAlert } from "lucide-react";
import { Alert, Box, Button, CircularProgress, Snackbar, Stack, Typography } from "@mui/material";
import { QRCodeSVG } from "qrcode.react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { deriveGatewayTopology } from "../../../active_lib";
import { certificateUntrusted } from "../../domain";
import { createStartupMonitor, type StartupState } from "../../startup";
import { diagnoseStartup, probeReadme } from "../../startupProbe";
import { WizardData } from "../../types";
import { copyTextToClipboard } from "../../utils/clipboard";
import { openExternalUrl } from "../../utils/external";
import { StatusDot, mono } from "../ui";
import type { Activated } from "./ReviewStep";

type Props = {
  wizardData: WizardData;
  activated: Activated;
};

const Badge = ({ tone, children }: { tone: "success" | "danger" | "warning"; children: ReactNode }) => (
  <Box
    sx={{
      width: 40,
      height: 40,
      borderRadius: "50%",
      display: "grid",
      placeItems: "center",
      color: `var(--cp-${tone})`,
      bgcolor: `color-mix(in srgb, var(--cp-${tone}) 14%, var(--cp-surface))`,
    }}
  >
    {children}
  </Box>
);

const Heading = ({ title, children }: { title: string; children?: ReactNode }) => (
  <Stack spacing={1} alignItems="center">
    <Typography variant="h5" component="h2">
      {title}
    </Typography>
    {children ? (
      <Typography variant="body2" color="text.secondary" component="div" sx={{ maxWidth: 520, whiteSpace: "pre-line" }}>
        {children}
      </Typography>
    ) : null}
  </Stack>
);

/** Everything here must already be loaded: the activation service is gone two seconds after the commit. */
const ResultStep = ({ wizardData, activated }: Props) => {
  const { t } = useTranslation();
  const [state, setState] = useState<StartupState>({ phase: "starting" });
  const [rechecking, setRechecking] = useState(false);
  const [feedback, setFeedback] = useState<{ open: boolean; severity: "success" | "error"; message: string }>({
    open: false,
    severity: "success",
    message: "",
  });
  const monitor = useRef<ReturnType<typeof createStartupMonitor> | null>(null);
  const { accessHostname, zoneDid, activatedAt } = activated;
  const url = `https://${accessHostname}`;
  const loginUsername = wizardData.owner_document?.name || wizardData.sn_user_name?.trim() || "";
  const netId = deriveGatewayTopology(wizardData).net_id;
  const untrusted = certificateUntrusted(wizardData);

  useEffect(() => {
    const current = createStartupMonitor({
      activatedAt,
      certificateTrusted: !untrusted,
      probe: () => probeReadme(accessHostname),
      diagnose: () => diagnoseStartup(window.location.hostname, zoneDid, accessHostname),
      onChange: setState,
    });
    monitor.current = current;
    current.start();
    const wake = () => {
      if (!document.hidden) current.wake();
    };
    document.addEventListener("visibilitychange", wake);
    return () => {
      document.removeEventListener("visibilitychange", wake);
      current.stop();
    };
  }, [accessHostname, zoneDid, activatedAt, untrusted]);

  const openUrl = async () => {
    if (await openExternalUrl(url)) return;
    setFeedback({ open: true, severity: "error", message: t("error_open_failed") });
  };

  const copyUrl = async () => {
    const copied = await copyTextToClipboard(url);
    setFeedback({
      open: true,
      severity: copied ? "success" : "error",
      message: t(copied ? "success_copied" : "error_copy_failed"),
    });
  };

  const recheck = async () => {
    setRechecking(true);
    await monitor.current?.recheck();
    setRechecking(false);
  };

  const iconProps = { size: 16, strokeWidth: 1.75 };
  const copyButton = (
    <Button variant="outlined" size="large" startIcon={<Copy {...iconProps} />} onClick={copyUrl}>
      {t("copy_link")}
    </Button>
  );
  const recheckButton = (variant: "contained" | "outlined") => (
    <Button
      variant={variant}
      size="large"
      startIcon={<RotateCw {...iconProps} />}
      onClick={recheck}
      disabled={rechecking}
    >
      {t("result_recheck")}
    </Button>
  );
  const openButton = (variant: "contained" | "outlined", label: string) => (
    <Button variant={variant} size="large" endIcon={<ExternalLink {...iconProps} />} onClick={openUrl}>
      {label}
    </Button>
  );

  const address = (
    <Box
      sx={{
        width: "100%",
        maxWidth: 480,
        p: 2,
        borderRadius: "12px",
        border: "1px solid var(--cp-border)",
        bgcolor: "var(--cp-surface-2)",
      }}
    >
      <Typography sx={{ ...mono, fontSize: 17, fontWeight: 500, overflowWrap: "anywhere", userSelect: "all" }}>{url}</Typography>
      {loginUsername && (
        <Typography variant="body2" color="text.secondary" sx={{ mt: 0.5 }}>
          {t("default_credentials", { username: loginUsername })}
        </Typography>
      )}
    </Box>
  );

  const qrCode = (
    <Stack spacing={1} alignItems="center" sx={{ display: { xs: "none", sm: "flex" } }} data-testid="result-qr">
      <Box sx={{ p: 2, borderRadius: "12px", border: "1px solid var(--cp-border)", bgcolor: "#ffffff", lineHeight: 0 }}>
        <QRCodeSVG value={url} size={160} bgColor="#ffffff" fgColor="#000000" level="M" />
      </Box>
      <Typography variant="caption" color="text.secondary">
        {t("result_qr_hint")}
      </Typography>
    </Stack>
  );

  const actions = (children: ReactNode) => (
    <Stack direction={{ xs: "column", sm: "row" }} spacing={1.5} sx={{ width: { xs: "100%", sm: "auto" } }}>
      {children}
    </Stack>
  );

  const hints =
    state.phase !== "failed"
      ? []
      : state.cause === "node"
        ? ["result_hint_tunnel"]
        : state.cause === "dns"
          ? ["result_hint_dns"]
          : netId === "wan_dyn"
            ? ["result_hint_loopback"]
            : [];

  const content = () => {
    switch (state.phase) {
      case "starting":
        return (
          <>
            <CircularProgress size={28} thickness={4} />
            <Heading title={t("result_starting")}>{t("result_starting_hint")}</Heading>
          </>
        );
      case "checking":
        return (
          <>
            <CircularProgress size={28} thickness={4} />
            <Heading title={t("result_checking")}>{`public.${accessHostname}`}</Heading>
          </>
        );
      case "ready":
        return (
          <>
            <Badge tone="success">
              <Check size={20} strokeWidth={2} />
            </Badge>
            <Heading title={t("result_ready")} />
            {address}
            {qrCode}
            {actions(
              <>
                {openButton("contained", t("result_open"))}
                {copyButton}
              </>,
            )}
          </>
        );
      case "unverifiable":
        return (
          <>
            <Badge tone="warning">
              <TriangleAlert size={20} strokeWidth={2} />
            </Badge>
            <Heading title={t(state.nodeUp ? "result_node_up" : "result_unverifiable")}>
              {t("result_unverifiable_hint")}
            </Heading>
            {address}
            {qrCode}
            {actions(
              <>
                {openButton("contained", t("result_open"))}
                {recheckButton("outlined")}
                {copyButton}
              </>,
            )}
          </>
        );
      case "failed":
        return (
          <>
            <Badge tone="danger">
              <TriangleAlert size={20} strokeWidth={2} />
            </Badge>
            <Heading title={t("result_failed")}>{t(`result_cause_${state.cause}`)}</Heading>
            {address}
            <Stack spacing={1} sx={{ width: "100%", maxWidth: 520, textAlign: "left" }}>
              {hints.map((key) => (
                <Stack key={key} direction="row" spacing={1.5}>
                  <Box sx={{ display: "flex", alignItems: "center", height: 20 }}>
                    <StatusDot tone="muted" />
                  </Box>
                  <Typography variant="body2" color="text.secondary">
                    {t(key)}
                  </Typography>
                </Stack>
              ))}
            </Stack>
            {actions(
              <>
                {recheckButton("contained")}
                {openButton("outlined", t("result_open_anyway"))}
              </>,
            )}
          </>
        );
    }
  };

  return (
    <Stack spacing={3} alignItems="center" data-phase={state.phase} sx={{ textAlign: "center", py: { xs: 1, sm: 2 } }}>
      {content()}
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

export default ResultStep;
