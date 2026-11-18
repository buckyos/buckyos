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
  SN_API_URL,
  buildWebOwnerDocument,
  check_bucky_username,
  check_sn_active_code,
  generateWebOwnerMaterial,
  registerWebOwner,
  resolveEnabledFeatures,
  waitForRegionProbe,
} from "../../../active_lib";
import { RegionProbeStatus, WebOwnerMaterial, WizardData } from "../../types";
import RegionProbePanel from "../RegionProbePanel";
import { Lead, StepActions, mono } from "../ui";

type Props = {
  wizardData: WizardData;
  onUpdate: (data: Partial<WizardData>) => void;
  onNext: () => void;
};

type NameStatus = "idle" | "checking" | "ok" | "taken" | "invalid";
type ActiveCodeStatus = "idle" | "checking" | "valid" | "invalid";
type Phase = "generate" | "backup" | "registering" | "complete";
const REGION_PREFERENCE_STORAGE_KEY = "buckyos.node_active.region_preference";

function unavailableRegionProbeStatus(): RegionProbeStatus {
  return {
    phase: "unavailable",
    region: null,
    source: "none",
    config_version: null,
    confidence: "none",
    measured_at: null,
    expires_at: null,
    available_regions: [],
    regions: [],
  };
}

function loadSavedRegionPreference(): string {
  try {
    const value = JSON.parse(localStorage.getItem(REGION_PREFERENCE_STORAGE_KEY) || "null") as {
      sn_url?: unknown;
      region?: unknown;
    } | null;
    if (
      value?.sn_url === SN_API_URL &&
      typeof value.region === "string" &&
      /^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(value.region)
    ) {
      return value.region;
    }
  } catch {
    return "auto";
  }
  return "auto";
}

function saveRegionPreference(region: string, configVersion: string | null) {
  try {
    if (region === "auto") {
      localStorage.removeItem(REGION_PREFERENCE_STORAGE_KEY);
      return;
    }
    localStorage.setItem(
      REGION_PREFERENCE_STORAGE_KEY,
      JSON.stringify({
        sn_url: SN_API_URL,
        region,
        config_version: configVersion,
      }),
    );
  } catch {
    return;
  }
}

function confirmationPositions(): [number, number] {
  const bytes = new Uint8Array(2);
  crypto.getRandomValues(bytes);
  const first = bytes[0] % 12;
  let second = bytes[1] % 12;
  if (second === first) second = (second + 5) % 12;
  return first < second ? [first, second] : [second, first];
}

const SecurityStep = ({ wizardData, onUpdate, onNext }: Props) => {
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
  const [error, setError] = useState("");
  const [regionPreference, setRegionPreference] = useState(
    wizardData.region_preference === "auto"
      ? loadSavedRegionPreference()
      : wizardData.region_preference,
  );
  const [regionProbeStatus, setRegionProbeStatus] = useState<RegionProbeStatus | null>(
    wizardData.region_probe_status,
  );
  const mountedRef = useRef(true);
  const probePromiseRef = useRef<Promise<RegionProbeStatus> | null>(null);
  const registrationRegionRef = useRef<{ name: string; region: string | null } | null>(null);

  const runRegionProbe = (force = false): Promise<RegionProbeStatus> => {
    if (!force && probePromiseRef.current) return probePromiseRef.current;
    const promise = waitForRegionProbe(force)
      .catch(() => unavailableRegionProbeStatus())
      .then((status) => {
        if (mountedRef.current) {
          setRegionProbeStatus(status);
          onUpdate({ region_probe_status: status });
        }
        return status;
      })
      .finally(() => {
        if (probePromiseRef.current === promise) probePromiseRef.current = null;
      });
    probePromiseRef.current = promise;
    return promise;
  };

  useEffect(() => {
    mountedRef.current = true;
    void runRegionProbe();
    return () => {
      mountedRef.current = false;
    };
  }, []);

  useEffect(() => {
    if (
      regionPreference !== "auto" &&
      regionProbeStatus?.config_version &&
      !regionProbeStatus.available_regions.some(
        (region) => region.region_id === regionPreference,
      )
    ) {
      setRegionPreference("auto");
      saveRegionPreference("auto", null);
      onUpdate({ region_preference: "auto" });
    }
  }, [regionPreference, regionProbeStatus]);

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

  useEffect(
    () => () => {
      setMaterial(null);
      setConfirmWords(["", ""]);
    },
    [],
  );

  const generateMaterial = async () => {
    setError("");
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
        prepared_documents: null,
        signed_documents: null,
      });
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  };

  const register = async () => {
    setError("");
    if (!material) return setError(t("error_activation_failed", "Identity material is missing"));
    const normalizedName = username.trim().toLowerCase();
    if (nameStatus === "taken") {
      return setError(
        t(
          "existing_owner_requires_wallet",
          "This name already exists. Use the BuckyOS App wallet that owns it.",
        ),
      );
    }
    if (nameStatus !== "ok") return setError(t("error_name_invalid", "Invalid username"));
    if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email.trim())) {
      return setError(t("error_email_invalid", "Enter a valid email address"));
    }
    if (!activeCode.trim()) {
      return setError(t("invite_code_placeholder", "Please enter invitation code"));
    }
    if (activeCodeStatus === "checking") {
      return setError(t("invite_checking", "Checking invitation code…"));
    }
    if (activeCodeStatus !== "valid") {
      return setError(t("error_invite_code_invalid", "Invitation code is incorrect"));
    }
    if (!password || password !== passwordConfirm) {
      return setError(t("error_password_mismatch", "Passwords do not match"));
    }
    const wordsConfirmed = positions.every(
      (position, index) =>
        confirmWords[index].trim().toLowerCase() === material.mnemonic_words[position],
    );
    if (!wordsConfirmed) {
      return setError(t("mnemonic_confirmation_failed", "Mnemonic confirmation is incorrect"));
    }

    setPhase("registering");
    try {
      let selectedRegion: string | null;
      let effectiveProbeStatus = regionProbeStatus;
      if (registrationRegionRef.current?.name === normalizedName) {
        selectedRegion = registrationRegionRef.current.region;
      } else if (regionPreference !== "auto") {
        selectedRegion = regionPreference;
      } else {
        effectiveProbeStatus = await runRegionProbe();
        selectedRegion = effectiveProbeStatus.region;
      }
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
        sn_refresh_token: result.refresh_token,
        admin_password_hash: pwdHash,
        owner_document: ownerDocument,
        evm_address: material.evm_address,
        web_owner_material: material,
        enabled_features: resolveEnabledFeatures(activeCode, wizardData.enabled_features),
        region_preference: regionPreference,
        region_probe_status: effectiveProbeStatus,
        selected_region: selectedRegion,
      });
      setPassword("");
      setPasswordConfirm("");
      setConfirmWords(["", ""]);
      setPhase("complete");
    } catch (cause) {
      setPhase("backup");
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  };

  if (phase === "generate") {
    return (
      <Stack spacing={3}>
        <Lead>
          {t(
            "mnemonic_generate_intro",
            "Generate one recovery phrase for both your Owner identity and BNS asset-owner wallet.",
          )}
        </Lead>
        {error && <Alert severity="error">{error}</Alert>}
        <StepActions>
          <Button
            variant="contained"
            size="large"
            startIcon={<KeyRound size={18} strokeWidth={1.75} />}
            onClick={generateMaterial}
          >
            {t("generate_identity_material", "Generate identity material")}
          </Button>
        </StepActions>
      </Stack>
    );
  }

  if (phase === "complete") {
    return (
      <Stack spacing={3}>
        <RegionProbePanel
          preference={regionPreference}
          status={regionProbeStatus}
          locked
          onPreferenceChange={() => undefined}
          onRetry={() => undefined}
        />
        <Alert severity="success">
          {t("owner_registration_complete", "OwnerDocument and BNS name are registered.")}
        </Alert>
        <StepActions>
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
      <RegionProbePanel
        preference={regionPreference}
        status={regionProbeStatus}
        locked={
          phase === "registering" ||
          registrationRegionRef.current?.name === username.trim().toLowerCase()
        }
        onPreferenceChange={(preference) => {
          setRegionPreference(preference);
          saveRegionPreference(preference, regionProbeStatus?.config_version || null);
          onUpdate({ region_preference: preference });
        }}
        onRetry={() => void runRegionProbe(true)}
      />

      <Stack spacing={2}>
        <Alert severity="warning">
          {t(
            "mnemonic_backup_warning",
            "Write these 12 words down in order. They are the only recovery method for this identity.",
          )}
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
              {t(
                "evm_asset_owner_hint",
                "This recoverable EVM address is the asset_owner of your BNS name.",
              )}
            </Typography>
          </Box>
        </Box>
        <Stack direction={{ xs: "column", sm: "row" }} spacing={2}>
          {positions.map((position, index) => (
            <TextField
              key={position}
              label={t("mnemonic_word_position", "Word #{{position}}", { position: position + 1 })}
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
          label={t("email_placeholder", "Email")}
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

      {error && <Alert severity="error">{error}</Alert>}
      <StepActions
        back={
          <Button variant="text" onClick={generateMaterial} disabled={phase === "registering"}>
            {t("regenerate_identity", "Regenerate")}
          </Button>
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
          {phase === "registering" ? t("registering", "Registering…") : t("register_button", "Register")}
        </Button>
      </StepActions>
    </Stack>
  );
};

export default SecurityStep;
