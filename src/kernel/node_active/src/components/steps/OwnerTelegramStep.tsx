import { Button, Link, Stack, TextField } from "@mui/material";
import { useTranslation } from "react-i18next";
import { WizardData } from "../../types";
import { TELEGRAM_ACCOUNT_ID_TUTORIAL_URL } from "../../../active_lib";
import { buckyos, RuntimeType } from "buckyos";
import { Lead, StepActions } from "../ui";

type Props = {
    wizardData: WizardData;
    onUpdate: (data: Partial<WizardData>) => void;
    onNext: () => void;
    onBack: () => void;
};

const OwnerTelegramStep = ({
    wizardData,
    onUpdate,
    onNext,
    onBack,
}: Props) => {
    const { t } = useTranslation();
    const accountId = wizardData.owner_telegram_account_id.trim();
    const isInvalid = accountId.length > 0 && !/^\d+$/.test(accountId);

    const openExternal = (url: string) => {
        if (!url) {
            return;
        }
        if (buckyos.getRuntimeType?.() === RuntimeType.AppRuntime) {
            buckyos.openExternalUrl?.(url);
            return;
        }
        window.open(url, "_blank", "noopener,noreferrer");
    };

    const handleNext = () => {
        onUpdate({ owner_telegram_account_id: accountId });
        onNext();
    };

    return (
        <Stack spacing={3}>
            <Lead>{t("owner_telegram_description")}</Lead>

            <TextField
                label={t("owner_telegram_account_id_label")}
                value={wizardData.owner_telegram_account_id}
                onChange={(event) =>
                    onUpdate({ owner_telegram_account_id: event.target.value })
                }
                error={isInvalid}
                helperText={
                    isInvalid ? (
                        t("error_owner_telegram_account_id_invalid")
                    ) : (
                        <Link
                            component="button"
                            type="button"
                            underline="hover"
                            onClick={() => openExternal(TELEGRAM_ACCOUNT_ID_TUTORIAL_URL)}
                            sx={{ color: "var(--cp-accent)", fontSize: "inherit", verticalAlign: "baseline" }}
                        >
                            {t("owner_telegram_account_id_tutorial_link")}
                        </Link>
                    )
                }
                autoComplete="off"
                inputProps={{ inputMode: "numeric", style: { fontFamily: "var(--cp-font-mono)" } }}
                sx={{ maxWidth: { sm: 360 } }}
            />

            <StepActions back={<Button variant="text" onClick={onBack}>{t("back_button")}</Button>}>
                <Button
                    variant="contained"
                    size="large"
                    onClick={handleNext}
                    disabled={isInvalid}
                    sx={{ minWidth: 120 }}
                >
                    {accountId ? t("next_button") : t("skip_button")}
                </Button>
            </StepActions>
        </Stack>
    );
};

export default OwnerTelegramStep;
