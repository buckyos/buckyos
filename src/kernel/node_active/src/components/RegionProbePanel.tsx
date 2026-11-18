import { RotateCw } from "lucide-react";
import {
  Box,
  Button,
  FormControl,
  MenuItem,
  Select,
  Stack,
  Typography,
  type SelectChangeEvent,
} from "@mui/material";
import { useTranslation } from "react-i18next";
import type { RegionProbeStatus } from "../types";
import { StatusLine, type Tone } from "./ui";

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

  const tone: Tone = preference !== "auto" ? "accent" : isRunning ? "muted" : selectedRegion ? "success" : "warning";
  const detail = [
    preference === "auto" && status?.confidence === "low" ? t("region_probe_low_confidence", "Low confidence.") : "",
    preference === "auto" && status?.source === "cache" ? t("region_probe_cached", "Cached result.") : "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <Box
      sx={{
        p: 2,
        borderRadius: "12px",
        border: "1px solid var(--cp-border)",
        bgcolor: "var(--cp-surface-2)",
      }}
    >
      <Stack
        direction={{ xs: "column", sm: "row" }}
        spacing={{ xs: 1.5, sm: 2 }}
        alignItems={{ xs: "stretch", sm: "center" }}
      >
        <Box sx={{ flex: 1, minWidth: 0 }}>
          <Typography variant="subtitle2" sx={{ mb: 0.5 }}>
            {t("region_probe_title", "Network Region")}
          </Typography>
          <StatusLine tone={tone} live={preference === "auto" && isRunning}>
            {statusText}
            {detail ? ` ${detail}` : ""}
          </StatusLine>
        </Box>

        <Stack direction="row" spacing={1} alignItems="center">
          <FormControl size="small" sx={{ minWidth: { xs: 0, sm: 200 }, flex: { xs: 1, sm: "none" } }}>
            <Select
              value={preference}
              onChange={handleChange}
              disabled={locked}
              inputProps={{ "aria-label": t("region_preference_label", "Region") }}
              sx={{ height: 40, fontSize: 14 }}
            >
              <MenuItem value="auto">{t("region_auto_option", "Auto (recommended)")}</MenuItem>
              {regions.map((region) => (
                <MenuItem key={region.region_id} value={region.region_id} sx={{ fontFamily: "var(--cp-font-mono)", fontSize: 13 }}>
                  {region.region_id}
                </MenuItem>
              ))}
            </Select>
          </FormControl>
          {!isRunning && !selectedRegion && (
            <Button
              variant="outlined"
              startIcon={<RotateCw size={16} strokeWidth={1.75} />}
              onClick={onRetry}
              disabled={locked}
            >
              {t("retry_button", "Retry")}
            </Button>
          )}
        </Stack>
      </Stack>
    </Box>
  );
};

export default RegionProbePanel;
