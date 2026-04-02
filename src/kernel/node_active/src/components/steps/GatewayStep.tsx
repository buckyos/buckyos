import {
  Alert,
  Box,
  Button,
  FormControl,
  Grid,
  InputLabel,
  MenuItem,
  Select,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import { CloudOutlined, CloudSyncRounded, LanRounded, WifiRounded } from "@mui/icons-material";
import { ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { GatewayType, WizardData } from "../../types";

type Props = {
  wizardData: WizardData;
  onUpdate: (data: Partial<WizardData>) => void;
  onNext: () => void;
  isWalletRuntime: boolean;
};

const GatewayStep = ({ wizardData, onUpdate, onNext, isWalletRuntime }: Props) => {
  const { t } = useTranslation();
  const [mode, setMode] = useState<GatewayType>(
    wizardData.gatewy_type === GatewayType.WAN ? GatewayType.WAN : wizardData.gatewy_type || GatewayType.BuckyForward,
  );
  const [portMappingMode, setPortMappingMode] = useState<WizardData["port_mapping_mode"]>(
    wizardData.port_mapping_mode || "full",
  );
  const [rtcpPort, setRtcpPort] = useState<string>((wizardData.rtcp_port ?? 2980).toString());
  const [formError, setFormError] = useState("");

  const handleNext = () => {
    setFormError("");

    if (mode !== GatewayType.WAN && portMappingMode === "rtcp_only") {
      const port = parseInt(rtcpPort, 10);
      if (Number.isNaN(port) || port < 1 || port > 65535) {
        setFormError(t("error_invalid_port") || "Invalid port");
        return;
      }
    }

    const port = parseInt(rtcpPort, 10);
    const finalPortMode = mode === GatewayType.WAN ? "full" : portMappingMode;
    const gatewayType = mode === GatewayType.WAN ? GatewayType.WAN : mode;

    onUpdate({
      gatewy_type: gatewayType,
      port_mapping_mode: finalPortMode,
      rtcp_port: Number.isNaN(port) ? 2980 : port,
      is_wallet_runtime: isWalletRuntime,
    });
    onNext();
  };

  const renderCard = (
    title: "bucky" | "direct" | "vps",
    description: string,
    selected: boolean,
    icon: ReactNode,
  ) => (
    <Box
      onClick={() => {
        if (title === "bucky") {
          setMode(GatewayType.BuckyForward);
        } else if (title === "direct") {
          setMode(GatewayType.PortForward);
          setPortMappingMode("full");
        } else {
          setMode(GatewayType.WAN);
          setPortMappingMode("full");
        }
      }}
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
            width: 28,
            height: 28,
            display: "grid",
            placeItems: "center",
            flexShrink: 0,
            color: selected ? "primary.main" : "text.secondary",
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
              : title === "vps"
              ? t("public_ip_desc")
              : t("direct_connect_desc") || "Direct routes leverage your own NAT/port mapping setup."}
          </Typography>
        </Box>
      </Stack>
    </Box>
  );

  const portHint = mode === GatewayType.WAN
    ? t("public_ip_hint")
    : portMappingMode === "rtcp_only"
    ? t("port_mapping_rtcp_only_hint")
    : t("port_mapping_full_hint");

  return (
    <Stack spacing={3}>
      <Grid container spacing={2}>
        <Grid item xs={12} md={4}>
          {renderCard(
            "bucky",
            t("use_buckyos_sn"),
            mode === GatewayType.BuckyForward,
            <CloudSyncRounded />,
          )}
        </Grid>
        <Grid item xs={12} md={4}>
          {renderCard(
            "direct",
            t("direct_connect_label"),
            mode === GatewayType.PortForward,
            <LanRounded />,
          )}
        </Grid>
        <Grid item xs={12} md={4}>
          {renderCard(
            "vps",
            t("public_ip_option"),
            mode === GatewayType.WAN,
            <CloudOutlined />,
          )}
        </Grid>
      </Grid>

      {mode === GatewayType.BuckyForward ? (
        <Stack spacing={2}>
          <Typography variant="subtitle1" fontWeight={600}>
            {t("use_buckyos_sn")}
          </Typography>
          <Alert severity="info">{t("bucky_forward_desc")}</Alert>
        </Stack>
      ) : (
        <Stack spacing={2}>
          <Typography variant="subtitle1" fontWeight={600}>
            {mode === GatewayType.WAN ? t("public_ip_desc") : t("direct_connect_desc")}
          </Typography>
          {mode !== GatewayType.WAN ? (
            <>
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
              {portMappingMode === "rtcp_only" ? (
                <TextField
                  type="number"
                  label={t("rtcp_port_label")}
                  value={rtcpPort}
                  onChange={(e) => setRtcpPort(e.target.value)}
                  helperText={t("rtcp_port_placeholder")}
                  fullWidth
                />
              ) : null}
            </>
          ) : null}
          <Alert icon={<WifiRounded fontSize="small" />} severity="info">
            {portHint}
          </Alert>
        </Stack>
      )}

      {formError ? <Alert severity="error">{formError}</Alert> : null}
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
