import {
  Alert,
  Box,
  Button,
  Chip,
  CircularProgress,
  IconButton,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import { CheckCircleRounded, ContentCopyRounded, LaunchRounded, DnsRounded, WarningRounded } from "@mui/icons-material";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { GatewayType, WizardData } from "../../types";
import { do_active, do_active_by_wallet, generate_zone_txt_records, get_net_id_by_gateway_type, SN_BASE_HOST } from "../../../active_lib";

type Props = {
  wizardData: WizardData;
  onUpdate: (data: Partial<WizardData>) => void;
  onActivated: (targetUrl: string) => void;
  onBack: () => void;
  isWalletRuntime: boolean;
};

const ReviewStep = ({ wizardData, onActivated, onBack, isWalletRuntime }: Props) => {
  const { t } = useTranslation();
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const requiresDnsTxt = useMemo(
    () => wizardData.gatewy_type === GatewayType.WAN && wizardData.use_self_domain,
    [wizardData.gatewy_type, wizardData.use_self_domain]
  );
  const [dnsReady, setDnsReady] = useState(!requiresDnsTxt && !!wizardData.zone_config_jwt);
  const [dnsRecords, setDnsRecords] = useState<Array<{ key: string; value: string }>>(
    wizardData.zone_config_jwt ? [{ key: "BOOT", value: `DID=${wizardData.zone_config_jwt};` }] : []
  );
  const [dnsLoading, setDnsLoading] = useState(false);

  useEffect(() => {
    setDnsReady(!requiresDnsTxt && !!wizardData.zone_config_jwt);
    setDnsRecords(wizardData.zone_config_jwt ? [{ key: "BOOT", value: `DID=${wizardData.zone_config_jwt};` }] : []);
  }, [requiresDnsTxt, wizardData.zone_config_jwt]);

  const targetHost = wizardData.use_self_domain
    ? wizardData.self_domain
    : wizardData.sn_user_name && wizardData.web3_base_host
    ? `${wizardData.sn_user_name}.${wizardData.web3_base_host}`
    : "";
  const targetUrl = targetHost ? `https://${targetHost}` : "";

  const copyKey = () => {
    const key = wizardData.owner_private_key as string;
    if (key) {
      navigator.clipboard?.writeText(key);
    }
  };

  const handleActivate = async () => {
    setError("");
    if (requiresDnsTxt && !dnsReady) {
      setError(t("error_generate_txt_records_failed") || "DNS record not ready");
      return;
    }
    setLoading(true);
    try {
      const ok = isWalletRuntime ? await do_active_by_wallet(wizardData) : await do_active(wizardData);
      if (ok) {
        onActivated(targetUrl);
      } else {
        setError(t("error_activation_failed") || "Activation failed");
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setError(`${t("error_activation_failed") || "Activation failed"} ${msg}`);
    } finally {
      setLoading(false);
    }
  };

  const buildDnsRecord = async () => {
    setError("");

    if (!wizardData.is_wallet_runtime && !wizardData.owner_private_key) {
      setError(t("error_private_key_not_ready") || "Private key missing");
      return;
    }
    setDnsLoading(true);
    try {
      const snHost = (() => {
        try {
          return new URL(wizardData.sn_url || "https://sn.buckyos.ai").hostname;
        } catch {
          return "sn.buckyos.ai";
        }
      })();
      const netid = get_net_id_by_gateway_type(wizardData.gatewy_type, wizardData.port_mapping_mode);
      const result = await generate_zone_txt_records(
        snHost,
        wizardData.owner_public_key,
        wizardData.owner_private_key,
        wizardData.device_public_key,
        netid,
        wizardData.rtcp_port,
        wizardData.is_wallet_runtime
      );
      if (!result) {
        throw new Error("No TXT records returned");
      }
      const records = Object.entries(result).map(([key, value]) => ({
        key,
        value: typeof value === "string" ? value : JSON.stringify(value),
      }));

      setDnsRecords(records);
      setDnsReady(true);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setError(`${t("error_generate_txt_records_failed") || "Failed to generate TXT"} ${msg}`);
      setDnsReady(false);
    } finally {
      setDnsLoading(false);
    }
  };

  return (
    <Stack spacing={3}>

      <Alert icon={<CheckCircleRounded />} severity="info">
        {wizardData.use_self_domain ? (
          <span>
            {t("access_domain")}: <strong>{targetHost || t("domain_placeholder")}</strong>
          </span>
        ) : (
          <span>
            {t("access_domain")}: <strong>{targetHost || t("domain_format")}</strong>
          </span>
        )}
      </Alert>
      <Stack direction="row" spacing={1} flexWrap="wrap">
        <Chip label={wizardData.gatewy_type} color="primary" />
        <Chip
          label={wizardData.use_self_domain ? t("use_own_domain") : t("use_buckyos_domain")}
          color="secondary"
        />
        <Chip
          label={
            wizardData.enable_guest_access ? t("enable_guest_mode") : (t("guest_mode_desc") || "Private access")
          }
        />
      </Stack>

      {wizardData.use_self_domain && wizardData.gatewy_type !== GatewayType.WAN && (
        <>
          <Alert icon={<DnsRounded />} severity="info">
            {t("dns_ns_record", { sn_host_base: SN_BASE_HOST })}
          </Alert>
          <Typography fontWeight={700}>{t("dns_txt_records_title")}</Typography>
        </>
      )}

      {requiresDnsTxt && (
        <Stack spacing={1.5}>
          <Stack direction={{ xs: "column", sm: "row" }} spacing={1.5} alignItems="center">
            <Button variant="outlined" onClick={buildDnsRecord} disabled={dnsLoading}>
              {dnsLoading ? t("generating_txt_records") : t("generate_txt_records_button")}
            </Button>
            <Alert icon={<WarningRounded />} severity="warning">
              {t("dns_config_txt_tips")}
            </Alert>
          </Stack>
          
          {dnsRecords.map((rec) => (
            <TextField
              key={rec.key}
              label={`${rec.key}`}
              value={`${rec.key}=${rec.value};`}
              InputProps={{ readOnly: true }}
              multiline
              minRows={2}
            />
          ))}
        </Stack>
      )}

      {!wizardData.is_wallet_runtime && (
        <Stack spacing={1}>
          <Typography fontWeight={700}>{t("owner_private_key")}</Typography>
          <TextField
            value={wizardData.owner_private_key as string}
            multiline
            minRows={4}
            InputProps={{
              readOnly: true,
              endAdornment: (
                <IconButton onClick={copyKey} aria-label="copy private key">
                  <ContentCopyRounded />
                </IconButton>
              ),
            }}
          />
          <Alert severity="warning">
            <Typography variant="body2">{t("private_key_warning1")}</Typography>
            <Typography variant="body2">{t("private_key_warning2")}</Typography>
            <Typography variant="body2">{t("private_key_warning3")}</Typography>
          </Alert>
        </Stack>
      )}

      {error && <Alert severity="error">{error}</Alert>}

      <Stack direction="row" justifyContent="space-between" spacing={1.5} flexWrap="wrap" alignItems="center">
        <Button variant="text" onClick={onBack}>
          {t("back_button")}
        </Button>
        <Button
          variant="contained"
          size="large"
          onClick={handleActivate}
          endIcon={!loading ? <LaunchRounded /> : undefined}
          disabled={loading || (requiresDnsTxt && !dnsReady)}
          sx={{ py: 1.15, minWidth: 160 }}
        >
          {loading ? <CircularProgress size={20} /> : t("activate_button")}
        </Button>
      </Stack>

    </Stack>
  );
};

export default ReviewStep;
