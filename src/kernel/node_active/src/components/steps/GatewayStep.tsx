import {
  Alert,
  Box,
  Button,
  FormControl,
  Grid,
  InputLabel,
  MenuItem,
  Paper,
  Select,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import { CloudSyncRounded, LanRounded, WifiRounded } from "@mui/icons-material";
import { ReactNode, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { GatewayType, WizardData } from "../../types";
import { SN_API_URL, WEB3_BASE_HOST, check_sn_active_code, set_sn_api_url } from "@legacy/active_lib";

type Props = {
  wizardData: WizardData;
  onUpdate: (data: Partial<WizardData>) => void;
  onNext: () => void;
};

const GatewayStep = ({ wizardData, onUpdate, onNext }: Props) => {
  const { t } = useTranslation();
  const [mode, setMode] = useState<GatewayType>(wizardData.gatewy_type || GatewayType.BuckyForward);
  const [inviteCode, setInviteCode] = useState(wizardData.sn_active_code || "");
  const [checkingInvite, setCheckingInvite] = useState(false);
  const [inviteValid, setInviteValid] = useState<boolean | null>(null);
  const [portMappingMode, setPortMappingMode] = useState<WizardData["port_mapping_mode"]>(
    wizardData.port_mapping_mode || "full"
  );
  const [rtcpPort, setRtcpPort] = useState<string>((wizardData.rtcp_port ?? 2980).toString());
  const [formError, setFormError] = useState("");

  useEffect(() => {
    if (mode !== GatewayType.BuckyForward || inviteCode.length < 7) {
      setInviteValid(null);
      return;
    }
    let cancelled = false;
    setCheckingInvite(true);
    check_sn_active_code(inviteCode)
      .then((ok) => {
        if (!cancelled) {
          setInviteValid(ok);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setInviteValid(false);
        }
      })
      .finally(() => {
        if (!cancelled) {
          setCheckingInvite(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [inviteCode, mode]);

  const handleNext = () => {
    setFormError("");

    if (mode === GatewayType.BuckyForward) {
      if (!inviteCode || inviteCode.length < 8) {
        setFormError(t("error_invite_code_too_short") || "Invitation code is required");
        return;
      }
      if (inviteValid === false) {
        setFormError(t("error_invite_code_invalid") || "Invitation code is invalid");
        return;
      }
    } else if (portMappingMode === "rtcp_only") {
      const port = parseInt(rtcpPort, 10);
      if (Number.isNaN(port) || port < 1 || port > 65535) {
        setFormError(t("error_invalid_port") || "Invalid port");
        return;
      }
    }

    const port = parseInt(rtcpPort, 10);
    const nextData: Partial<WizardData> = {
      gatewy_type: mode,
      is_direct_connect: mode === GatewayType.PortForward,
      sn_active_code: inviteCode,
      sn_url: SN_API_URL,
      web3_base_host: WEB3_BASE_HOST,
      port_mapping_mode: portMappingMode,
      rtcp_port: Number.isNaN(port) ? 2980 : port,
    };

    set_sn_api_url(SN_API_URL);
    onUpdate(nextData);
    onNext();
  };

  const renderCard = (title: string, description: string, selected: boolean, icon: ReactNode) => (
    <Paper
      onClick={() => setMode(title === "bucky" ? GatewayType.BuckyForward : GatewayType.PortForward)}
      sx={{
        p: 2,
        borderRadius: 3,
        height: "100%",
        cursor: "pointer",
        border: `1px solid ${selected ? "transparent" : "divider"}`,
        boxShadow: selected ? "0 10px 40px rgba(79,70,229,0.18)" : "none",
        background: selected ? "linear-gradient(135deg, rgba(79,70,229,0.12), rgba(0,172,255,0.12))" : undefined,
        backgroundColor: selected ? undefined : "background.paper",
        transition: "all 0.25s ease",
        "&:hover": {
          boxShadow: "0 10px 40px rgba(0,0,0,0.12)",
          transform: "translateY(-2px)",
        },
      }}
    >
      <Stack direction="row" spacing={1.5} alignItems="flex-start">
        <Box
          sx={{
            width: 44,
            height: 44,
            borderRadius: "12px",
            display: "grid",
            placeItems: "center",
            backgroundColor: selected ? "primary.main" : "action.hover",
            color: selected ? "primary.contrastText" : "text.secondary",
          }}
        >
          {icon}
        </Box>
        <Box>
          <Typography fontWeight={700} gutterBottom>
            {description}
          </Typography>
          <Typography variant="body2" color="text.secondary">
            {title === "bucky"
              ? t("bucky_forward_benefit1")
              : t("direct_connect_desc") || "Direct routes leverage your own NAT/port mapping setup."}
          </Typography>
        </Box>
      </Stack>
    </Paper>
  );

  const portHint =
    portMappingMode === "rtcp_only" ? t("port_mapping_rtcp_only_hint") : t("port_mapping_full_hint");

  return (
    <Stack spacing={3}>
      <Grid container spacing={2}>
        <Grid item xs={12} md={6}>
          {renderCard(
            "bucky",
            t("use_buckyos_sn"),
            mode === GatewayType.BuckyForward,
            <CloudSyncRounded />
          )}
        </Grid>
        <Grid item xs={12} md={6}>
          {renderCard(
            "direct",
            t("direct_connect_label"),
            mode === GatewayType.PortForward,
            <LanRounded />
          )}
        </Grid>
      </Grid>

      {mode === GatewayType.BuckyForward ? (
        <Stack spacing={2}>
          <Typography variant="subtitle1" fontWeight={600}>
            {t("use_buckyos_sn")}
          </Typography>
          <TextField
            label={t("invite_code_placeholder")}
            value={inviteCode}
            onChange={(e) => setInviteCode(e.target.value)}
            error={inviteValid === false}
            helperText={
              checkingInvite
                ? t("invite_checking")
                : inviteValid === false
                ? t("error_invite_code_invalid")
                : inviteValid === true
                ? t("invite_valid")
                : t("bucky_forward_desc")
            }
            fullWidth
            required
          />
        </Stack>
      ) : (
        <Stack spacing={2}>
          <Typography variant="subtitle1" fontWeight={600}>
            {t("direct_connect_desc")}
          </Typography>
          <FormControl fullWidth>
            <InputLabel id="port-mode-label">{t("port_mapping_mode_label")}</InputLabel>
            <Select
              labelId="port-mode-label"
              value={portMappingMode}
              label={t("port_mapping_mode_label")}
              onChange={(e) => setPortMappingMode(e.target.value as WizardData["port_mapping_mode"])}
            >
              <MenuItem value="full">{t("port_mapping_full")}</MenuItem>
              <MenuItem value="rtcp_only">{t("port_mapping_rtcp_only")}</MenuItem>
            </Select>
          </FormControl>
          {portMappingMode === "rtcp_only" && (
            <TextField
              type="number"
              label={t("rtcp_port_label")}
              value={rtcpPort}
              onChange={(e) => setRtcpPort(e.target.value)}
              helperText={t("rtcp_port_placeholder")}
              fullWidth
            />
          )}
          <Alert icon={<WifiRounded fontSize="small" />} severity="info">
            {portHint}
          </Alert>
        </Stack>
      )}

      {formError && <Alert severity="error">{formError}</Alert>}
      <Stack direction="row" justifyContent="flex-end" spacing={1.5} flexWrap="wrap" alignItems="center">
        <Button
          variant="contained"
          size="large"
          onClick={handleNext}
          sx={{ py: 1.15, minWidth: 160 }}
        >
          {t("next_button")}
        </Button>
      </Stack>
    </Stack>
  );
};

export default GatewayStep;
