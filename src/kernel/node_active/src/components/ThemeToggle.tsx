import { DarkModeRounded, LightModeRounded } from "@mui/icons-material";
import { IconButton, PaletteMode, Tooltip } from "@mui/material";

type Props = {
  mode: PaletteMode;
  onToggle: () => void;
};

const ThemeToggle = ({ mode, onToggle }: Props) => (
  <Tooltip title={mode === "dark" ? "Light mode" : "Dark mode"}>
    <IconButton onClick={onToggle} size="small" sx={{ bgcolor: "action.hover" }}>
      {mode === "dark" ? <LightModeRounded /> : <DarkModeRounded />}
    </IconButton>
  </Tooltip>
);

export default ThemeToggle;
