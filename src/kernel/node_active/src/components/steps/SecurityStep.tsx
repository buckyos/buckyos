import { Check, KeyRound } from "lucide-react";
import {
  Alert,
  Box,
  Button,
  CircularProgress,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import { buckyos } from "buckyos";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  WEB3_BASE_HOST,
  buildWebOwnerDocument,
  check_bucky_username,
  check_sn_active_code,
  generateWebOwnerMaterial,
  get_net_id_by_gateway_type,
  registerWebOwner,
  resolveEnabledFeatures,
} from "../../../active_lib";
import { errorText, type ErrorText } from "../../errors";
import { WebOwnerMaterial, WizardData } from "../../types";
import type { RegionProbe } from "../useRegionProbe";
import { ErrorAlert, Lead, StepActions, mono } from "../ui";
import RestoreIdentity from "./RestoreIdentity";

type Props = {
  wizardData: WizardData;
  onUpdate: (data: Partial<WizardData>) => void;
  onNext: () => void;
  onBack: () => void;
  region: RegionProbe;
};

type NameStatus = "idle" | "checking" | "ok" | "taken" | "invalid";
type ActiveCodeStatus = "idle" | "checking" | "valid" | "invalid";
type Phase = "generate" | "backup" | "registering" | "restore" | "complete";

function confirmationPositions(): [number, number] {
  const bytes = new Uint8Array(2);
  crypto.getRandomValues(bytes);
  const first = bytes[0] % 12;
  let second = bytes[1] % 12;
  if (second === first) second = (second + 5) % 12;
  return first < second ? [first, second] : [second, first];
}

const SecurityStep = ({ wizardData, onUpdate, onNext, onBack, region }: Props) => {
  const { t } = useTranslation();
  const [phase, setPhase] = useState<Phase>(
    wizardData.owner_document ? "complete" : wizardData.web_owner_material ? "backup" : "generate",
  );
  const [material, setMaterial] = useState<WebOwnerMaterial | null>(
    wizardData.web_owner_material,
  );
  const [positions, setPositions] = useState<[number, number]>([2, 8]);
  const [confirmWords, setConfirmWords] = useState<[string, string]>(["", ""]);
  const [username, setUsername] = useState(wizardData.sn_user_name || "");
  const [email, setEmail] = useState("");
  const [activeCode, setActiveCode] = useState(wizardData.sn_active_code || "");
  const [password, setPassword] = useState("");
  const [passwordConfirm, setPasswordConfirm] = useState("");
  const [nameStatus, setNameStatus] = useState<NameStatus>("idle");
  const [activeCodeStatus, setActiveCodeStatus] = useState<ActiveCodeStatus>("idle");
  const [error, setError] = useState<ErrorText | null>(null);
  const registrationRegionRef = useRef<{ name: string; region: string | null } | null>(null);
  const fail = (message: string) => setError({ message });

  useEffect(() => {
    const name = username.trim().toLowerCase();
    if (!name) {
      setNameStatus("idle");
      return;
    }
    let cancelled = false;
    setNameStatus("checking");
    const timer = window.setTimeout(() => {
      check_bucky_username(name)
        .then((result) => {
          if (cancelled) return;
          if (result.normalized_name && result.normalized_name !== name) {
            setUsername(result.normalized_name);
          }
          setNameStatus(
            result.valid ? "ok" : result.reason === "already_exists" ? "taken" : "invalid",
          );
        })
        .catch(() => !cancelled && setNameStatus("idle"));
    }, 350);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [username]);

  useEffect(() => {
    const code = activeCode.trim();
    if (code.length < 7) {
      setActiveCodeStatus("idle");
      return;
    }
    let cancelled = false;
    setActiveCodeStatus("checking");
    const timer = window.setTimeout(() => {
      check_sn_active_code(code)
        .then((valid) => !cancelled && setActiveCodeStatus(valid ? "valid" : "invalid"))
        .catch(() => !cancelled && setActiveCodeStatus("invalid"));
    }, 350);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [activeCode]);

  const generateMaterial = async () => {
    setError(null);
    try {
      const next = await generateWebOwnerMaterial();
      const nextPositions = confirmationPositions();
      setMaterial(next);
      setPositions(nextPositions);
      setConfirmWords(["", ""]);
      setPhase("backup");
      onUpdate({
        web_owner_material: next,
        evm_address: next.evm_address,
        owner_document: null,
      });
    } catch (cause) {
      setError(errorText(t, cause));
    }
  };

  const register = async () => {
    setError(null);
    if (!material) return;
    const normalizedName = username.trim().toLowerCase();
    if (nameStatus === "taken") return fail(t("error_name_registered"));
    if (nameStatus !== "ok") return fail(t("error_name_invalid"));
    if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email.trim())) return fail(t("error_email_invalid"));
    if (!activeCode.trim()) return fail(t("invite_code_placeholder"));
    if (activeCodeStatus === "checking") return fail(t("invite_checking"));
    if (activeCodeStatus !== "valid") return fail(t("error_invite_code_invalid"));
    if (!password || password !== passwordConfirm) return fail(t("error_password_mismatch"));
    const wordsConfirmed = positions.every(
      (position, index) =>
        confirmWords[index].trim().toLowerCase() === material.mnemonic_words[position],
    );
    if (!wordsConfirmed) return fail(t("mnemonic_confirmation_failed"));

    setPhase("registering");
    try {
      const netId = get_net_id_by_gateway_type(wizardData.gatewy_type, wizardData.port_mapping_mode);
      const selectedRegion =
        registrationRegionRef.current?.name === normalizedName
          ? registrationRegionRef.current.region
          : await region.resolve(netId);
      const pwdHash = await buckyos.hashPassword(normalizedName, password);
      const ownerDocument = buildWebOwnerDocument(normalizedName, material);
      registrationRegionRef.current = { name: normalizedName, region: selectedRegion };
      const result = await registerWebOwner({
        name: normalizedName,
        email: email.trim(),
        pwdHash,
        activeCode: activeCode.trim(),
        ownerDocument,
        evmAddress: material.evm_address,
        region: selectedRegion,
      });
      onUpdate({
        sn_user_name: normalizedName,
        sn_active_code: activeCode.trim(),
        sn_access_token: result.access_token,
        sn_access_token_at: Date.now(),
        sn_refresh_token: result.refresh_token,
        admin_password_hash: pwdHash,
        owner_document: ownerDocument,
        evm_address: material.evm_address,
        web_owner_material: material,
        enabled_features: resolveEnabledFeatures(activeCode, wizardData.enabled_features),
        selected_region: selectedRegion,
      });
      setPassword("");
      setPasswordConfirm("");
      setConfirmWords(["", ""]);
      setPhase("complete");
    } catch (cause) {
      setPhase("backup");
      setError(errorText(t, cause));
    }
  };

  const backButton = (
    <Button variant="text" onClick={onBack}>
      {t("back_button")}
    </Button>
  );

  if (phase === "restore") {
    return (
      <RestoreIdentity
        initialName={username.trim().toLowerCase()}
        onCancel={() => {
          setError(null);
          setPhase(material ? "backup" : "generate");
        }}
        onRestored={(restored) => {
          onUpdate({
            sn_user_name: restored.ownerDocument.name,
            sn_active_code: "",
            sn_access_token: restored.accessToken,
            sn_access_token_at: Date.now(),
            sn_refresh_token: restored.refreshToken,
            admin_password_hash: restored.pwdHash,
            owner_document: restored.ownerDocument,
            evm_address: restored.material.evm_address,
            web_owner_material: restored.material,
            enabled_features: { llm_router: true },
            selected_region: null,
          });
          onNext();
        }}
      />
    );
  }

  if (phase === "generate") {
    return (
      <Stack spacing={3}>
        <Lead>{t("mnemonic_generate_intro")}</Lead>
        {error && <ErrorAlert {...error} />}
        <Stack direction={{ xs: "column", sm: "row" }} spacing={1.5}>
          <Button
            variant="contained"
            size="large"
            startIcon={<KeyRound size={18} strokeWidth={1.75} />}
            onClick={generateMaterial}
          >
            {t("generate_identity_material")}
          </Button>
          <Button variant="outlined" size="large" onClick={() => setPhase("restore")}>
            {t("already_registered")}
          </Button>
        </Stack>
        <StepActions back={backButton} />
      </Stack>
    );
  }

  if (phase === "complete") {
    return (
      <Stack spacing={3}>
        <Alert severity="success">{t("owner_registration_complete")}</Alert>
        <StepActions back={backButton}>
          <Button variant="contained" size="large" onClick={onNext} sx={{ minWidth: 120 }}>
            {t("next_button")}
          </Button>
        </StepActions>
      </Stack>
    );
  }

  const validAdornment = (
    <Box component="span" sx={{ display: "flex", color: "var(--cp-success)" }}>
      <Check size={18} strokeWidth={2} />
    </Box>
  );

  return (
    <Stack spacing={4}>
      <Stack spacing={2}>
        <Alert severity="warning">
          {t("mnemonic_backup_warning")}
        </Alert>
        <Box
          sx={{
            borderRadius: "12px",
            border: "1px solid var(--cp-border)",
            bgcolor: "var(--cp-surface-2)",
            overflow: "hidden",
          }}
        >
          <Box
            component="ol"
            sx={{
              listStyle: "none",
              m: 0,
              p: { xs: 1.5, sm: 2 },
              display: "grid",
              gridTemplateColumns: { xs: "repeat(2, minmax(0, 1fr))", sm: "repeat(3, minmax(0, 1fr))" },
              gap: 1,
            }}
          >
            {material?.mnemonic_words.map((word, index) => (
              <Box
                component="li"
                key={`${word}-${index}`}
                sx={{
                  display: "flex",
                  alignItems: "baseline",
                  gap: 1.5,
                  height: 40,
                  px: 1.5,
                  borderRadius: "8px",
                  border: "1px solid var(--cp-border)",
                  bgcolor: "var(--cp-surface)",
                }}
              >
                <Box component="span" sx={{ ...mono, width: 20, textAlign: "right", fontSize: 12, color: "text.secondary" }}>
                  {index + 1}
                </Box>
                <Box component="span" sx={{ ...mono, fontSize: 15, lineHeight: "38px", fontWeight: 500, color: "text.primary" }}>
                  {word}
                </Box>
              </Box>
            ))}
          </Box>
          <Box sx={{ px: 2, py: 1.5, borderTop: "1px solid var(--cp-border)" }}>
            <Typography variant="caption" color="text.secondary" component="div" sx={{ mb: 0.5 }}>
              {t("evm_asset_owner_label")}
            </Typography>
            <Typography sx={{ ...mono, fontSize: { xs: 12, sm: 13 }, overflowWrap: "anywhere", userSelect: "all" }}>
              {material?.evm_address || ""}
            </Typography>
            <Typography variant="caption" color="text.secondary" component="div" sx={{ mt: 0.5 }}>
              {t("evm_asset_owner_hint")}
            </Typography>
          </Box>
        </Box>
        <Stack direction={{ xs: "column", sm: "row" }} spacing={2}>
          {positions.map((position, index) => (
            <TextField
              key={position}
              label={t("mnemonic_word_position", { position: position + 1 })}
              value={confirmWords[index]}
              autoComplete="off"
              inputProps={{ autoCapitalize: "none", spellCheck: false }}
              onChange={(event) => {
                const next = [...confirmWords] as [string, string];
                next[index] = event.target.value;
                setConfirmWords(next);
              }}
            />
          ))}
        </Stack>
      </Stack>

      <Stack spacing={2}>
        <TextField
          label={t("username_placeholder")}
          value={username}
          onChange={(event) => setUsername(event.target.value)}
          autoComplete="username"
          inputProps={{ autoCapitalize: "none", spellCheck: false }}
          helperText={username.trim() ? `https://${username.trim().toLowerCase()}.${WEB3_BASE_HOST}` : ""}
          FormHelperTextProps={{ sx: { ...mono, fontSize: 12 } }}
          InputProps={{
            endAdornment:
              nameStatus === "checking" ? (
                <CircularProgress size={16} thickness={5} />
              ) : nameStatus === "ok" ? (
                validAdornment
              ) : undefined,
          }}
        />
        <TextField
          required
          type="email"
          autoComplete="email"
          label={t("email_placeholder")}
          value={email}
          onChange={(event) => setEmail(event.target.value)}
        />
        <TextField
          required
          label={t("invite_code_placeholder")}
          value={activeCode}
          autoComplete="off"
          onChange={(event) => {
            const nextCode = event.target.value;
            setActiveCode(nextCode);
            setActiveCodeStatus(nextCode.trim().length >= 7 ? "checking" : "idle");
          }}
          error={activeCodeStatus === "invalid"}
          helperText={
            activeCodeStatus === "checking"
              ? t("invite_checking")
              : activeCodeStatus === "invalid"
                ? t("error_invite_code_invalid")
                : activeCodeStatus === "valid"
                  ? t("invite_valid")
                  : ""
          }
          InputProps={{
            endAdornment:
              activeCodeStatus === "checking" ? (
                <CircularProgress size={16} thickness={5} />
              ) : activeCodeStatus === "valid" ? (
                validAdornment
              ) : undefined,
          }}
        />
        <Stack direction={{ xs: "column", sm: "row" }} spacing={2}>
          <TextField
            required
            type="password"
            autoComplete="new-password"
            label={t("admin_password_placeholder")}
            value={password}
            onChange={(event) => setPassword(event.target.value)}
          />
          <TextField
            required
            type="password"
            autoComplete="new-password"
            label={t("confirm_password_placeholder")}
            value={passwordConfirm}
            onChange={(event) => setPasswordConfirm(event.target.value)}
          />
        </Stack>
      </Stack>

      {error && <ErrorAlert {...error} />}
      {nameStatus === "taken" && (
        <Button
          variant="outlined"
          sx={{ alignSelf: "flex-start" }}
          onClick={() => setPhase("restore")}
          disabled={phase === "registering"}
        >
          {t("already_registered")}
        </Button>
      )}
      <StepActions
        back={
          <Stack direction="row" spacing={1}>
            <Button variant="text" onClick={onBack} disabled={phase === "registering"}>
              {t("back_button")}
            </Button>
            <Button variant="text" onClick={generateMaterial} disabled={phase === "registering"}>
              {t("regenerate_identity")}
            </Button>
          </Stack>
        }
      >
        <Button
          variant="contained"
          size="large"
          onClick={register}
          disabled={phase === "registering"}
          startIcon={phase === "registering" ? <CircularProgress size={16} thickness={5} color="inherit" /> : undefined}
          sx={{ minWidth: 120 }}
        >
          {phase === "registering" ? t("registering") : t("register_button")}
        </Button>
      </StepActions>
    </Stack>
  );
};

export default SecurityStep;
