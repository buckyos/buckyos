import { CheckCircleRounded } from "@mui/icons-material";
import { Alert, Button, CircularProgress, Divider, Stack, Typography } from "@mui/material";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { activateNode, deriveActiveNames, deriveGatewayTopology } from "../../../active_lib";
import { WizardData } from "../../types";

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

  return (
    <Stack spacing={3}>
      <Alert severity="info">
        {isWalletRuntime
          ? t(
              "wallet_signing_review",
              "The wallet will ask twice: first for Boot, Device and DeviceMini, then for ZoneDocument.",
            )
          : t(
              "web_signing_review",
              "The recovery phrase will be used once in local memory to sign four activation documents.",
            )}
      </Alert>
      <Stack spacing={1.25} divider={<Divider flexItem />}>
        <Stack direction="row" justifyContent="space-between" gap={2}>
          <Typography color="text.secondary">Owner DID</Typography>
          <Typography sx={{ wordBreak: "break-all" }}>{names.owner_did}</Typography>
        </Stack>
        <Stack direction="row" justifyContent="space-between" gap={2}>
          <Typography color="text.secondary">Zone DID</Typography>
          <Typography sx={{ wordBreak: "break-all" }}>{names.zone_did}</Typography>
        </Stack>
        <Stack direction="row" justifyContent="space-between" gap={2}>
          <Typography color="text.secondary">{t("domain_placeholder")}</Typography>
          <Typography sx={{ wordBreak: "break-all" }}>{names.access_hostname}</Typography>
        </Stack>
        <Stack direction="row" justifyContent="space-between" gap={2}>
          <Typography color="text.secondary">Gateway</Typography>
          <Typography>{topology.net_id}</Typography>
        </Stack>
        {!isWalletRuntime && (
          <Stack direction="row" justifyContent="space-between" gap={2}>
            <Typography color="text.secondary">{t("region_probe_title", "Network Region")}</Typography>
            <Typography>
              {wizardData.selected_region ||
                t("region_server_fallback", "Automatic server fallback")}
            </Typography>
          </Stack>
        )}
        <Stack direction="row" justifyContent="space-between" gap={2}>
          <Typography color="text.secondary">BNS publish name</Typography>
          <Typography>{names.bns_publish_name}</Typography>
        </Stack>
      </Stack>
      {error && <Alert severity="error">{error}</Alert>}
      <Stack direction="row" justifyContent="space-between" spacing={2}>
        <Button onClick={onBack} disabled={loading}>
          {t("back_button")}
        </Button>
        <Button
          variant="contained"
          size="large"
          onClick={activate}
          disabled={loading}
          startIcon={loading ? <CircularProgress size={18} /> : <CheckCircleRounded />}
        >
          {loading ? t("activating", "Activating…") : t("activate_button", "Activate")}
        </Button>
      </Stack>
    </Stack>
  );
};

export default ReviewStep;
