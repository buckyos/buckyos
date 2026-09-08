import { CheckCircleRounded, DownloadRounded } from "@mui/icons-material";
import { Alert, Button, CircularProgress, Divider, Stack, Typography } from "@mui/material";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { activateNode, customDomainPublication, deriveActiveNames, deriveGatewayTopology, prepareSignedActivation } from "../../../active_lib";
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
      {wizardData.use_self_domain && (
        <Alert severity="info">
          <Stack spacing={1.5}>
            <Typography>
              {t("device_publication_intro", "Before activation, host the signed device identity document at the HTTPS address below. This address must remain available while this node is offline; it cannot depend on this node's relay tunnel.")}
            </Typography>
            <Typography sx={{ wordBreak: "break-all" }}>
              {`https://ood1.${names.access_hostname}/.well-known/did.json`}
            </Typography>
            <Typography variant="body2">
              {t("device_publication_content", "Publish this HTTPS document path through an independent host or proxy, and serve the file unchanged as application/jwt. Preserve RTCP routing for the device hostname and port. Activation will verify the exact signed document.")}
            </Typography>
            <Stack direction="row" spacing={2}>
              <Button onClick={preparePublication} disabled={loading || Boolean(wizardData.signed_documents)}>
                {t("prepare_device_publication", "Prepare signed document")}
              </Button>
              <Button onClick={downloadPublication} disabled={loading || !wizardData.signed_documents} startIcon={<DownloadRounded />}>
                {t("download_device_publication", "Download did.json")}
              </Button>
            </Stack>
          </Stack>
        </Alert>
      )}
      {error && <Alert severity="error">{error}</Alert>}
      <Stack direction="row" justifyContent="space-between" spacing={2}>
        <Button onClick={onBack} disabled={loading}>
          {t("back_button")}
        </Button>
        <Button
          variant="contained"
          size="large"
          onClick={activate}
          disabled={loading || (wizardData.use_self_domain && !wizardData.signed_documents)}
          startIcon={loading ? <CircularProgress size={18} /> : <CheckCircleRounded />}
        >
          {loading
            ? t("activating", "Activating…")
            : wizardData.use_self_domain
              ? t("verify_publication_and_activate", "Verify publication and activate")
              : t("activate_button", "Activate")}
        </Button>
      </Stack>
    </Stack>
  );
};

export default ReviewStep;
