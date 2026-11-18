import { Moon, Sun } from "lucide-react";
import { IconButton, PaletteMode, Tooltip } from "@mui/material";
import { useTranslation } from "react-i18next";

type Props = {
  mode: PaletteMode;
  onToggle: () => void;
};

const ThemeToggle = ({ mode, onToggle }: Props) => {
  const { t } = useTranslation();
  const label =
    mode === "dark"
      ? t("theme_light", { defaultValue: "Light mode" })
      : t("theme_dark", { defaultValue: "Dark mode" });
  return (
    <Tooltip title={label}>
      <IconButton
        onClick={onToggle}
        aria-label={label}
        sx={{
          width: 40,
          height: 40,
          flexShrink: 0,
          borderRadius: "10px",
          border: "1px solid var(--cp-border)",
          bgcolor: "var(--cp-surface)",
          "&:hover": { bgcolor: "color-mix(in srgb, var(--cp-text) 4%, var(--cp-surface))" },
        }}
      >
        {mode === "dark" ? <Sun size={18} strokeWidth={1.75} /> : <Moon size={18} strokeWidth={1.75} />}
      </IconButton>
    </Tooltip>
  );
};

export default ThemeToggle;
