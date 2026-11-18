import { useEffect, useMemo, useState } from "react";
import {
  Alert,
  Box,
  Button,
  CircularProgress,
  CssBaseline,
  PaletteMode,
  Stack,
  ThemeProvider,
  Typography,
  useMediaQuery,
} from "@mui/material";
import { useTranslation } from "react-i18next";
import { buckyos, RuntimeType } from "buckyos";
import { validateOwnerDocument } from "../active_lib";
import ActiveWizard from "./components/ActiveWizard";
import LanguageSwitch from "./components/LanguageSwitch";
import ThemeToggle from "./components/ThemeToggle";
import { BrandMark, StatusDot } from "./components/ui";
import { createActiveTheme } from "./theme";
import { WalletUser } from "./types";

function normalizeWalletPublicKey(value: unknown) {
  if (!value) {
    return undefined;
  }
  if (typeof value === "object") {
    return value as WalletUser["public_key"];
  }
  if (typeof value === "string") {
    try {
      return JSON.parse(value);
    } catch {
      return undefined;
    }
  }
  return undefined;
}

const App = () => {
  const { t, i18n } = useTranslation();
  const prefersDark = useMediaQuery("(prefers-color-scheme: dark)");
  const [mode, setMode] = useState<PaletteMode>(prefersDark ? "dark" : "light");
  const [isWalletRuntime, setIsWalletRuntime] = useState(false);
  const [walletUser, setWalletUser] = useState<WalletUser | null>(null);
  const [isInitialized, setIsInitialized] = useState(false);
  const [initError, setInitError] = useState<string | null>(null);
  const [initAttempt, setInitAttempt] = useState(0);

  useEffect(() => {
    setMode(prefersDark ? "dark" : "light");
  }, [prefersDark]);

  useEffect(() => {
    document.documentElement.dataset.theme = mode;
  }, [mode]);

  useEffect(() => {
    document.title = t("active_title");
  }, [t, i18n.language]);

  useEffect(() => {
    let cancelled = false;
    let timeout: ReturnType<typeof setTimeout> | undefined;
    const init = async () => {
      let errorKey = "wallet_identity_load_failed";
      try {
        setIsInitialized(false);
        setInitError(null);
        setWalletUser(null);
        const runtime = buckyos.getRuntimeType?.();
        const isAppRuntime = runtime === RuntimeType.AppRuntime;
        setIsWalletRuntime(isAppRuntime);
        if (!isAppRuntime) {
          return;
        }

        const user = await Promise.race([
          buckyos.getCurrentWalletUser(),
          new Promise<never>((_, reject) => {
            timeout = setTimeout(() => reject(new Error("Wallet identity request timed out")), 15000);
          }),
        ]);
        if (!user) throw new Error("Wallet identity unavailable");

        errorKey = "wallet_identity_incomplete";

        const normalizedUser = user as Record<string, unknown>;
        const snUsername =
          typeof normalizedUser.sn_username === "string"
            ? normalizedUser.sn_username.trim().toLowerCase()
            : "";
        if (!snUsername) throw new Error("Wallet user is missing sn_username");
        if (!normalizedUser.owner_document) {
          throw new Error("Wallet user is missing owner_document");
        }
        const bridgePublicKey = normalizeWalletPublicKey(normalizedUser.public_key);
        const ownerDocument = validateOwnerDocument(
          normalizedUser.owner_document,
          snUsername,
          bridgePublicKey,
        );

        if (cancelled) return;
        setWalletUser({
          owner_document: ownerDocument,
          public_key: bridgePublicKey,
          sn_username: snUsername,
        });
      } catch (err: any) {
        console.warn("App initialization failed", err);
        if (!cancelled) {
          setInitError(errorKey);
        }
      } finally {
        clearTimeout(timeout);
        if (!cancelled) {
          setIsInitialized(true);
        }
      }
    };

    init();
    return () => {
      cancelled = true;
      clearTimeout(timeout);
    };
  }, [initAttempt]);

  const walletPubKeyDisplay = (() => {
    const pk = walletUser?.public_key;
    if (!pk) return "";
    const text = typeof pk === "string" ? pk : pk.x;
    return text;
  })();

  const theme = useMemo(() => createActiveTheme(mode), [mode]);

  return (
    <ThemeProvider theme={theme}>
      <CssBaseline />
      <Box
        sx={{
          minHeight: "100dvh",
          display: "flex",
          justifyContent: "center",
          alignItems: { xs: "flex-start", md: "center" },
          px: { xs: 2, sm: 3 },
          py: { xs: 2, sm: 5 },
          bgcolor: "var(--cp-bg)",
        }}
      >
        <Box sx={{ width: "100%", maxWidth: 760 }}>
          <Stack
            component="header"
            direction="row"
            alignItems="center"
            spacing={1.5}
            sx={{ mb: { xs: 3, sm: 3 }, minHeight: 40 }}
          >
            <BrandMark size={28} />
            <Typography component="h1" sx={{ flex: 1, minWidth: 0, fontSize: 16, fontWeight: 500, lineHeight: 1.4 }}>
              {t("active_title")}
            </Typography>
            <Stack direction="row" spacing={1} alignItems="center" sx={{ flexShrink: 0 }}>
              <LanguageSwitch />
              <ThemeToggle mode={mode} onToggle={() => setMode((prev) => (prev === "light" ? "dark" : "light"))} />
            </Stack>
          </Stack>
          <Box
            component="main"
            sx={{
              p: { xs: 0, sm: 4 },
              borderRadius: { xs: 0, sm: "16px" },
              border: { xs: "none", sm: "1px solid var(--cp-border)" },
              bgcolor: { xs: "transparent", sm: "var(--cp-surface)" },
              boxShadow: { xs: "none", sm: "var(--cp-panel-shadow)" },
            }}
          >
            {isWalletRuntime && walletUser?.owner_document.name && (
              <Stack
                direction="row"
                spacing={1}
                alignItems="center"
                sx={{ mb: 3, pb: 2, borderBottom: "1px solid var(--cp-border)", minWidth: 0 }}
              >
                <StatusDot tone="success" />
                <Typography variant="body2" color="text.secondary" sx={{ minWidth: 0, overflowWrap: "anywhere" }}>
                  {t("wallet_device_owner", {
                    user_name: walletUser.owner_document.name,
                    public_key: walletPubKeyDisplay,
                  })}
                </Typography>
              </Stack>
            )}
            {!isInitialized ? (
              <Stack direction="row" spacing={1.5} alignItems="center" sx={{ py: 2 }}>
                <CircularProgress size={16} thickness={5} />
                <Typography variant="body2" color="text.secondary">
                  {t(isWalletRuntime ? "wallet_identity_loading" : "loading")}
                </Typography>
              </Stack>
            ) : initError ? (
              <Stack spacing={2}>
                <Alert severity="error">{t(initError)}</Alert>
                <Typography variant="body2" color="text.secondary">
                  {t("wallet_identity_help")}
                </Typography>
                <Button
                  variant="contained"
                  sx={{ alignSelf: "flex-start" }}
                  onClick={() => {
                    setIsInitialized(false);
                    setInitError(null);
                    setInitAttempt((attempt) => attempt + 1);
                  }}
                >
                  {t("retry_button")}
                </Button>
              </Stack>
            ) : (
              <ActiveWizard isWalletRuntime={isWalletRuntime} walletUser={walletUser || undefined} />
            )}
          </Box>
        </Box>
      </Box>
    </ThemeProvider>
  );
};

export default App;
