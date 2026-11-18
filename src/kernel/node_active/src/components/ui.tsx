import { Box, Stack, Typography } from "@mui/material";
import type { ReactNode } from "react";

/** Some older strings were written as inline labels ("RTCP Port:"); a floating label or a card title has no colon. */
export const plainLabel = (text: string) => text.replace(/[:：]\s*$/, "");

export type Tone = "success" | "warning" | "danger" | "accent" | "muted";

const toneColor: Record<Tone, string> = {
  success: "var(--cp-success)",
  warning: "var(--cp-warning)",
  danger: "var(--cp-danger)",
  accent: "var(--cp-accent)",
  muted: "var(--cp-muted)",
};

/** The Desktop shell's 6px status dot. `live` breathes while something is still in progress. */
export const StatusDot = ({ tone, live = false }: { tone: Tone; live?: boolean }) => (
  <Box
    component="span"
    aria-hidden="true"
    sx={{
      width: 6,
      height: 6,
      flexShrink: 0,
      borderRadius: "50%",
      bgcolor: toneColor[tone],
      animation: live ? "na-live 1.6s var(--cp-ease-smooth) infinite" : "none",
    }}
  />
);

/** A status line: dot + text, used where a state needs to read at a glance. */
export const StatusLine = ({ tone, live, children }: { tone: Tone; live?: boolean; children: ReactNode }) => (
  <Stack direction="row" spacing={1} alignItems="center" sx={{ minWidth: 0 }}>
    <StatusDot tone={tone} live={live} />
    <Typography variant="body2" color="text.secondary" sx={{ minWidth: 0 }}>
      {children}
    </Typography>
  </Stack>
);

/** The step's explanation, in plain secondary text instead of a coloured banner. */
export const Lead = ({ children }: { children: ReactNode }) => (
  <Typography variant="body2" color="text.secondary" sx={{ whiteSpace: "pre-line", maxWidth: 640 }}>
    {children}
  </Typography>
);

/** A neutral boxed note for instructions that belong to one choice (port hints, publication steps). */
export const Note = ({ title, children }: { title?: ReactNode; children: ReactNode }) => (
  <Box
    sx={{
      p: 2,
      borderRadius: "12px",
      border: "1px solid var(--cp-border)",
      bgcolor: "var(--cp-surface-2)",
    }}
  >
    {title ? (
      <Typography variant="subtitle2" sx={{ mb: 1 }}>
        {title}
      </Typography>
    ) : null}
    <Box sx={{ color: "text.secondary", fontSize: 14, lineHeight: 1.6, whiteSpace: "pre-line" }}>{children}</Box>
  </Box>
);

type ChoiceCardProps = {
  selected: boolean;
  onSelect: () => void;
  icon: ReactNode;
  title: ReactNode;
  description: ReactNode;
};

/** One option of a single-choice group: hairline card, the chosen one takes the accent border and a radio mark. */
export const ChoiceCard = ({ selected, onSelect, icon, title, description }: ChoiceCardProps) => (
  <Box
    role="radio"
    aria-checked={selected}
    tabIndex={0}
    onClick={onSelect}
    onKeyDown={(event) => {
      if (event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        onSelect();
      }
    }}
    sx={{
      position: "relative",
      height: "100%",
      p: 2,
      pr: 5,
      borderRadius: "12px",
      cursor: "pointer",
      border: "1px solid",
      borderColor: selected ? "var(--cp-accent)" : "var(--cp-border)",
      boxShadow: selected ? "inset 0 0 0 1px var(--cp-accent)" : "none",
      bgcolor: selected ? "var(--cp-accent-surface)" : "var(--cp-surface)",
      transition:
        "border-color 150ms var(--cp-ease-smooth), background-color 150ms var(--cp-ease-smooth), box-shadow 150ms var(--cp-ease-smooth)",
      "&:hover": {
        borderColor: selected ? "var(--cp-accent)" : "var(--cp-border-strong)",
      },
    }}
  >
    <Box
      aria-hidden="true"
      sx={{
        position: "absolute",
        top: 16,
        right: 16,
        width: 16,
        height: 16,
        borderRadius: "50%",
        border: "1.5px solid",
        borderColor: selected ? "var(--cp-accent)" : "var(--cp-border-strong)",
        display: "grid",
        placeItems: "center",
        "&::after": {
          content: '""',
          width: 8,
          height: 8,
          borderRadius: "50%",
          bgcolor: "var(--cp-accent)",
          opacity: selected ? 1 : 0,
          transition: "opacity 150ms var(--cp-ease-smooth)",
        },
      }}
    />
    <Box sx={{ display: "flex", flexDirection: { xs: "row", md: "column" }, gap: { xs: 1.5, md: 1.5 } }}>
      <Box sx={{ display: "flex", flexShrink: 0, color: selected ? "text.primary" : "text.secondary" }}>{icon}</Box>
      <Box sx={{ minWidth: 0 }}>
        <Typography variant="subtitle2" sx={{ fontSize: 15, lineHeight: "20px", mb: 0.5 }}>
          {title}
        </Typography>
        <Typography variant="body2" color="text.secondary">
          {description}
        </Typography>
      </Box>
    </Box>
  </Box>
);

/** The bottom row of a step: Back on the left, the one primary action on the right. */
export const StepActions = ({ back, children }: { back?: ReactNode; children?: ReactNode }) => (
  <Stack direction="row" justifyContent="space-between" alignItems="center" spacing={2} sx={{ pt: 1 }}>
    <Box>{back}</Box>
    <Stack direction="row" spacing={1.5} alignItems="center">
      {children}
    </Stack>
  </Stack>
);

/** The BuckyOS mark from res/favicon.svg, drawn flat in the brand blue. */
export const BrandMark = ({ size = 28 }: { size?: number }) => (
  <Box component="svg" viewBox="0 0 64 64" aria-hidden="true" sx={{ width: size, height: size, flexShrink: 0, display: "block" }}>
    <circle cx="32" cy="32" r="32" fill="#406AD5" />
    <g transform="translate(1.5 0) rotate(14 32 32)" fill="#FFFFFF">
      <path
        fillRule="evenodd"
        d="M21 11H27V30C32 26 38 25 42 28C47 31 48 38 44 43C40 48 31 51 21 53ZM27 36V45C31 43.5 36 41 38 38C40 35 39 32.5 36 33C33 33.4 29.5 35 27 36Z"
      />
      <path d="M30.5 11H33.5V53H30.5Z" />
    </g>
  </Box>
);

export const mono = { fontFamily: "var(--cp-font-mono)", fontVariantNumeric: "tabular-nums" } as const;
