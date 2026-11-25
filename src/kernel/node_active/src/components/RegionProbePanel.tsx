import { RotateCw } from "lucide-react";
import { Box, Button, FormControl, MenuItem, Select, Stack, Typography } from "@mui/material";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { RegionProbe } from "./useRegionProbe";
import { StatusLine, mono, type Tone } from "./ui";

const Panel = ({ children, control }: { children: ReactNode; control?: ReactNode }) => {
  const { t } = useTranslation();
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
            {t("region_probe_title")}
          </Typography>
          {children}
        </Box>
        {control}
      </Stack>
    </Box>
  );
};

/** An account that already exists: SN has assigned its relay and activation cannot change it. */
export const CurrentRelayPanel = ({ relay }: { relay: string | null }) => {
  const { t } = useTranslation();
  return (
    <Panel>
      <StatusLine tone={relay ? "success" : "muted"}>
        {relay ? t("region_current_relay", { relay }) : t("region_server_assigned")}
      </StatusLine>
      <Typography variant="caption" color="text.secondary" component="div" sx={{ mt: 0.5 }}>
        {t("region_existing_account")}
      </Typography>
    </Panel>
  );
};

const RegionProbePanel = ({ probe }: { probe: RegionProbe }) => {
  const { t } = useTranslation();
  const { status, preference } = probe;
  const running = !status || status.phase === "idle" || status.phase === "running";
  const selected = preference === "auto" ? status?.region ?? null : preference;
  const regionName = (id: string) => t(`region_name_${id}`, { defaultValue: id });
  const latency = (id: string) => {
    const score = status?.regions.find((region) => region.region_id === id)?.score_ms;
    return typeof score === "number" ? `${Math.round(score)} ms` : null;
  };
  const described = (id: string) => [regionName(id), latency(id)].filter(Boolean).join(" · ");

  const ids = (status?.available_regions ?? []).map((region) => region.region_id);
  if (preference !== "auto" && !ids.includes(preference)) ids.push(preference);

  const tone: Tone = running ? "muted" : selected ? "success" : "warning";
  const text = running
    ? t("region_probe_detecting")
    : preference !== "auto"
      ? t("region_manual_status", { region: described(preference) })
      : selected
        ? t("region_probe_selected", { region: described(selected) })
        : t("region_server_assigned");

  return (
    <Panel
      control={
        <Stack direction="row" spacing={1} alignItems="center">
          <FormControl size="small" sx={{ minWidth: { xs: 0, sm: 220 }, flex: { xs: 1, sm: "none" } }}>
            <Select
              value={preference}
              onChange={(event) => probe.choose(event.target.value)}
              inputProps={{ "aria-label": t("region_preference_label") }}
              renderValue={(value) => (value === "auto" ? t("region_auto_option") : regionName(value))}
              sx={{ height: 40, fontSize: 14 }}
            >
              <MenuItem value="auto">{t("region_auto_option")}</MenuItem>
              {ids.map((id) => (
                <MenuItem key={id} value={id} sx={{ justifyContent: "space-between", gap: 3 }}>
                  <span>{regionName(id)}</span>
                  <Box component="span" sx={{ ...mono, fontSize: 13, color: "text.secondary" }}>
                    {latency(id) ?? t("region_no_latency")}
                  </Box>
                </MenuItem>
              ))}
            </Select>
          </FormControl>
          <Button
            variant="outlined"
            startIcon={<RotateCw size={16} strokeWidth={1.75} />}
            onClick={probe.retry}
            disabled={running}
            sx={{ flexShrink: 0 }}
          >
            {t("region_probe_again")}
          </Button>
        </Stack>
      }
    >
      <StatusLine tone={tone} live={running}>
        {text}
      </StatusLine>
    </Panel>
  );
};

export default RegionProbePanel;
