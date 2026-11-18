import { createTheme, type PaletteMode } from "@mui/material";

/** The Desktop shell's MUI theme (src/frame/desktop/src/theme/provider.tsx), trimmed to what activation uses. */
export function createActiveTheme(mode: PaletteMode) {
  const light = mode === "light";
  const primaryButton = light
    ? { background: "#406ad5", hover: "#3a60c4", active: "#3557b3", text: "#ffffff" }
    : { background: "#ededed", hover: "#ffffff", active: "#d6d6d6", text: "#141415" };

  return createTheme({
    palette: {
      mode,
      primary: { main: light ? "#406ad5" : "#87aaf1" },
      secondary: { main: light ? "#6a7587" : "#a1a1a6" },
      success: { main: light ? "#3ea67e" : "#69c49c" },
      background: {
        default: light ? "#f3f3f2" : "#141415",
        paper: light ? "#fdfdfc" : "#1c1c1e",
      },
      text: {
        primary: light ? "#1b1b1a" : "#ededed",
        secondary: light ? "#6b6b69" : "#a1a1a6",
      },
      divider: light ? "rgba(27, 27, 26, 0.12)" : "rgba(237, 237, 237, 0.1)",
    },
    typography: {
      fontFamily: "var(--cp-font-sans)",
      h1: { fontWeight: 500 },
      h2: { fontWeight: 500 },
      h3: { fontWeight: 500 },
      h4: { fontWeight: 500 },
      h5: { fontWeight: 500, fontSize: "1.25rem", lineHeight: 1.4 },
      h6: { fontWeight: 500, fontSize: "1rem", lineHeight: 1.5 },
      subtitle1: { fontWeight: 500 },
      subtitle2: { fontWeight: 500 },
      button: { fontWeight: 500, letterSpacing: 0, textTransform: "none" },
    },
    shape: { borderRadius: 12 },
    components: {
      MuiPaper: {
        styleOverrides: { root: { backgroundImage: "none" } },
      },
      MuiButton: {
        defaultProps: { disableElevation: true },
        styleOverrides: {
          root: {
            minHeight: 40,
            borderRadius: 10,
            paddingInline: 16,
            boxShadow: "none",
            transition:
              "transform 120ms var(--cp-ease-smooth), background-color 150ms var(--cp-ease-smooth), border-color 150ms var(--cp-ease-smooth), color 150ms var(--cp-ease-smooth)",
            "&:hover": { boxShadow: "none" },
            "&:active": { transform: "scale(0.98)" },
            "&.Mui-disabled": { boxShadow: "none", transform: "none" },
          },
          containedPrimary: {
            background: primaryButton.background,
            color: primaryButton.text,
            "&:hover": { background: primaryButton.hover },
            "&:active": { background: primaryButton.active },
          },
          contained: {
            "&.Mui-disabled": {
              background: "color-mix(in srgb, var(--cp-text) 8%, var(--cp-surface-2))",
              color: "var(--cp-muted)",
            },
          },
          outlined: {
            borderColor: "var(--cp-border)",
            backgroundColor: "var(--cp-surface)",
            color: "var(--cp-text)",
            "&:hover": {
              borderColor: "var(--cp-border-strong)",
              backgroundColor: "color-mix(in srgb, var(--cp-text) 4%, var(--cp-surface))",
            },
          },
          text: {
            color: "var(--cp-text)",
            "&:hover": { backgroundColor: "color-mix(in srgb, var(--cp-text) 6%, transparent)" },
          },
          sizeSmall: { minHeight: 32, paddingInline: 12, borderRadius: 8 },
          sizeLarge: { minHeight: 48, paddingInline: 20, borderRadius: 12, fontSize: "0.9375rem" },
        },
      },
      MuiIconButton: {
        styleOverrides: {
          root: {
            color: "var(--cp-muted)",
            "&:hover": { backgroundColor: "color-mix(in srgb, var(--cp-text) 7%, transparent)" },
          },
        },
      },
      MuiTextField: {
        defaultProps: { fullWidth: true, variant: "outlined" },
      },
      MuiOutlinedInput: {
        styleOverrides: {
          root: {
            borderRadius: 10,
            backgroundColor: "var(--cp-surface)",
            "& .MuiOutlinedInput-notchedOutline": { borderColor: "var(--cp-border)" },
            "&:hover .MuiOutlinedInput-notchedOutline": { borderColor: "var(--cp-border-strong)" },
            "&.Mui-focused .MuiOutlinedInput-notchedOutline": { borderColor: "var(--cp-accent)", borderWidth: 1 },
            "&.Mui-error .MuiOutlinedInput-notchedOutline": { borderColor: "var(--cp-danger)" },
          },
          input: { color: "var(--cp-text)" },
        },
      },
      MuiInputLabel: {
        styleOverrides: {
          root: {
            color: "var(--cp-muted)",
            "&.Mui-focused": { color: "var(--cp-accent)" },
            "&.Mui-error": { color: "var(--cp-danger)" },
          },
        },
      },
      MuiFormHelperText: {
        styleOverrides: {
          root: {
            marginInline: 2,
            marginTop: 6,
            color: "var(--cp-muted)",
            "&.Mui-error": { color: "var(--cp-danger)" },
          },
        },
      },
      MuiMenu: {
        styleOverrides: {
          paper: {
            border: "1px solid var(--cp-border)",
            borderRadius: 12,
            background: "var(--cp-surface)",
            boxShadow: "var(--cp-panel-shadow)",
          },
        },
      },
      MuiMenuItem: {
        styleOverrides: {
          root: { minHeight: 36, borderRadius: 8, marginInline: 4, marginBlock: 2, fontSize: 14 },
        },
      },
      MuiAlert: {
        styleOverrides: {
          root: { borderRadius: 10, border: "1px solid var(--cp-border)" },
        },
      },
      MuiDialog: {
        styleOverrides: {
          paper: {
            borderRadius: 16,
            border: "1px solid var(--cp-border)",
            boxShadow: "var(--cp-panel-shadow)",
          },
        },
      },
      MuiTooltip: {
        styleOverrides: { tooltip: { fontSize: 12, borderRadius: 6 } },
      },
    },
  });
}
