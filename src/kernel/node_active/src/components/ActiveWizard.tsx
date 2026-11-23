import { useEffect, useMemo, useRef, useState } from "react";
import { Alert, Box, CircularProgress, Stack, Typography } from "@mui/material";
import { useTranslation } from "react-i18next";
import { StepKey, WalletUser, WizardData, ActiveWizzardData } from "../types";
import { createInitialWizardData, type SignedActivation } from "../../active_lib";
import GatewayStep from "./steps/GatewayStep";
import DomainStep from "./steps/DomainStep";
import SecurityStep from "./steps/SecurityStep";
import ReviewStep, { type Activated } from "./steps/ReviewStep";
import ResultStep from "./steps/ResultStep";
import { useRegionProbe } from "./useRegionProbe";
import { mono } from "./ui";

export type UpdateWizard = (
  partial: Partial<WizardData> | ((previous: WizardData) => Partial<WizardData>),
) => void;

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
  const [activated, setActivated] = useState<Activated | null>(null);
  const region = useRegionProbe(!isWalletRuntime);
  const signed = useRef<SignedActivation | null>(null);
  const walletInfoReady =
    !isWalletRuntime ||
    (!!walletUser?.sn_username && !!walletUser?.owner_document);
  const stepOrder = useMemo<StepKey[]>(
    () =>
      isWalletRuntime
        ? ["gateway", "domain", "review", "result"]
        : ["gateway", "security", "domain", "review", "result"],
    [isWalletRuntime]
  );
  const visibleSteps = useMemo(
    () => stepOrder.filter((step) => step !== "result"),
    [stepOrder]
  );

  // 异步初始化 wizardData（必须等 walletUser 就绪，避免初始化路径错误）
  useEffect(() => {
    let cancelled = false;
    const init = async () => {
      try {
        setWizardData(null);
        setActiveStep(0);
        setActivated(null);

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

  const handleUpdate: UpdateWizard = (partial) =>
    setWizardData((prev) =>
      prev ? { ...prev, ...(typeof partial === "function" ? partial(prev) : partial) } : null,
    );

  const goNext = () => setActiveStep((prev) => Math.min(prev + 1, stepOrder.length - 1));
  const goBack = () => setActiveStep((prev) => Math.max(prev - 1, 0));

  const handleActivationDone = (result: Activated) => {
    setActivated(result);
    setActiveStep(stepOrder.indexOf("result"));
  };

  const currentStepKey = stepOrder[activeStep];

  const renderStep = () => {
    if (isWalletRuntime && !walletUser) {
      return <LoadingLine label={t("loading")} />;
    }
    if (isWalletRuntime && !walletInfoReady) {
      return (
        <Alert severity="error">
          {t("wallet_info_incomplete")}
        </Alert>
      );
    }
    if (!wizardData) {
      return <LoadingLine label={t("loading")} />;
    }
    
    switch (currentStepKey) {
      case "gateway":
        return (
          <GatewayStep
            wizardData={wizardData}
            onUpdate={handleUpdate}
            onNext={goNext}
            region={region}
          />
        );
      case "security":
        return (
          <SecurityStep
            wizardData={wizardData}
            onUpdate={handleUpdate}
            onNext={goNext}
            onBack={goBack}
            region={region}
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
      case "review":
        return (
          <ReviewStep
            wizardData={wizardData}
            onUpdate={handleUpdate}
            onBack={goBack}
            onActivated={handleActivationDone}
            isWalletRuntime={isWalletRuntime}
            signed={signed}
          />
        );
      case "result":
        return activated ? <ResultStep wizardData={wizardData} activated={activated} /> : null;
      default:
        return null;
    }
  };

  const totalSteps = visibleSteps.length;
  const isResult = currentStepKey === "result";
  const stepNumber = isResult ? totalSteps : activeStep + 1;

  return (
    <Box>
      {!isResult && (
        <Box sx={{ mb: 4 }}>
          <Stack direction="row" alignItems="baseline" justifyContent="space-between" spacing={2} sx={{ mb: 1.5 }}>
            <Typography variant="h5" component="h2">
              {t(`title_step_${currentStepKey}`)}
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
