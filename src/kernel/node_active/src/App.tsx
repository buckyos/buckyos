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

const App = () => {
  const { t, i18n } = useTranslation();
  const prefersDark = useMediaQuery("(prefers-color-scheme: dark)");
  const [mode, setMode] = useState<PaletteMode>(prefersDark ? "dark" : "light");
  const [isWalletRuntime, setIsWalletRuntime] = useState(false);
  const [walletUser, setWalletUser] = useState<WalletUser | null>(null);

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
    try {
      const runtime = buckyos.getRuntimeType?.();
      if (runtime === RuntimeType.AppRuntime) {
        setIsWalletRuntime(true);
        buckyos.getCurrentWalletUser?.()
          .then((user) => {
            if (user) {
              setWalletUser({
                user_name: user.user_name || user.username || "",
                user_id: user.user_id,
                public_key: user.public_key || user.owner_public_key,
              });
            }
          })
          .catch((err: any) => {
            console.warn("Failed to load wallet user", err);
          });
      }
    } catch (err) {
      console.warn("Detect runtime failed", err);
    }
  }, []);

  const walletPubKeyDisplay = (() => {
    const pk = walletUser?.public_key;
    if (!pk) return "";
    const text = typeof pk === "string" ? pk : JSON.stringify(pk);
    return text.length > 22 ? `${text.slice(0, 10)}...${text.slice(-8)}` : text;
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
          alignItems: "center",
          py: { xs: 3, md: 6 },
          position: "relative",
        }}
      >
        <Container maxWidth="lg">
          <Paper
            elevation={0}
            sx={{
              p: { xs: 2.5, md: 4 },
              borderRadius: 4,
              border: `1px solid ${theme.palette.divider}`,
              boxShadow: "0 30px 90px rgba(0,0,0,0.15)",
              backdropFilter: "blur(10px)",
            }}
          >
            <Stack
              direction={{ xs: "column", sm: "row" }}
              justifyContent="space-between"
              alignItems={{ xs: "flex-start", sm: "center" }}
              spacing={2}
              sx={{ mb: 3 }}
            >
              <Box>
                <Typography variant="h4">{t("active_title")}</Typography>
                {isWalletRuntime && walletUser?.user_name && (
                  <Stack direction="row" spacing={1} alignItems="center" mt={0.5} flexWrap="wrap">
                    <Chip size="small" label={walletUser.user_name} />
                    {walletPubKeyDisplay && (
                      <Chip size="small" color="primary" label={walletPubKeyDisplay} />
                    )}
                  </Stack>
                )}
              </Box>
              <Stack direction="row" spacing={1.25}>
                <LanguageSwitch />
                <ThemeToggle mode={mode} onToggle={() => setMode((prev) => (prev === "light" ? "dark" : "light"))} />
              </Stack>
            </Stack>
            <ActiveWizard isWalletRuntime={isWalletRuntime} walletUser={walletUser || undefined} />
          </Paper>
        </Container>
      </Box>
    </ThemeProvider>
  );
};

export default App;
