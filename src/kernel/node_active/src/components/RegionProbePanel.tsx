import { PublicRounded, RefreshRounded } from "@mui/icons-material";
import {
  Box,
  Button,
  CircularProgress,
  FormControl,
  InputLabel,
  MenuItem,
  Paper,
  Select,
  Stack,
  Typography,
  type SelectChangeEvent,
} from "@mui/material";
import { useTranslation } from "react-i18next";
import type { RegionProbeStatus } from "../types";

type Props = {
  preference: string;
  status: RegionProbeStatus | null;
  locked: boolean;
  onPreferenceChange: (preference: string) => void;
  onRetry: () => void;
};

const RegionProbePanel = ({
  preference,
  status,
  locked,
  onPreferenceChange,
  onRetry,
}: Props) => {
  const { t } = useTranslation();
  const isRunning = !status || status.phase === "idle" || status.phase === "running";
  const selectedRegion = preference === "auto" ? status?.region : preference;
  const regions = [...(status?.available_regions || [])];
  if (
    preference !== "auto" &&
    !regions.some((region) => region.region_id === preference)
  ) {
    regions.push({ region_id: preference, priority: 0 });
  }

  const statusText = (() => {
    if (preference !== "auto") {
      return t("region_manual_status", {
        defaultValue: "Using manually selected Region: {{region}}",
        region: preference,
      });
    }
    if (isRunning) {
      return t("region_probe_detecting", "Detecting the nearest Region…");
    }
    if (selectedRegion) {
      return t("region_probe_selected", {
        defaultValue: "Detected Region: {{region}}",
        region: selectedRegion,
      });
    }
    return t(
      "region_probe_unavailable",
      "Region detection is unavailable. Registration will continue with automatic server fallback.",
    );
  })();

  const handleChange = (event: SelectChangeEvent<string>) => {
    onPreferenceChange(event.target.value);
  };

  return (
    <Paper
      variant="outlined"
      sx={{
        p: 2,
        borderRadius: 3,
        borderColor: selectedRegion ? "success.main" : "divider",
        bgcolor: "background.default",
      }}
    >
      <Stack
        direction={{ xs: "column", sm: "row" }}
        spacing={2}
        alignItems={{ xs: "stretch", sm: "center" }}
      >
        <Stack direction="row" spacing={1.5} alignItems="center" sx={{ flex: 1, minWidth: 0 }}>
          <Box
            sx={{
              display: "grid",
              placeItems: "center",
              width: 36,
              height: 36,
              flexShrink: 0,
              color: selectedRegion ? "success.main" : "text.secondary",
            }}
          >
            {isRunning ? <CircularProgress size={22} /> : <PublicRounded fontSize="small" />}
          </Box>
          <Box sx={{ minWidth: 0 }}>
            <Typography variant="subtitle2">
              {t("region_probe_title", "Network Region")}
            </Typography>
            <Typography variant="body2" color="text.secondary">
              {statusText}
              {preference === "auto" && status?.confidence === "low"
                ? ` ${t("region_probe_low_confidence", "Low confidence.")}`
                : ""}
              {preference === "auto" && status?.source === "cache"
                ? ` ${t("region_probe_cached", "Cached result.")}`
                : ""}
            </Typography>
          </Box>
        </Stack>

        <Stack direction="row" spacing={1} alignItems="center">
          <FormControl size="small" sx={{ minWidth: { xs: 0, sm: 190 }, flex: { xs: 1, sm: "none" } }}>
            <InputLabel id="region-preference-label">
              {t("region_preference_label", "Region")}
            </InputLabel>
            <Select
              labelId="region-preference-label"
              label={t("region_preference_label", "Region")}
              value={preference}
              onChange={handleChange}
              disabled={locked}
            >
              <MenuItem value="auto">{t("region_auto_option", "Auto (recommended)")}</MenuItem>
              {regions.map((region) => (
                <MenuItem key={region.region_id} value={region.region_id}>
                  {region.region_id}
                </MenuItem>
              ))}
            </Select>
          </FormControl>
          {!isRunning && !selectedRegion && (
            <Button
              variant="text"
              size="small"
              startIcon={<RefreshRounded />}
              onClick={onRetry}
              disabled={locked}
            >
              {t("retry_button", "Retry")}
            </Button>
          )}
        </Stack>
      </Stack>
    </Paper>
  );
};

export default RegionProbePanel;
