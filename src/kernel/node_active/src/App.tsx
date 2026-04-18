import { useEffect, useMemo, useState } from "react";
import {
  Box,
  Container,
  CssBaseline,
  PaletteMode,
  Paper,
  Stack,
  Chip,
  ThemeProvider,
  Typography,
  createTheme,
  useMediaQuery,
} from "@mui/material";
import { useTranslation } from "react-i18next";
import { buckyos, RuntimeType } from "buckyos";
import ActiveWizard from "./components/ActiveWizard";
import LanguageSwitch from "./components/LanguageSwitch";
import ThemeToggle from "./components/ThemeToggle";
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

function normalizeWalletPasswordHash(user: Record<string, unknown>): string {
  const candidates = [
    user.user_password_hash,
    user.password_hash,
    user.pwd_hash,
  ];
  for (const candidate of candidates) {
    if (typeof candidate === "string" && candidate.trim().length > 0) {
      return candidate.trim();
    }
  }
  return "";
}

const App = () => {
  const { t, i18n } = useTranslation();
  const prefersDark = useMediaQuery("(prefers-color-scheme: dark)");
  const isMobile = useMediaQuery("(max-width:600px)");
  const [mode, setMode] = useState<PaletteMode>(prefersDark ? "dark" : "light");
  const [isWalletRuntime, setIsWalletRuntime] = useState(false);
  const [walletUser, setWalletUser] = useState<WalletUser | null>(null);
  const [isInitialized, setIsInitialized] = useState(false);
  const [initError, setInitError] = useState<string | null>(null);

  useEffect(() => {
    setMode(prefersDark ? "dark" : "light");
  }, [prefersDark]);

  useEffect(() => {
    document.body.dataset.theme = mode;
  }, [mode]);

  useEffect(() => {
    document.title = t("active_title");
  }, [t, i18n.language]);

  useEffect(() => {
    let cancelled = false;
    const init = async () => {
      try {
        setIsInitialized(false);
        setInitError(null);
        setWalletUser(null);
        setIsWalletRuntime(false);

        const runtime = buckyos.getRuntimeType?.();
        const isAppRuntime = runtime === RuntimeType.AppRuntime;
        if (!isAppRuntime) {
          return;
        }

        // Wallet runtime: wait wallet user result BEFORE rendering wizard.
        const user = await buckyos.getCurrentWalletUser?.();
        if (!user) {
          // If wallet user is unavailable, fall back to non-wallet flow.
          return;
        }

        const normalizedUser = user as Record<string, unknown>;

        if (cancelled) return;
        setWalletUser({
          user_name: String(normalizedUser.user_name || normalizedUser.username || "").toLowerCase(),
          user_id: typeof normalizedUser.did === "string" ? normalizedUser.did : undefined,
          public_key: normalizeWalletPublicKey(
            normalizedUser.owner_public_key || normalizedUser.public_key || normalizedUser.user_owner_key,
          ),
          sn_username: String(normalizedUser.sn_username || "").toLowerCase(),
          password_hash: normalizeWalletPasswordHash(normalizedUser),
        });
        setIsWalletRuntime(true);
      } catch (err: any) {
        console.warn("App initialization failed", err);
        if (!cancelled) {
          setInitError(err?.message || String(err));
        }
      } finally {
        if (!cancelled) {
          setIsInitialized(true);
        }
      }
    };

    init();
    return () => {
      cancelled = true;
    };
  }, []);

  const walletPubKeyDisplay = (() => {
    const pk = walletUser?.public_key;
    if (!pk) return "";
    const text = typeof pk === "string" ? pk : pk.x;
    return text;
  })();

  const theme = useMemo(
    () =>
      createTheme({
        palette: {
          mode,
          primary: {
            main: mode === "dark" ? "#9ad5ff" : "#4f46e5",
          },
          secondary: {
            main: mode === "dark" ? "#f3b0ff" : "#7c3aed",
          },
          background: {
            default: mode === "dark" ? "#0b1224" : "#eef1ff",
            paper: mode === "dark" ? "rgba(17, 26, 46, 0.9)" : "rgba(255,255,255,0.9)",
          },
        },
        shape: {
          borderRadius: 16,
        },
        typography: {
          fontFamily: "'Manrope','Space Grotesk','Inter',system-ui,-apple-system,sans-serif",
          h4: { fontWeight: 700, letterSpacing: "-0.02em" },
          subtitle1: { fontWeight: 600 },
        },
      }),
    [mode]
  );

  return (
    <ThemeProvider theme={theme}>
      <CssBaseline />
      <Box
        sx={{
          minHeight: "100vh",
          display: "flex",
          alignItems: { xs: "flex-start", md: "center" },
          py: { xs: 2, md: 6 },
          position: "relative",
        }}
      >
        <Container
          maxWidth="lg"
          disableGutters={isMobile}
          sx={{
            px: { xs: 2, sm: 3 },
          }}
        >
          <Paper
            elevation={0}
            sx={{
              p: { xs: 0, sm: 2.5, md: 4 },
              borderRadius: { xs: 0, sm: 4 },
              border: { xs: "none", sm: `1px solid ${theme.palette.divider}` },
              boxShadow: { xs: "none", sm: "0 30px 90px rgba(0,0,0,0.15)" },
              backdropFilter: { xs: "none", sm: "blur(10px)" },
              backgroundColor: { xs: "transparent", sm: theme.palette.background.paper },
            }}
          >
            <Stack
              direction={{ xs: "column", sm: "row" }}
              justifyContent="space-between"
              alignItems={{ xs: "flex-start", sm: "center" }}
              spacing={2}
              sx={{ mb: 3 }}
            >
              <Box sx={{ minWidth: 0, width: { xs: "100%", sm: "auto" } }}>
                <Typography variant="h4">{t("active_title")}</Typography>
                {isWalletRuntime && walletUser?.user_name && (
                  <Stack
                    direction="row"
                    spacing={1}
                    alignItems="center"
                    mt={0.5}
                    flexWrap="wrap"
                    sx={{ minWidth: 0, width: { xs: "100%", sm: "auto" } }}
                  >
                    <Chip
                      size="small"
                      label={t("wallet_device_owner", {
                        user_name: walletUser.user_name,
                        public_key: walletPubKeyDisplay,
                      })}
                      sx={{
                        maxWidth: { xs: "100%", sm: "fit-content" },
                        height: { xs: "auto", sm: 32 },
                        alignItems: { xs: "flex-start", sm: "center" },
                        "& .MuiChip-label": {
                          display: "block",
                          whiteSpace: { xs: "normal", sm: "nowrap" },
                          overflowWrap: { xs: "anywhere", sm: "normal" },
                          wordBreak: { xs: "break-word", sm: "normal" },
                          lineHeight: { xs: 1.4, sm: "32px" },
                          py: { xs: 0.75, sm: 0 },
                        },
                      }}
                    />
                  </Stack>
                )}
              </Box>
              <Stack direction="row" spacing={1.25}>
                <LanguageSwitch />
                <ThemeToggle mode={mode} onToggle={() => setMode((prev) => (prev === "light" ? "dark" : "light"))} />
              </Stack>
            </Stack>
            {!isInitialized ? (
              <Box sx={{ py: 2 }}>
                <Typography variant="body2" color="text.secondary" sx={{ mb: 1 }}>
                  {t("loading") || "Loading..."}
                </Typography>
                <Box
                  sx={{
                    height: 8,
                    borderRadius: 999,
                    backgroundColor: "action.hover",
                    overflow: "hidden",
                  }}
                >
                  <Box
                    sx={{
                      height: "100%",
                      width: "35%",
                      bgcolor: "primary.main",
                      opacity: 0.75,
                    }}
                  />
                </Box>
                {initError ? (
                  <Typography variant="body2" color="warning.main" sx={{ mt: 1 }}>
                    {initError}
                  </Typography>
                ) : null}
              </Box>
            ) : (
              <ActiveWizard isWalletRuntime={isWalletRuntime} walletUser={walletUser || undefined} />
            )}
          </Paper>
        </Container>
      </Box>
    </ThemeProvider>
  );
};

export default App;
