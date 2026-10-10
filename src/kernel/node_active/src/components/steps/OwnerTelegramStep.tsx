import { Alert, Button, Link, Stack, TextField } from "@mui/material";
import { SendRounded } from "@mui/icons-material";
import { useTranslation } from "react-i18next";
import { WizardData } from "../../types";
import { TELEGRAM_ACCOUNT_ID_TUTORIAL_URL } from "../../../active_lib";
import { buckyos, RuntimeType } from "buckyos";

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
            <Alert icon={<SendRounded />} severity="info">
                {t("owner_telegram_description")}
            </Alert>

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
                        >
                            {t("owner_telegram_account_id_tutorial_link")}
                        </Link>
                    )
                }
                inputProps={{ inputMode: "numeric" }}
                fullWidth
            />

            <Stack
                direction="row"
                justifyContent="space-between"
                spacing={1.5}
                flexWrap="wrap"
                alignItems="center"
            >
                <Button variant="text" onClick={onBack}>
                    {t("back_button")}
                </Button>
                <Button
                    variant="contained"
                    onClick={handleNext}
                    disabled={isInvalid}
                    sx={{ py: 1.15, minWidth: 160 }}
                >
                    {accountId ? t("next_button") : t("skip_button")}
                </Button>
            </Stack>
        </Stack>
    );
};

export default OwnerTelegramStep;
