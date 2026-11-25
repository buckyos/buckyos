import {
  Alert,
  Box,
  Button,
  Checkbox,
  CircularProgress,
  FormControlLabel,
  MenuItem,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import { MutableRefObject, ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  activateNode,
  deriveActiveNames,
  deriveDomainSetup,
  deriveGatewayTopology,
  type SignedActivation,
} from "../../../active_lib";
import { activeErrorOf, certificateUntrusted, checkItemsOf } from "../../domain";
import { errorText, type ErrorText } from "../../errors";
import { CheckItem, GatewayType, WizardData } from "../../types";
import type { UpdateWizard } from "../ActiveWizard";
import { UntrustedImpact } from "../domain/CertificatePanel";
import CheckList from "../domain/CheckList";
import { ErrorAlert, Lead, StatusLine, StepActions, mono } from "../ui";

export type Activated = { accessHostname: string; zoneDid: string; activatedAt: number };

type Props = {
  wizardData: WizardData;
  onUpdate: UpdateWizard;
  onActivated: (activated: Activated) => void;
  onBack: () => void;
  isWalletRuntime: boolean;
  /** Survives going back to another step, so a wallet is not asked to sign the same documents again. */
  signed: MutableRefObject<SignedActivation | null>;
};

const ReviewStep = ({ wizardData, onUpdate, onActivated, onBack, isWalletRuntime, signed }: Props) => {
  const { t } = useTranslation();
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<ErrorText | null>(null);
  const [failedItems, setFailedItems] = useState<CheckItem[]>([]);
  const names = wizardData.owner_document ? deriveActiveNames(wizardData) : null;
  const topology = deriveGatewayTopology(wizardData);
  const setup = deriveDomainSetup(wizardData);

  const activate = async () => {
    setError(null);
    setFailedItems([]);
    setLoading(true);
    try {
      const result = await activateNode(wizardData, signed);
      onUpdate({
        sn_access_token: null,
        sn_access_token_at: null,
        sn_refresh_token: null,
        admin_password_hash: "",
        device_private_key: "",
        web_owner_material: null,
      });
      onActivated({ ...result, activatedAt: Date.now() });
    } catch (cause) {
      setFailedItems(
        checkItemsOf(activeErrorOf(cause)?.detail ?? "").filter(
          (item) => item.status === "fail" || item.status === "waiting",
        ),
      );
      setError(errorText(t, cause));
    } finally {
      setLoading(false);
    }
  };

  if (!names || !wizardData.owner_document) {
    return <Alert severity="error">{t("wallet_info_incomplete")}</Alert>;
  }

  const gatewayTitle =
    wizardData.gatewy_type === GatewayType.WAN
      ? t("gateway_public_ip_title")
      : wizardData.gatewy_type === GatewayType.PortForward
        ? t("gateway_direct_title")
        : t("gateway_sn_relay_title");
  const gatewayDetail =
    topology.net_id === "portmap"
      ? `RTCP ${topology.rtcp_port}`
      : topology.public_ips.length > 0
        ? topology.public_ips.join(", ")
        : "";

  // Identifiers (DIDs, host names) are set in mono; words stay in the interface font.
  const rows: Array<{ label: ReactNode; value: ReactNode; detail?: ReactNode; mono?: boolean }> = [
    { label: t("review_owner_did"), value: names.owner_did, mono: true },
    { label: t("review_zone_did"), value: names.zone_did, mono: true },
    { label: t("access_domain"), value: names.access_hostname, mono: true },
    ...(setup
      ? [
          { label: t("domain_mode_title"), value: t(`mode_${setup.mode}`) },
          { label: t("cert_source_title"), value: t(`cert_source_${setup.certificate}`) },
        ]
      : []),
    { label: t("review_gateway"), value: gatewayTitle, detail: gatewayDetail },
    ...(wizardData.selected_region
      ? [
          {
            label: t("region_probe_title"),
            value: t(`region_name_${wizardData.selected_region}`, { defaultValue: wizardData.selected_region }),
          },
        ]
      : []),
    { label: t("review_bns_name"), value: names.bns_publish_name, mono: true },
  ];

  return (
    <Stack spacing={3}>
      <Lead>{t(isWalletRuntime ? "wallet_signing_review" : "web_signing_review")}</Lead>
      <Box component="dl" sx={{ m: 0, borderRadius: "12px", border: "1px solid var(--cp-border)", overflow: "hidden" }}>
        {rows.map((row, index) => (
          <Box
            key={index}
            sx={{
              display: "grid",
              gridTemplateColumns: { xs: "1fr", sm: "180px minmax(0, 1fr)" },
              alignItems: "baseline",
              columnGap: 2,
              rowGap: 0.5,
              px: 2,
              py: 1.5,
              borderTop: index === 0 ? "none" : "1px solid var(--cp-border)",
            }}
          >
            <Typography component="dt" variant="body2" color="text.secondary">
              {row.label}
            </Typography>
            <Box component="dd" sx={{ m: 0, minWidth: 0 }}>
              <Typography
                variant="body2"
                sx={{ ...(row.mono ? { ...mono, fontSize: 13 } : {}), overflowWrap: "anywhere" }}
              >
                {row.value}
              </Typography>
              {row.detail ? (
                <Typography variant="caption" color="text.secondary" sx={{ ...mono, display: "block" }}>
                  {row.detail}
                </Typography>
              ) : null}
            </Box>
          </Box>
        ))}
      </Box>
      {certificateUntrusted(wizardData) && (
        <Alert severity="warning">
          {t("cert_untrusted_title")}
          <UntrustedImpact />
        </Alert>
      )}
      {wizardData.host_accounts.length > 0 && (
        <Stack spacing={1.5}>
          <TextField
            select
            label={t("host_account_label")}
            helperText={t("host_account_hint")}
            value={wizardData.host_account}
            disabled={loading}
            onChange={(event) => onUpdate({ host_account: event.target.value })}
            SelectProps={{ displayEmpty: true }}
            InputLabelProps={{ shrink: true }}
          >
            <MenuItem value="">{t("host_account_none")}</MenuItem>
            {wizardData.host_accounts.map((account) => (
              <MenuItem key={account} value={account} sx={{ fontFamily: "var(--cp-font-mono)", fontSize: 13 }}>
                {account}
              </MenuItem>
            ))}
          </TextField>
          {wizardData.host_account === "root" && <Alert severity="warning">{t("host_account_root_warning")}</Alert>}
        </Stack>
      )}
      {!isWalletRuntime && wizardData.web_owner_material && (
        <Stack spacing={1.5}>
          <FormControlLabel
            control={
              <Checkbox
                size="small"
                checked={wizardData.save_owner_mnemonic}
                disabled={loading}
                onChange={(event) => onUpdate({ save_owner_mnemonic: event.target.checked })}
              />
            }
            label={<Typography variant="body2">{t("save_mnemonic_label")}</Typography>}
          />
          {wizardData.save_owner_mnemonic && <Alert severity="warning">{t("save_mnemonic_warning")}</Alert>}
        </Stack>
      )}
      {loading && (
        <StatusLine tone="accent" live>
          {t(setup?.certificate === "acme" ? "activating_hint_challenge" : "activating_hint")}
        </StatusLine>
      )}
      {error && <ErrorAlert {...error} />}
      {failedItems.length > 0 && <CheckList items={failedItems} />}
      <StepActions
        back={
          <Button variant="text" onClick={onBack} disabled={loading}>
            {t("back_button")}
          </Button>
        }
      >
        <Button
          variant="contained"
          size="large"
          onClick={activate}
          disabled={loading}
          startIcon={loading ? <CircularProgress size={16} thickness={5} color="inherit" /> : undefined}
          sx={{ minWidth: 120 }}
        >
          {loading ? t("activating") : t("activate_button")}
        </Button>
      </StepActions>
    </Stack>
  );
};

export default ReviewStep;
