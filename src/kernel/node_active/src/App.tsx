import { useEffect, useMemo, useState } from "react";
import {
  Box,
  Container,
  CssBaseline,
  PaletteMode,
  Paper,
  Stack,
  ThemeProvider,
  Typography,
  createTheme,
  useMediaQuery,
} from "@mui/material";
import { useTranslation } from "react-i18next";
import ActiveWizard from "./components/ActiveWizard";
import LanguageSwitch from "./components/LanguageSwitch";
import ThemeToggle from "./components/ThemeToggle";

const App = () => {
  const { t, i18n } = useTranslation();
  const prefersDark = useMediaQuery("(prefers-color-scheme: dark)");
  const [mode, setMode] = useState<PaletteMode>(prefersDark ? "dark" : "light");

  useEffect(() => {
    setMode(prefersDark ? "dark" : "light");
  }, [prefersDark]);

  useEffect(() => {
    document.body.dataset.theme = mode;
  }, [mode]);

  useEffect(() => {
    document.title = t("active_title");
  }, [t, i18n.language]);

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
              </Box>
              <Stack direction="row" spacing={1.25}>
                <LanguageSwitch />
                <ThemeToggle mode={mode} onToggle={() => setMode((prev) => (prev === "light" ? "dark" : "light"))} />
              </Stack>
            </Stack>
            <ActiveWizard />
          </Paper>
        </Container>
      </Box>
    </ThemeProvider>
  );
};

export default App;
