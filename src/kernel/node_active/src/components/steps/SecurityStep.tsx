import {
  CheckCircleRounded,
  KeyRounded,
  VerifiedRounded,
} from "@mui/icons-material";
import {
  Alert,
  Box,
  Button,
  Chip,
  CircularProgress,
  Paper,
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

type Props = {
  wizardData: WizardData;
  onUpdate: (data: Partial<WizardData>) => void;
  onNext: () => void;
};

type NameStatus = "idle" | "checking" | "ok" | "taken" | "invalid";
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
  const [activeCodeValid, setActiveCodeValid] = useState<boolean | null>(null);
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
      setActiveCodeValid(null);
      return;
    }
    let cancelled = false;
    const timer = window.setTimeout(() => {
      check_sn_active_code(code)
        .then((valid) => !cancelled && setActiveCodeValid(valid))
        .catch(() => !cancelled && setActiveCodeValid(false));
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
    if (!activeCode.trim() || activeCodeValid === false) {
      return setError(t("invite_code_placeholder", "Enter a valid invitation code"));
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
        <Alert severity="info">
          {t(
            "mnemonic_generate_intro",
            "Generate one recovery phrase for both your Owner identity and BNS asset-owner wallet.",
          )}
        </Alert>
        <Button variant="contained" size="large" startIcon={<KeyRounded />} onClick={generateMaterial}>
          {t("generate_identity_material", "Generate identity material")}
        </Button>
        {error && <Alert severity="error">{error}</Alert>}
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
        <Alert severity="success" icon={<CheckCircleRounded />}>
          {t("owner_registration_complete", "OwnerDocument and BNS name are registered.")}
        </Alert>
        <Button variant="contained" size="large" onClick={onNext}>
          {t("next_button")}
        </Button>
      </Stack>
    );
  }

  return (
    <Stack spacing={3}>
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
      <Alert severity="warning">
        {t(
          "mnemonic_backup_warning",
          "Write these 12 words down in order. They are the only recovery method for this identity.",
        )}
      </Alert>
      <Paper variant="outlined" sx={{ p: 2.5, borderRadius: 3 }}>
        <Stack spacing={2}>
          <Stack direction="row" flexWrap="wrap" gap={1}>
            {material?.mnemonic_words.map((word, index) => (
              <Chip key={`${word}-${index}`} label={`${index + 1}. ${word}`} />
            ))}
          </Stack>
          <Typography variant="body2" color="text.secondary">
            {t(
              "evm_asset_owner_hint",
              "This recoverable EVM address is the asset_owner of your BNS name.",
            )}
          </Typography>
          <TextField label="EVM asset_owner" value={material?.evm_address || ""} InputProps={{ readOnly: true }} />
        </Stack>
      </Paper>

      <Stack direction={{ xs: "column", sm: "row" }} spacing={2}>
        {positions.map((position, index) => (
          <TextField
            key={position}
            fullWidth
            label={t("mnemonic_word_position", "Word #{{position}}", { position: position + 1 })}
            value={confirmWords[index]}
            onChange={(event) => {
              const next = [...confirmWords] as [string, string];
              next[index] = event.target.value;
              setConfirmWords(next);
            }}
          />
        ))}
      </Stack>

      <TextField
        label={t("username_placeholder")}
        value={username}
        onChange={(event) => setUsername(event.target.value)}
        helperText={username.trim() ? `https://${username.trim().toLowerCase()}.${WEB3_BASE_HOST}` : ""}
        InputProps={{
          endAdornment:
            nameStatus === "checking" ? (
              <CircularProgress size={18} />
            ) : nameStatus === "ok" ? (
              <VerifiedRounded color="success" />
            ) : undefined,
        }}
      />
      <TextField
        required
        type="email"
        label={t("email_placeholder", "Email")}
        value={email}
        onChange={(event) => setEmail(event.target.value)}
      />
      <TextField
        required
        label={t("invite_code_placeholder")}
        value={activeCode}
        onChange={(event) => setActiveCode(event.target.value)}
        InputProps={{
          endAdornment: activeCodeValid ? <VerifiedRounded color="success" /> : undefined,
        }}
      />
      <Stack direction={{ xs: "column", sm: "row" }} spacing={2}>
        <TextField
          fullWidth
          required
          type="password"
          label={t("admin_password_placeholder")}
          value={password}
          onChange={(event) => setPassword(event.target.value)}
        />
        <TextField
          fullWidth
          required
          type="password"
          label={t("confirm_password_placeholder")}
          value={passwordConfirm}
          onChange={(event) => setPasswordConfirm(event.target.value)}
        />
      </Stack>
      {error && <Alert severity="error">{error}</Alert>}
      <Stack direction="row" justifyContent="space-between">
        <Button variant="text" onClick={generateMaterial} disabled={phase === "registering"}>
          {t("regenerate_identity", "Regenerate")}
        </Button>
        <Button
          variant="contained"
          size="large"
          onClick={register}
          disabled={phase === "registering"}
          startIcon={phase === "registering" ? <CircularProgress size={18} /> : undefined}
        >
          {phase === "registering" ? t("registering", "Registering…") : t("register_button", "Register")}
        </Button>
      </Stack>
    </Stack>
  );
};

export default SecurityStep;
