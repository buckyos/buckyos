import { useEffect, useMemo, useState } from "react";
import { Box, LinearProgress, Stack, Typography } from "@mui/material";
import { useTranslation } from "react-i18next";
import { StepKey, WalletUser, WizardData, ActiveWizzardData } from "../types";
import { createInitialWizardData, WEB3_BASE_HOST } from "../../active_lib";
import GatewayStep from "./steps/GatewayStep";
import DomainStep from "./steps/DomainStep";
import SecurityStep from "./steps/SecurityStep";
import ReviewStep from "./steps/ReviewStep";
import SuccessStep from "./steps/SuccessStep";

const stepOrder: StepKey[] = ["gateway", "domain", "security", "review", "success"];
const visibleSteps: StepKey[] = ["gateway", "domain", "security", "review"];

type Props = {
  isWalletRuntime: boolean;
  walletUser?: WalletUser;
};

const ActiveWizard = ({ isWalletRuntime, walletUser }: Props) => {
  const { t } = useTranslation();
  const [wizardData, setWizardData] = useState<ActiveWizzardData | null>(null);
  const [activeStep, setActiveStep] = useState(0);
  const [completedUrl, setCompletedUrl] = useState("");

  // 异步初始化 wizardData
  useEffect(() => {
    if(isWalletRuntime) {
      createInitialWizardData({
        is_wallet_runtime: isWalletRuntime,
        owner_user_name: walletUser?.user_name.toLowerCase(),
        owner_public_key: walletUser?.public_key,
        sn_user_name: walletUser?.sn_username?.toLowerCase() || "",
      }).then((data) => {
        setWizardData(data);
      })
    } else {
      createInitialWizardData().then((data) => {
        setWizardData(data);
      });
    }
  }, []); // 只在组件挂载时执行一次

  // 更新 wallet 相关信息
  useEffect(() => {
    if (!wizardData) return;
    
    if (!isWalletRuntime || !walletUser) {
      setWizardData((prev) => prev ? { ...prev, is_wallet_runtime: false } : null);
      return;
    }
    setWizardData((prev) => prev ? {
      ...prev,
      is_wallet_runtime: true,
      owner_user_name: walletUser.user_name?.toLowerCase() || "",
      owner_public_key: walletUser.public_key || {},
      sn_user_name: walletUser.sn_username?.toLowerCase() || "",
    } : null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isWalletRuntime, walletUser]);

  const stepTitles = useMemo(
    () => ({
      gateway: t("title_step_1"),
      domain: t("title_step_2"),
      security: t("title_step_3"),
      review: t("title_step_4"),
      success: t("activation_success"),
    }),
    [t]
  );

  const handleUpdate = (partial: Partial<WizardData>) => setWizardData((prev) => prev ? ({ ...prev, ...partial }) : null);

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
    if (!wizardData) {
      return <Box>Loading...</Box>;
    }
    
    switch (currentStepKey) {
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
            isWalletRuntime={isWalletRuntime}
            walletUser={walletUser}
          />
        );
      case "security":
        return <SecurityStep wizardData={wizardData} onUpdate={handleUpdate} onNext={goNext} onBack={goBack} />;
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
  const stepNumber = currentStepKey === "success" ? totalSteps : activeStep + 1;
  const progress =
    currentStepKey === "success"
      ? 100
      : Math.min((stepNumber / totalSteps) * 100, 100);

  return (
    <Box>
      <Stack spacing={1} mb={2}>
        <Stack direction="row" alignItems="center" justifyContent="space-between">
          <Typography variant="h6" fontWeight={700}>
            {stepTitles[currentStepKey]}
          </Typography>
          <Typography variant="body2" color="text.secondary">
            {stepNumber}/{totalSteps}
          </Typography>
        </Stack>
        <Box sx={{ position: "relative" }}>
          <LinearProgress
            variant="determinate"
            value={progress}
            sx={{
              height: 8,
              borderRadius: 999,
              backgroundColor: "action.hover",
            }}
          />
          <Box
            sx={{
              position: "absolute",
              top: "50%",
              left: `${progress}%`,
              transform: "translate(-50%, -50%)",
              width: 14,
              height: 14,
              borderRadius: "50%",
              bgcolor: "primary.main",
              boxShadow: "0 0 0 3px rgba(79,70,229,0.18)",
            }}
          />
        </Box>
      </Stack>
      {renderStep()}
    </Box>
  );
};

export default ActiveWizard;
