import {
    Box,
    Button,
    Dialog,
    DialogActions,
    DialogContent,
    DialogTitle,
    IconButton,
    InputAdornment,
    Link,
    Stack,
    TextField,
    Typography,
} from "@mui/material";
import { ExternalLink, Eye, EyeOff } from "lucide-react";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { AIProviderConfig, WizardData } from "../../types";
import { AI_PROVIDER_TUTORIAL_URL } from "../../../active_lib";
import { buckyos, RuntimeType } from "buckyos";
import { Lead, StatusLine, StepActions } from "../ui";

type Props = {
    wizardData: WizardData;
    onUpdate: (data: Partial<WizardData>) => void;
    onNext: () => void;
    onBack: () => void;
};

type ProviderKey = keyof AIProviderConfig;

const providers: Array<{ key: ProviderKey; title: string }> = [
    { key: "openai_api_token", title: "OpenAI" },
    { key: "claude_api_token", title: "Claude" },
    { key: "google_api_token", title: "Google" },
    { key: "glm_api_token", title: "GLM" },
];

const AIProviderStep = ({ wizardData, onUpdate, onNext, onBack }: Props) => {
    const { t } = useTranslation();
    const [visibleTokens, setVisibleTokens] = useState<Record<ProviderKey, boolean>>({
        openai_api_token: false,
        claude_api_token: false,
        google_api_token: false,
        openrouter_api_token: false,
        glm_api_token: false,
    });
    const [skipConfirmOpen, setSkipConfirmOpen] = useState(false);

    const aiProviderConfig = wizardData.ai_provider_config;
    const hasAnyProviderToken = useMemo(
        () =>
            Object.values(aiProviderConfig).some(
                (value) => typeof value === "string" && value.trim().length > 0,
            ),
        [aiProviderConfig],
    );
    const hasLlmRouterEnabled = wizardData.enabled_features.llm_router;
    const primaryLabel =
        hasAnyProviderToken || hasLlmRouterEnabled ? t("next_button") : t("skip_button");

    const normalizeProviderConfig = (
        config: AIProviderConfig,
    ): AIProviderConfig => ({
        openai_api_token: config.openai_api_token.trim(),
        claude_api_token: config.claude_api_token.trim(),
        google_api_token: config.google_api_token.trim(),
        openrouter_api_token: config.openrouter_api_token.trim(),
        glm_api_token: config.glm_api_token.trim(),
    });

    const updateProviderField = (field: ProviderKey, value: string) => {
        onUpdate({
            ai_provider_config: {
                ...wizardData.ai_provider_config,
                [field]: value,
            },
        });
    };

    const openTutorial = () => {
        const isAppRuntime = buckyos.getRuntimeType?.() === RuntimeType.AppRuntime;
        console.info("isAppRuntime", isAppRuntime);

        if (!AI_PROVIDER_TUTORIAL_URL) {
            return;
        }
        if (isAppRuntime) {
            console.info("openExternal via buckyos.openExternalUrl", {
                AI_PROVIDER_TUTORIAL_URL
            });
            buckyos.openExternalUrl?.(AI_PROVIDER_TUTORIAL_URL);
            return;
        }
        window.open(AI_PROVIDER_TUTORIAL_URL, "_blank", "noopener,noreferrer");
    };

    const handlePrimaryAction = () => {
        const normalizedConfig = normalizeProviderConfig(aiProviderConfig);
        onUpdate({ ai_provider_config: normalizedConfig });

        if (hasAnyProviderToken || hasLlmRouterEnabled) {
            onNext();
            return;
        }

        setSkipConfirmOpen(true);
    };

    const handleConfirmSkip = () => {
        setSkipConfirmOpen(false);
        onNext();
    };

    return (
        <Stack spacing={3}>
            <Stack spacing={1} alignItems="flex-start">
                <Lead>{t("ai_provider_description")}</Lead>
                <Link
                    component="button"
                    type="button"
                    underline="hover"
                    onClick={openTutorial}
                    sx={{ display: "inline-flex", alignItems: "center", gap: 0.5, fontSize: 14, color: "var(--cp-accent)" }}
                >
                    {t("ai_provider_tutorial_link")}
                    <ExternalLink size={14} strokeWidth={1.75} />
                </Link>
            </Stack>

            {hasLlmRouterEnabled ? (
                <StatusLine tone="success">{t("ai_provider_sn_included")}</StatusLine>
            ) : null}

            <Box
                sx={{
                    display: "grid",
                    gridTemplateColumns: { xs: "1fr", sm: "repeat(2, minmax(0, 1fr))" },
                    gap: 2,
                }}
            >
                {providers.map((provider) => (
                    <TextField
                        key={provider.key}
                        label={`${provider.title} ${t("ai_provider_token_label")}`}
                        type={visibleTokens[provider.key] ? "text" : "password"}
                        value={aiProviderConfig[provider.key]}
                        onChange={(event) => updateProviderField(provider.key, event.target.value)}
                        autoComplete="off"
                        inputProps={{ spellCheck: false, style: { fontFamily: "var(--cp-font-mono)", fontSize: 14 } }}
                        InputProps={{
                            endAdornment: (
                                <InputAdornment position="end">
                                    <IconButton
                                        onClick={() =>
                                            setVisibleTokens((prev) => ({
                                                ...prev,
                                                [provider.key]: !prev[provider.key],
                                            }))
                                        }
                                        edge="end"
                                        size="small"
                                        aria-label={t("toggle_secret_visibility")}
                                    >
                                        {visibleTokens[provider.key] ? (
                                            <EyeOff size={18} strokeWidth={1.75} />
                                        ) : (
                                            <Eye size={18} strokeWidth={1.75} />
                                        )}
                                    </IconButton>
                                </InputAdornment>
                            ),
                        }}
                    />
                ))}
            </Box>

            <StepActions back={<Button variant="text" onClick={onBack}>{t("back_button")}</Button>}>
                <Button variant="contained" size="large" onClick={handlePrimaryAction} sx={{ minWidth: 120 }}>
                    {primaryLabel}
                </Button>
            </StepActions>

            <Dialog
                open={skipConfirmOpen}
                onClose={() => setSkipConfirmOpen(false)}
                fullWidth
                maxWidth="xs"
            >
                <DialogTitle sx={{ fontSize: 18, fontWeight: 500, pb: 1 }}>{t("ai_provider_skip_title")}</DialogTitle>
                <DialogContent>
                    <Typography variant="body2" color="text.secondary">
                        {t("ai_provider_skip_body")}
                    </Typography>
                </DialogContent>
                <DialogActions sx={{ px: 3, pb: 3, gap: 1 }}>
                    <Button variant="outlined" onClick={() => setSkipConfirmOpen(false)}>
                        {t("ai_provider_skip_back")}
                    </Button>
                    <Button onClick={handleConfirmSkip} variant="contained">
                        {t("ai_provider_skip_confirm")}
                    </Button>
                </DialogActions>
            </Dialog>
        </Stack>
    );
};

export default AIProviderStep;
