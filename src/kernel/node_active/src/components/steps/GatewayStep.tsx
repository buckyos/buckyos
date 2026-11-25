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
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ensureSnSession, get_net_id_by_gateway_type, listPublicAddresses, loadCurrentRelay } from "../../../active_lib";
import { isPublicIPv4, isPublicIPv6 } from "../../domain";
import { GatewayType, WizardData } from "../../types";
import RegionProbePanel, { CurrentRelayPanel } from "../RegionProbePanel";
import { regionMatters, type RegionProbe } from "../useRegionProbe";
import { ChoiceCard, Lead, Note, StepActions, mono } from "../ui";

type Props = {
  wizardData: WizardData;
  onUpdate: (data: Partial<WizardData>) => void;
  onNext: () => void;
  region: RegionProbe;
};

const GatewayStep = ({ wizardData, onUpdate, onNext, region }: Props) => {
  const { t } = useTranslation();
  const [mode, setMode] = useState<GatewayType>(wizardData.gatewy_type);
  const [portMappingMode, setPortMappingMode] = useState(wizardData.port_mapping_mode);
  const [rtcpPort, setRtcpPort] = useState(wizardData.rtcp_port.toString());
  const [ipv4, setIpv4] = useState(wizardData.public_ips.find((ip) => !ip.includes(":")) ?? "");
  const [ipv6, setIpv6] = useState(wizardData.public_ips.find((ip) => ip.includes(":")) ?? "");
  const [addressesLoaded, setAddressesLoaded] = useState(wizardData.public_ips.length > 0);
  const [relay, setRelay] = useState<string | null>(null);
  const [formError, setFormError] = useState("");

  const netId = get_net_id_by_gateway_type(mode, portMappingMode);
  const accountExists = wizardData.owner_document !== null;

  useEffect(() => {
    if (mode !== GatewayType.WAN || addressesLoaded) return;
    let cancelled = false;
    listPublicAddresses()
      .catch(() => [])
      .then((addresses) => {
        if (cancelled) return;
        setIpv4((current) => current || addresses.find((ip) => !ip.includes(":")) || "");
        setIpv6((current) => current || addresses.find((ip) => ip.includes(":")) || "");
        setAddressesLoaded(true);
      });
    return () => {
      cancelled = true;
    };
  }, [mode, addressesLoaded]);

  useEffect(() => {
    if (!accountExists) return;
    let cancelled = false;
    void (async () => {
      const session = await ensureSnSession(wizardData);
      if (!session) return;
      onUpdate(session);
      const current = await loadCurrentRelay(session.sn_access_token).catch(() => null);
      if (!cancelled) setRelay(current);
    })();
    return () => {
      cancelled = true;
    };
  }, [accountExists]);

  const handleNext = () => {
    const port = parseInt(rtcpPort, 10);
    if (mode === GatewayType.PortForward && portMappingMode === "rtcp_only" && !(port >= 1 && port <= 65535)) {
      return setFormError(t("error_invalid_port"));
    }
    const publicIps = [ipv4.trim(), ipv6.trim().toLowerCase()].filter(Boolean);
    if (mode === GatewayType.WAN) {
      if (!isPublicIPv4(ipv4.trim())) return setFormError(t("error_public_ipv4"));
      if (ipv6.trim() && !isPublicIPv6(ipv6.trim())) return setFormError(t("error_public_ipv6"));
    }
    setFormError("");
    onUpdate({
      gatewy_type: mode,
      port_mapping_mode: mode === GatewayType.PortForward ? portMappingMode : "full",
      rtcp_port: port >= 1 && port <= 65535 ? port : 2980,
      public_ips: mode === GatewayType.WAN ? publicIps : [],
    });
    onNext();
  };

  const choose = (next: GatewayType) => {
    setMode(next);
    setFormError("");
    if (next !== GatewayType.PortForward) setPortMappingMode("full");
  };

  const portHint =
    mode === GatewayType.WAN
      ? t("public_ip_hint")
      : portMappingMode === "rtcp_only"
        ? t("port_mapping_rtcp_only_hint")
        : t("port_mapping_full_hint");

  const iconProps = { size: 20, strokeWidth: 1.75 };
  const options = [
    {
      type: GatewayType.BuckyForward,
      icon: <Network {...iconProps} />,
      title: t("gateway_sn_relay_title"),
      description: t("use_buckyos_sn"),
    },
    {
      type: GatewayType.PortForward,
      icon: <Router {...iconProps} />,
      title: t("gateway_direct_title"),
      description: t("direct_connect_desc"),
    },
    {
      type: GatewayType.WAN,
      icon: <Server {...iconProps} />,
      title: t("gateway_public_ip_title"),
      description: t("public_ip_desc"),
    },
  ];

  const ipInput = { autoCapitalize: "none", autoComplete: "off", spellCheck: false, style: { ...mono, fontSize: 14 } };

  return (
    <Stack spacing={3}>
      <Box
        role="radiogroup"
        aria-label={t("title_step_gateway")}
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
          {mode === GatewayType.PortForward ? (
            <>
              <FormControl fullWidth>
                <InputLabel id="port-mode-label">{t("port_mapping_mode_label")}</InputLabel>
                <Select
                  labelId="port-mode-label"
                  value={portMappingMode}
                  label={t("port_mapping_mode_label")}
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
                  label={t("rtcp_port_label")}
                  value={rtcpPort}
                  onChange={(e) => setRtcpPort(e.target.value)}
                  helperText={t("rtcp_port_placeholder")}
                  inputProps={{ inputMode: "numeric", min: 1, max: 65535 }}
                />
              ) : null}
            </>
          ) : (
            <Stack direction={{ xs: "column", sm: "row" }} spacing={2}>
              <TextField
                required
                label={t("public_ipv4_label")}
                value={ipv4}
                onChange={(event) => setIpv4(event.target.value)}
                helperText={
                  addressesLoaded && !ipv4 ? t("public_ip_not_found") : t("public_ip_confirm")
                }
                inputProps={ipInput}
              />
              <TextField
                label={t("public_ipv6_label")}
                value={ipv6}
                onChange={(event) => setIpv6(event.target.value)}
                inputProps={ipInput}
              />
            </Stack>
          )}
          <Note>{portHint}</Note>
        </Stack>
      )}

      {regionMatters(netId) &&
        (accountExists ? <CurrentRelayPanel relay={relay} /> : <RegionProbePanel probe={region} />)}

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
