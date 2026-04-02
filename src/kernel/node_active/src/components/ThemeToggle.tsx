import { DarkModeRounded, LightModeRounded } from "@mui/icons-material";
import { IconButton, PaletteMode, Tooltip } from "@mui/material";

type Props = {
  mode: PaletteMode;
  onToggle: () => void;
};

const ThemeToggle = ({ mode, onToggle }: Props) => (
  <Tooltip title={mode === "dark" ? "Light mode" : "Dark mode"}>
    <IconButton
      onClick={onToggle}
      size="small"
      sx={{
        width: 42,
        height: 42,
        flexShrink: 0,
        borderRadius: "50%",
        border: "1px solid",
        borderColor: "divider",
        bgcolor: "background.paper",
        boxShadow: "0 6px 16px rgba(15, 23, 42, 0.08)",
        "&:hover": {
          bgcolor: "action.hover",
        },
      }}
    >
      {mode === "dark" ? <LightModeRounded /> : <DarkModeRounded />}
    </IconButton>
  </Tooltip>
);

export default ThemeToggle;
