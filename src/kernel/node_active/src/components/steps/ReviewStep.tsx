import { Download } from "lucide-react";
import { Alert, Box, Button, CircularProgress, MenuItem, Stack, TextField, Typography } from "@mui/material";
import { ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { activateNode, customDomainPublication, deriveActiveNames, deriveGatewayTopology, prepareSignedActivation } from "../../../active_lib";
import { GatewayType, WizardData } from "../../types";
import { Lead, Note, StepActions, mono, plainLabel } from "../ui";

type Props = {
  wizardData: WizardData;
  onUpdate: (data: Partial<WizardData>) => void;
  onActivated: (targetUrl: string) => void;
  onBack: () => void;
  isWalletRuntime: boolean;
};

const ReviewStep = ({ wizardData, onUpdate, onActivated, onBack, isWalletRuntime }: Props) => {
  const { t } = useTranslation();
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const names = wizardData.owner_document ? deriveActiveNames(wizardData) : null;
  const topology = deriveGatewayTopology(wizardData);

  const preparePublication = async () => {
    setError("");
    setLoading(true);
    try {
      const result = await prepareSignedActivation(wizardData);
      onUpdate({
        prepared_documents: result.prepared,
        signed_documents: result.signed,
        admin_password_hash: result.adminPasswordHash,
      });
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setLoading(false);
    }
  };

  const downloadPublication = () => {
    try {
      const publication = customDomainPublication(wizardData);
      const url = URL.createObjectURL(new Blob([publication.content], { type: "application/jwt" }));
      const link = document.createElement("a");
      link.href = url;
      link.download = "did.json";
      link.click();
      setTimeout(() => URL.revokeObjectURL(url), 1000);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  };

  const activate = async () => {
    setError("");
    setLoading(true);
    try {
      const result = await activateNode(wizardData);
      onUpdate({
        prepared_documents: result.prepared,
        signed_documents: result.signed,
        sn_access_token: null,
        sn_refresh_token: null,
        admin_password_hash: "",
        device_private_key: "",
        web_owner_material: null,
      });
      onActivated(`https://${result.accessHostname}`);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setLoading(false);
    }
  };

  if (!names || !wizardData.owner_document) {
    return <Alert severity="error">{t("wallet_info_incomplete", "OwnerDocument is missing")}</Alert>;
  }

  const gatewayTitle =
    wizardData.gatewy_type === GatewayType.WAN
      ? t("gateway_public_ip_title", "Fixed public IP")
      : wizardData.gatewy_type === GatewayType.PortForward
        ? t("gateway_direct_title", "Direct connection")
        : t("gateway_sn_relay_title", "SN relay");
  const gatewayDetail =
    topology.net_id === "portmap" ? `${topology.net_id} · RTCP ${topology.rtcp_port}` : topology.net_id;

  // Identifiers (DIDs, host names, region ids) are set in mono; words stay in the interface font.
  const rows: Array<{ label: ReactNode; value: ReactNode; detail?: ReactNode; mono?: boolean }> = [
    { label: t("review_owner_did", "Owner DID"), value: names.owner_did, mono: true },
    { label: t("review_zone_did", "Zone DID"), value: names.zone_did, mono: true },
    { label: plainLabel(t("access_domain", "Access domain")), value: names.access_hostname, mono: true },
    { label: t("review_gateway", "Access route"), value: gatewayTitle, detail: gatewayDetail },
    ...(!isWalletRuntime
      ? [
          {
            label: t("region_probe_title", "Network Region"),
            value: wizardData.selected_region || t("region_server_fallback", "Automatic server fallback"),
            mono: Boolean(wizardData.selected_region),
          },
        ]
      : []),
    { label: t("review_bns_name", "BNS name"), value: names.bns_publish_name, mono: true },
  ];

  return (
    <Stack spacing={3}>
      <Lead>
        {isWalletRuntime
          ? t(
              "wallet_signing_review",
              "The wallet will ask twice: first for Boot, Device and DeviceMini, then for ZoneDocument.",
            )
          : t(
              "web_signing_review",
              "The recovery phrase will be used once in local memory to sign four activation documents.",
            )}
      </Lead>
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
      {wizardData.host_accounts.length > 0 && (
        <Stack spacing={1.5}>
          <TextField
            select
            label={t("host_account_label", "Host account for your Agents")}
            helperText={t(
              "host_account_hint",
              "Agents you enable for the host run commands on this machine as this account. Other Agents are not affected. A normal account cannot use sudo from an Agent.",
            )}
            value={wizardData.host_account}
            disabled={loading}
            onChange={(event) => onUpdate({ host_account: event.target.value })}
            SelectProps={{ displayEmpty: true }}
            InputLabelProps={{ shrink: true }}
          >
            <MenuItem value="">{t("host_account_none", "None (Agents cannot run commands on this machine)")}</MenuItem>
            {wizardData.host_accounts.map((account) => (
              <MenuItem key={account} value={account} sx={{ fontFamily: "var(--cp-font-mono)", fontSize: 13 }}>
                {account}
              </MenuItem>
            ))}
          </TextField>
          {wizardData.host_account === "root" && (
            <Alert severity="warning">
              {t(
                "host_account_root_warning",
                "Agents you enable for the host will have full control of this machine, and anyone who takes over your owner account gets the same control. Choose root only to let an Agent maintain this machine.",
              )}
            </Alert>
          )}
        </Stack>
      )}
      {wizardData.use_self_domain && (
        <Note>
          <Stack spacing={1.5}>
            <Box component="span" sx={{ color: "text.primary" }}>
              {t("device_publication_intro", "Before activation, host the signed device identity document at the HTTPS address below. This address must remain available while this node is offline; it cannot depend on this node's relay tunnel.")}
            </Box>
            <Box
              component="span"
              sx={{ ...mono, fontSize: 13, color: "text.primary", overflowWrap: "anywhere", userSelect: "all" }}
            >
              {`https://ood1.${names.access_hostname}/.well-known/did.json`}
            </Box>
            <Box component="span">
              {t("device_publication_content", "Publish this HTTPS document path through an independent host or proxy, and serve the file unchanged as application/jwt. Preserve RTCP routing for the device hostname and port. Activation will verify the exact signed document.")}
            </Box>
            <Stack direction={{ xs: "column", sm: "row" }} spacing={1}>
              <Button variant="outlined" onClick={preparePublication} disabled={loading || Boolean(wizardData.signed_documents)}>
                {t("prepare_device_publication", "Prepare signed document")}
              </Button>
              <Button
                variant="outlined"
                onClick={downloadPublication}
                disabled={loading || !wizardData.signed_documents}
                startIcon={<Download size={16} strokeWidth={1.75} />}
              >
                {t("download_device_publication", "Download did.json")}
              </Button>
            </Stack>
          </Stack>
        </Note>
      )}
      {error && <Alert severity="error">{error}</Alert>}
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
          disabled={loading || (wizardData.use_self_domain && !wizardData.signed_documents)}
          startIcon={loading ? <CircularProgress size={16} thickness={5} color="inherit" /> : undefined}
          sx={{ minWidth: 120 }}
        >
          {loading
            ? t("activating", "Activating…")
            : wizardData.use_self_domain
              ? t("verify_publication_and_activate", "Verify publication and activate")
              : t("activate_button", "Activate")}
        </Button>
      </StepActions>
    </Stack>
  );
};

export default ReviewStep;
