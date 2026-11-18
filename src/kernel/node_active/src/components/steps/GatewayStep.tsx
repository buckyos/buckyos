import {
  Alert,
  Box,
  Button,
  FormControl,
  InputLabel,
  MenuItem,
  Select,
  Stack,
  TextField,
} from "@mui/material";
import { Network, Router, Server } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { GatewayType, WizardData } from "../../types";
import { ChoiceCard, Lead, Note, StepActions, plainLabel } from "../ui";

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

  const choose = (next: GatewayType) => {
    setMode(next);
    if (next !== GatewayType.BuckyForward) setPortMappingMode("full");
  };

  const portHint = mode === GatewayType.WAN
    ? t("public_ip_hint")
    : portMappingMode === "rtcp_only"
    ? t("port_mapping_rtcp_only_hint")
    : t("port_mapping_full_hint");

  const iconProps = { size: 20, strokeWidth: 1.75 };
  const options = [
    {
      type: GatewayType.BuckyForward,
      icon: <Network {...iconProps} />,
      title: t("gateway_sn_relay_title", "SN relay"),
      description: t("use_buckyos_sn"),
    },
    {
      type: GatewayType.PortForward,
      icon: <Router {...iconProps} />,
      title: t("gateway_direct_title", "Direct connection"),
      description: t("direct_connect_desc"),
    },
    {
      type: GatewayType.WAN,
      icon: <Server {...iconProps} />,
      title: t("gateway_public_ip_title", "Fixed public IP"),
      description: t("public_ip_desc"),
    },
  ];

  return (
    <Stack spacing={3}>
      <Box
        role="radiogroup"
        aria-label={t("title_step_2").replace(/^\d+\.\s*/, "")}
        sx={{
          display: "grid",
          gridTemplateColumns: { xs: "1fr", md: "repeat(3, minmax(0, 1fr))" },
          gap: 1.5,
        }}
      >
        {options.map((option) => (
          <ChoiceCard
            key={option.type}
            selected={mode === option.type}
            onSelect={() => choose(option.type)}
            icon={option.icon}
            title={option.title}
            description={option.description}
          />
        ))}
      </Box>

      {mode === GatewayType.BuckyForward ? (
        <Lead>
          {t("bucky_forward_desc")} {t("bucky_forward_benefit1")}
        </Lead>
      ) : (
        <Stack spacing={2}>
          {mode !== GatewayType.WAN ? (
            <>
              <FormControl fullWidth>
                <InputLabel id="port-mode-label">{plainLabel(t("port_mapping_mode_label"))}</InputLabel>
                <Select
                  labelId="port-mode-label"
                  value={portMappingMode}
                  label={plainLabel(t("port_mapping_mode_label"))}
                  onChange={(e) => setPortMappingMode(e.target.value as WizardData["port_mapping_mode"])}
                  sx={{ "& .MuiSelect-select": { whiteSpace: "normal" } }}
                >
                  <MenuItem value="full" sx={{ whiteSpace: "normal" }}>{t("port_mapping_full")}</MenuItem>
                  <MenuItem value="rtcp_only" sx={{ whiteSpace: "normal" }}>{t("port_mapping_rtcp_only")}</MenuItem>
                </Select>
              </FormControl>
              {portMappingMode === "rtcp_only" ? (
                <TextField
                  type="number"
                  label={plainLabel(t("rtcp_port_label"))}
                  value={rtcpPort}
                  onChange={(e) => setRtcpPort(e.target.value)}
                  helperText={t("rtcp_port_placeholder")}
                  inputProps={{ inputMode: "numeric", min: 1, max: 65535 }}
                />
              ) : null}
            </>
          ) : null}
          <Note>{portHint}</Note>
        </Stack>
      )}

      {formError ? <Alert severity="error">{formError}</Alert> : null}
      <StepActions>
        <Button variant="contained" size="large" onClick={handleNext} sx={{ minWidth: 120 }}>
          {t("next_button")}
        </Button>
      </StepActions>
    </Stack>
  );
};

export default GatewayStep;
