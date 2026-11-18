import { useEffect, useMemo, useState } from "react";
import { Alert, Box, CircularProgress, Stack, Typography } from "@mui/material";
import { useTranslation } from "react-i18next";
import { StepKey, WalletUser, WizardData, ActiveWizzardData } from "../types";
import { createInitialWizardData, WEB3_BASE_HOST } from "../../active_lib";
import GatewayStep from "./steps/GatewayStep";
import DomainStep from "./steps/DomainStep";
import SecurityStep from "./steps/SecurityStep";
import AIProviderStep from "./steps/AIProviderStep";
import OwnerTelegramStep from "./steps/OwnerTelegramStep";
import ReviewStep from "./steps/ReviewStep";
import SuccessStep from "./steps/SuccessStep";
import { mono } from "./ui";

type Props = {
  isWalletRuntime: boolean;
  walletUser?: WalletUser;
};

const LoadingLine = ({ label }: { label: string }) => (
  <Stack direction="row" spacing={1.5} alignItems="center" sx={{ py: 2 }}>
    <CircularProgress size={16} thickness={5} />
    <Typography variant="body2" color="text.secondary">
      {label}
    </Typography>
  </Stack>
);

const ActiveWizard = ({ isWalletRuntime, walletUser }: Props) => {
  const { t } = useTranslation();
  const [wizardData, setWizardData] = useState<ActiveWizzardData | null>(null);
  const [activeStep, setActiveStep] = useState(0);
  const [completedUrl, setCompletedUrl] = useState("");
  const walletInfoReady =
    !isWalletRuntime ||
    (!!walletUser?.sn_username && !!walletUser?.owner_document);
  const stepOrder = useMemo<StepKey[]>(
    () =>
      isWalletRuntime
        ? ["gateway", "domain", "ai_provider", "owner_telegram", "review", "success"]
        : ["security", "gateway", "domain", "ai_provider", "owner_telegram", "review", "success"],
    [isWalletRuntime]
  );
  const visibleSteps = useMemo(
    () => stepOrder.filter((step) => step !== "success"),
    [stepOrder]
  );

  // 异步初始化 wizardData（必须等 walletUser 就绪，避免初始化路径错误）
  useEffect(() => {
    let cancelled = false;
    const init = async () => {
      try {
        setWizardData(null);
        setActiveStep(0);
        setCompletedUrl("");

        if (isWalletRuntime) {
          // Wallet runtime requires wallet user.
          if (!walletUser) {
            return;
          }
          const data = await createInitialWizardData({
            is_wallet_runtime: true,
            owner_document: walletUser.owner_document,
            evm_address: walletUser.owner_document.wallets?.main?.address || "",
            sn_user_name: (walletUser.sn_username || "").toLowerCase(),
            sn_active_code: null,
            enabled_features: {
              llm_router: true,
            },
          });
          if (!cancelled) setWizardData(data);
          return;
        }

        const data = await createInitialWizardData();
        if (!cancelled) setWizardData(data);
      } catch (err) {
        console.warn("Failed to init wizard data", err);
        if (!cancelled) setWizardData(null);
      }
    };

    init();
    return () => {
      cancelled = true;
    };
  }, [isWalletRuntime, walletUser]);

  const stepTitles = useMemo(
    () => ({
      security: t("title_step_1"),
      gateway: t("title_step_2"),
      domain: t("title_step_3"),
      ai_provider: t("title_step_4"),
      owner_telegram: t("title_step_5"),
      review: t("title_step_6"),
      success: t("activation_success"),
    }),
    [t]
  );

  const handleUpdate = (partial: Partial<WizardData>) =>
    setWizardData((prev) => {
      if (!prev) return null;
      const invalidatesDocuments = [
        "gatewy_type",
        "port_mapping_mode",
        "rtcp_port",
        "use_self_domain",
        "self_domain",
        "owner_document",
        "device_public_key",
      ].some((key) => key in partial);
      return {
        ...prev,
        ...partial,
        ...(invalidatesDocuments && !("prepared_documents" in partial)
          ? { prepared_documents: null, signed_documents: null }
          : {}),
      };
    });

  const goNext = () => setActiveStep((prev) => Math.min(prev + 1, stepOrder.length - 1));
  const goBack = () => setActiveStep((prev) => Math.max(prev - 1, 0));

  const handleActivationDone = (url: string) => {
    setCompletedUrl(url);
    setActiveStep(stepOrder.indexOf("success"));
  };

  const currentStepKey = stepOrder[activeStep];
  const fallbackUrl = wizardData?.use_self_domain
    ? wizardData.self_domain
      ? `https://${wizardData.self_domain}`
      : ""
    : wizardData?.sn_user_name 
    ? `https://${wizardData.sn_user_name}.${WEB3_BASE_HOST}`
    : "";
  const successUrl = completedUrl || fallbackUrl;

  const renderStep = () => {
    if (isWalletRuntime && !walletUser) {
      return <LoadingLine label={t("loading")} />;
    }
    if (isWalletRuntime && !walletInfoReady) {
      return (
        <Alert severity="error">
          {t("wallet_info_incomplete", {
            defaultValue: "Wallet user info is incomplete. Missing SN username or owner key.",
          })}
        </Alert>
      );
    }
    if (!wizardData) {
      return <LoadingLine label={t("loading")} />;
    }
    
    switch (currentStepKey) {
      case "security":
        return (
          <SecurityStep
            wizardData={wizardData}
            onUpdate={handleUpdate}
            onNext={goNext}
          />
        );
      case "gateway":
        return (
          <GatewayStep
            wizardData={wizardData}
            onUpdate={handleUpdate}
            onNext={goNext}
            isWalletRuntime={isWalletRuntime}
          />
        );
      case "domain":
        return (
          <DomainStep
            wizardData={wizardData}
            onUpdate={handleUpdate}
            onNext={goNext}
            onBack={goBack}
          />
        );
      case "ai_provider":
        return (
          <AIProviderStep
            wizardData={wizardData}
            onUpdate={handleUpdate}
            onNext={goNext}
            onBack={goBack}
          />
        );
      case "owner_telegram":
        return (
          <OwnerTelegramStep
            wizardData={wizardData}
            onUpdate={handleUpdate}
            onNext={goNext}
            onBack={goBack}
          />
        );
      case "review":
        return (
          <ReviewStep
            wizardData={wizardData}
            onUpdate={handleUpdate}
            onBack={goBack}
            onActivated={handleActivationDone}
            isWalletRuntime={isWalletRuntime}
          />
        );
      case "success":
        return <SuccessStep wizardData={wizardData} targetUrl={successUrl} />;
      default:
        return null;
    }
  };

  const totalSteps = visibleSteps.length;
  const isSuccess = currentStepKey === "success";
  const stepNumber = isSuccess ? totalSteps : activeStep + 1;
  // Titles in the language files carry a number ("2. …"); the counter beside them already says where we are,
  // and in the wallet runtime (no security step) the built-in numbers would be off by one.
  const displayStepTitle = stepTitles[currentStepKey].replace(/^\d+\.\s*/, "");

  return (
    <Box>
      {!isSuccess && (
        <Box sx={{ mb: 4 }}>
          <Stack direction="row" alignItems="baseline" justifyContent="space-between" spacing={2} sx={{ mb: 1.5 }}>
            <Typography variant="h5" component="h2">
              {displayStepTitle}
            </Typography>
            <Typography
              variant="body2"
              color="text.secondary"
              sx={{ flexShrink: 0, ...mono, fontSize: 13 }}
              aria-label={`${stepNumber} / ${totalSteps}`}
            >
              {stepNumber}/{totalSteps}
            </Typography>
          </Stack>
          <Box sx={{ display: "flex", gap: 0.5 }} aria-hidden="true">
            {visibleSteps.map((step, index) => (
              <Box
                key={step}
                sx={{
                  flex: 1,
                  height: 4,
                  borderRadius: 2,
                  bgcolor: index < stepNumber ? "var(--cp-accent)" : "var(--cp-border)",
                  transition: "background-color 240ms var(--cp-ease-smooth)",
                }}
              />
            ))}
          </Box>
        </Box>
      )}
      {renderStep()}
    </Box>
  );
};

export default ActiveWizard;
