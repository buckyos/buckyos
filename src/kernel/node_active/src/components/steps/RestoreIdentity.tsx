import { Button, CircularProgress, Stack, TextField } from "@mui/material";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { restoreWebOwner } from "../../../active_lib";
import { errorText, type ErrorText } from "../../errors";
import { ErrorAlert, Lead, StepActions } from "../ui";

type Props = {
  initialName: string;
  onCancel: () => void;
  onRestored: (restored: Awaited<ReturnType<typeof restoreWebOwner>>) => void;
};

const RestoreIdentity = ({ initialName, onCancel, onRestored }: Props) => {
  const { t } = useTranslation();
  const [name, setName] = useState(initialName);
  const [password, setPassword] = useState("");
  const [phrase, setPhrase] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<ErrorText | null>(null);

  const restore = async () => {
    const words = phrase.trim().toLowerCase().split(/[\s,]+/).filter(Boolean);
    const normalizedName = name.trim().toLowerCase();
    if (!normalizedName || !password) return setError({ message: t("error_restore_incomplete") });
    if (words.length !== 12) return setError({ message: t("code_mnemonic_invalid") });
    setError(null);
    setBusy(true);
    try {
      onRestored(await restoreWebOwner({ name: normalizedName, password, mnemonicWords: words }));
    } catch (cause) {
      setError(errorText(t, cause));
      setBusy(false);
    }
  };

  return (
    <Stack
      component="form"
      spacing={3}
      onSubmit={(event) => {
        event.preventDefault();
        void restore();
      }}
    >
      <Lead>{t("restore_intro")}</Lead>
      <Stack spacing={2}>
        <TextField
          label={t("username_placeholder")}
          value={name}
          onChange={(event) => setName(event.target.value)}
          autoComplete="username"
          inputProps={{ autoCapitalize: "none", spellCheck: false }}
        />
        <TextField
          type="password"
          label={t("restore_password_label")}
          value={password}
          onChange={(event) => setPassword(event.target.value)}
          autoComplete="current-password"
        />
        <TextField
          multiline
          minRows={2}
          label={t("restore_mnemonic_label")}
          value={phrase}
          onChange={(event) => setPhrase(event.target.value)}
          autoComplete="off"
          helperText={t("restore_mnemonic_hint")}
          inputProps={{ autoCapitalize: "none", spellCheck: false }}
        />
      </Stack>
      {error && <ErrorAlert {...error} />}
      <StepActions
        back={
          <Button variant="text" onClick={onCancel} disabled={busy}>
            {t("back_button")}
          </Button>
        }
      >
        <Button
          type="submit"
          variant="contained"
          size="large"
          disabled={busy}
          startIcon={busy ? <CircularProgress size={16} thickness={5} color="inherit" /> : undefined}
          sx={{ minWidth: 120 }}
        >
          {t("restore_button")}
        </Button>
      </StepActions>
    </Stack>
  );
};

export default RestoreIdentity;
