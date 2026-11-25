import { Box, Stack, Typography } from "@mui/material";
import { useTranslation } from "react-i18next";
import { relativeName } from "../../domain";
import type { CheckStatus, DnsRecordSet, PlannedRecord, RecordConflict } from "../../types";
import { CopyField, StatusDot, mono, type Tone } from "../ui";

export const statusTone: Record<CheckStatus, Tone> = {
  pass: "success",
  waiting: "accent",
  fail: "danger",
  skipped: "muted",
};

export const formatRecord = (record: DnsRecordSet) => `${record.type} ${record.values.join(", ")}`;

type Props = {
  record: PlannedRecord;
  zoneApex: string;
  status: CheckStatus | null;
  conflict: RecordConflict | null;
};

/** One record for the user to add: name and value each with a copy button, and whether DNS already has it. */
const RecordCard = ({ record, zoneApex, status, conflict }: Props) => {
  const { t } = useTranslation();
  const short = relativeName(record.name, zoneApex);
  const shown = conflict && status !== "pass" ? "fail" : status;
  return (
    <Box
      data-record={record.id}
      data-status={shown ?? ""}
      sx={{
        p: 2,
        borderRadius: "12px",
        border: "1px solid",
        borderColor: shown === "fail" ? "var(--cp-danger)" : "var(--cp-border)",
        bgcolor: "var(--cp-surface-2)",
      }}
    >
      <Stack direction="row" spacing={2} alignItems="center" justifyContent="space-between" sx={{ mb: 1.5 }}>
        <Typography sx={{ ...mono, fontSize: 14, fontWeight: 500 }}>{record.type}</Typography>
        {shown ? (
          <Stack direction="row" spacing={1} alignItems="center">
            <StatusDot tone={statusTone[shown]} live={shown === "waiting"} />
            <Typography variant="body2" color="text.secondary">
              {t(`check_status_${shown}`)}
            </Typography>
          </Stack>
        ) : null}
      </Stack>
      <Stack spacing={1.5}>
        <Box>
          <CopyField label={t("dns_record_name")} value={record.name} />
          {short ? (
            <Typography variant="caption" color="text.secondary" component="div" sx={{ mt: 0.5, overflowWrap: "anywhere" }}>
              {t("dns_record_relative", { zone: zoneApex, name: short })}
            </Typography>
          ) : null}
        </Box>
        {record.values.map((value) => (
          <CopyField key={value} label={t("dns_record_value")} value={value} />
        ))}
        {conflict && status !== "pass" ? (
          <Typography variant="body2" sx={{ color: "var(--cp-danger)", overflowWrap: "anywhere" }}>
            {t("dns_record_conflict", {
              existing: conflict.existing.map(formatRecord).join("; "),
            })}
          </Typography>
        ) : null}
      </Stack>
    </Box>
  );
};

export default RecordCard;
