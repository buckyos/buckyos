import {
  Alert,
  Box,
  Button,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  Grid,
  IconButton,
  InputAdornment,
  Paper,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import {
  AutoAwesomeRounded,
  CheckCircleRounded,
  LaunchRounded,
  LockOpenRounded,
  VisibilityOffRounded,
  VisibilityRounded,
} from "@mui/icons-material";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { AIProviderConfig, WizardData } from "../../types";
import {
  AI_PROVIDER_TUTORIAL_URL,
  check_sn_active_code,
} from "../../../active_lib";

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
  { key: "openrouter_api_token", title: "OpenRouter" },
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
  const [snDialogOpen, setSnDialogOpen] = useState(false);
  const [snDialogValue, setSnDialogValue] = useState("");
  const [snDialogError, setSnDialogError] = useState("");
  const [snSubmitting, setSnSubmitting] = useState(false);
  const [skipConfirmOpen, setSkipConfirmOpen] = useState(false);

  const aiProviderConfig = wizardData.ai_provider_config;
  const hasAnyProviderToken = useMemo(
    () =>
      Object.values(aiProviderConfig).some(
        (value) => typeof value === "string" && value.trim().length > 0
      ),
    [aiProviderConfig]
  );
  const hasActiveCode = Boolean(wizardData.sn_active_code?.trim());
  const primaryLabel =
    hasAnyProviderToken || hasActiveCode ? t("next_button") : t("skip_button");

  const normalizeProviderConfig = (
    config: AIProviderConfig
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
    if (!AI_PROVIDER_TUTORIAL_URL) {
      return;
    }
    window.open(AI_PROVIDER_TUTORIAL_URL, "_blank", "noopener,noreferrer");
  };

  const handlePrimaryAction = () => {
    const normalizedConfig = normalizeProviderConfig(aiProviderConfig);
    onUpdate({ ai_provider_config: normalizedConfig });

    if (hasAnyProviderToken || hasActiveCode) {
      onNext();
      return;
    }

    setSkipConfirmOpen(true);
  };

  const handleConfirmSkip = () => {
    setSkipConfirmOpen(false);
    onNext();
  };

  const handleConfirmSnCode = async () => {
    const value = snDialogValue.trim();
    setSnDialogError("");

    if (!value) {
      setSnDialogError(t("error_invite_code_too_short") || "");
      return;
    }

    setSnSubmitting(true);
    try {
      const ok = await check_sn_active_code(value);
      if (!ok) {
        setSnDialogError(t("error_invite_code_invalid") || "");
        return;
      }

      onUpdate({ sn_active_code: value });
      setSnDialogOpen(false);
      setSnDialogValue("");
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setSnDialogError(
        `${t("error_invite_code_invalid") || "Invalid active code"} ${msg}`
      );
    } finally {
      setSnSubmitting(false);
    }
  };

  return (
    <Stack spacing={3}>
      <Stack
        direction={{ xs: "column", sm: "row" }}
        spacing={1.5}
        justifyContent="space-between"
        alignItems={{ xs: "flex-start", sm: "center" }}
      >
        <Alert icon={<AutoAwesomeRounded />} severity="info" sx={{ flex: 1 }}>
          {t("ai_provider_description")}
        </Alert>
        <Button
          variant="outlined"
          onClick={openTutorial}
          endIcon={<LaunchRounded />}
          sx={{ minHeight: 44 }}
        >
          {t("ai_provider_tutorial_link")}
        </Button>
      </Stack>

      <Grid container spacing={2}>
        {providers.map((provider) => (
          <Grid item xs={12} md={6} key={provider.key}>
            <Paper
              variant="outlined"
              sx={{
                p: 2,
                borderRadius: 3,
                height: "100%",
              }}
            >
              <Stack spacing={1.25}>
                <Typography fontWeight={700}>{provider.title}</Typography>
                <TextField
                  label={t("ai_provider_token_label")}
                  type={visibleTokens[provider.key] ? "text" : "password"}
                  value={aiProviderConfig[provider.key]}
                  onChange={(event) =>
                    updateProviderField(provider.key, event.target.value)
                  }
                  fullWidth
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
                          aria-label={t("toggle_secret_visibility")}
                        >
                          {visibleTokens[provider.key] ? (
                            <VisibilityOffRounded />
                          ) : (
                            <VisibilityRounded />
                          )}
                        </IconButton>
                      </InputAdornment>
                    ),
                  }}
                />
              </Stack>
            </Paper>
          </Grid>
        ))}
      </Grid>

      {hasActiveCode ? (
        <Alert icon={<CheckCircleRounded />} severity="success">
          {t("ai_provider_sn_included")}
        </Alert>
      ) : (
        <Paper
          variant="outlined"
          sx={{
            p: 2,
            borderRadius: 3,
          }}
        >
          <Stack
            direction={{ xs: "column", sm: "row" }}
            spacing={1.5}
            justifyContent="space-between"
            alignItems={{ xs: "flex-start", sm: "center" }}
          >
            <Box>
              <Typography fontWeight={700}>
                {t("ai_provider_sn_section_title")}
              </Typography>
              <Typography variant="body2" color="text.secondary">
                {t("ai_provider_sn_section_desc")}
              </Typography>
            </Box>
            <Button
              variant="outlined"
              startIcon={<LockOpenRounded />}
              onClick={() => {
                setSnDialogValue(wizardData.sn_active_code || "");
                setSnDialogError("");
                setSnDialogOpen(true);
              }}
              sx={{ minHeight: 44 }}
            >
              {t("ai_provider_fill_sn_code")}
            </Button>
          </Stack>
        </Paper>
      )}

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
          onClick={handlePrimaryAction}
          sx={{ py: 1.15, minWidth: 160 }}
        >
          {primaryLabel}
        </Button>
      </Stack>

      <Dialog open={snDialogOpen} onClose={() => !snSubmitting && setSnDialogOpen(false)} fullWidth maxWidth="sm">
        <DialogTitle>{t("ai_provider_sn_dialog_title")}</DialogTitle>
        <DialogContent>
          <Stack spacing={2} sx={{ pt: 1 }}>
            <TextField
              label={t("sn_active_code_label")}
              value={snDialogValue}
              onChange={(event) => setSnDialogValue(event.target.value)}
              error={Boolean(snDialogError)}
              helperText={snDialogError || " "}
              fullWidth
              autoFocus
            />
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setSnDialogOpen(false)} disabled={snSubmitting}>
            {t("cancel_button")}
          </Button>
          <Button onClick={handleConfirmSnCode} disabled={snSubmitting} variant="contained">
            {t("confirm_button")}
          </Button>
        </DialogActions>
      </Dialog>

      <Dialog
        open={skipConfirmOpen}
        onClose={() => setSkipConfirmOpen(false)}
        fullWidth
        maxWidth="sm"
      >
        <DialogTitle>{t("ai_provider_skip_title")}</DialogTitle>
        <DialogContent>
          <Typography color="text.secondary">
            {t("ai_provider_skip_body")}
          </Typography>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setSkipConfirmOpen(false)}>
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
