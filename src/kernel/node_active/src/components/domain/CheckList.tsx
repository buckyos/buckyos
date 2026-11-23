import { Box, Stack, Typography } from "@mui/material";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { codeText } from "../../errors";
import type { CheckItem, CheckStatus, CheckTarget } from "../../types";
import { StatusDot, mono } from "../ui";
import { statusTone } from "./RecordCard";

const Values = ({ label, values }: { label: string; values: string[] }) => (
  <Box sx={{ display: "grid", gridTemplateColumns: "auto minmax(0, 1fr)", columnGap: 1.5, alignItems: "baseline" }}>
    <Typography variant="caption" color="text.secondary">
      {label}
    </Typography>
    <Typography sx={{ ...mono, fontSize: 12, overflowWrap: "anywhere" }}>{values.join("; ")}</Typography>
  </Box>
);

const Explanation = ({ entry }: { entry: Pick<CheckItem | CheckTarget, "code" | "expected" | "actual" | "status"> }) => {
  const { t } = useTranslation();
  if (entry.status === "pass") return null;
  return (
    <Stack spacing={0.5} sx={{ mt: 0.5 }}>
      {entry.code ? (
        <Typography variant="body2" color="text.secondary">
          {codeText(t, entry.code)}
        </Typography>
      ) : null}
      {entry.expected.length > 0 ? <Values label={t("check_expected")} values={entry.expected} /> : null}
      {entry.expected.length > 0 || entry.actual.length > 0 ? (
        <Values
          label={t("check_actual")}
          values={entry.actual.length > 0 ? entry.actual : [t("check_actual_none")]}
        />
      ) : null}
    </Stack>
  );
};

const Row = ({ title, status, children }: { title: ReactNode; status: CheckStatus; children?: ReactNode }) => {
  const { t } = useTranslation();
  return (
    <Stack direction="row" spacing={1.5} sx={{ minWidth: 0 }}>
      <Box sx={{ display: "flex", alignItems: "center", height: 20, flexShrink: 0 }}>
        <StatusDot tone={statusTone[status]} live={status === "waiting"} />
      </Box>
      <Box sx={{ flex: 1, minWidth: 0 }}>
        <Stack direction="row" spacing={2} justifyContent="space-between">
          <Typography variant="body2" component="div" sx={{ minWidth: 0, overflowWrap: "anywhere" }}>
            {title}
          </Typography>
          <Typography variant="body2" color="text.secondary" sx={{ flexShrink: 0 }}>
            {t(`check_status_${status}`)}
          </Typography>
        </Stack>
        {children}
      </Box>
    </Stack>
  );
};

type Props = {
  items: CheckItem[];
  /** Something the user can do right at that item (wallet authorization). */
  action?: (item: CheckItem) => ReactNode;
};

/** Every check with what is expected, what DNS answers now and what to do about it. */
const CheckList = ({ items, action }: Props) => {
  const { t } = useTranslation();
  return (
    <Box sx={{ borderRadius: "12px", border: "1px solid var(--cp-border)", overflow: "hidden" }}>
      {items.map((item, index) => {
        const targets = (item.targets ?? []).filter((target) => target.status !== "pass");
        return (
          <Box
            key={item.id}
            data-check={item.id}
            data-status={item.status}
            sx={{ px: 2, py: 1.5, borderTop: index === 0 ? "none" : "1px solid var(--cp-border)" }}
          >
            <Row title={t(`check_item_${item.id}`)} status={item.status}>
              {targets.length > 0 ? (
                <Stack spacing={1.5} sx={{ mt: 1 }}>
                  {targets.map((target) => (
                    <Row
                      key={target.host}
                      title={<Box component="span" sx={{ ...mono, fontSize: 13 }}>{target.host}</Box>}
                      status={target.status}
                    >
                      <Explanation entry={target} />
                    </Row>
                  ))}
                </Stack>
              ) : (
                <Explanation entry={item} />
              )}
              {action?.(item)}
            </Row>
          </Box>
        );
      })}
    </Box>
  );
};

export default CheckList;
