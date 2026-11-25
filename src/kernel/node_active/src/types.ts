import type {
  BuckyOSDeviceDocument,
  BuckyOSDeviceMiniDocument,
  BuckyOSOwnerDocument,
  BuckyOSZoneBootDocument,
  BuckyOSZoneDocument,
  Ed25519Jwk,
} from "buckyos";

export type WalletUser = {
  owner_document: BuckyOSOwnerDocument;
  sn_username: string;
  public_key?: Ed25519Jwk;
};

export type StepKey = "gateway" | "security" | "domain" | "review" | "result";

export enum GatewayType {
  BuckyForward = "BuckyForward",
  PortForward = "PortForward",
  WAN = "WAN",
}

export type JsonValue = Record<string, any>;

export type ActiveConfig = {
  sn_base_host: string;
  http_schema: "http" | "https";
};

export type EnabledFeatures = {
  llm_router: boolean;
};

export type RegionProbePhase = "idle" | "running" | "completed" | "unavailable";
export type RegionSelectionSource = "probe" | "cache" | "none";
export type RegionConfidence = "high" | "low" | "none";

export interface AvailableRegion {
  region_id: string;
  priority: number;
}

export interface RegionMeasurement {
  region_id: string;
  priority: number;
  score_ms: number | null;
  valid_urls: number;
  total_urls: number;
}

export interface RegionProbeStatus {
  phase: RegionProbePhase;
  region: string | null;
  source: RegionSelectionSource;
  config_version: string | null;
  confidence: RegionConfidence;
  measured_at: number | null;
  expires_at: number | null;
  available_regions: AvailableRegion[];
  regions: RegionMeasurement[];
}

export interface WebOwnerMaterial {
  mnemonic_words: string[];
  owner_public_jwk: Ed25519Jwk;
  owner_derivation_path: string;
  evm_address: string;
  evm_derivation_path: string;
}

export interface ActiveNameMapping {
  owner_name: string;
  owner_did: string;
  zone_did: string;
  access_hostname: string;
  bns_publish_name: string;
  use_self_domain: boolean;
}

export type NetId = "nat" | "wan_dyn" | "portmap" | "wan";

export interface GatewayTopology {
  net_id: NetId;
  rtcp_port: number;
  support_container: boolean;
  sn_url: string;
  /** Only with net_id `wan`: one IPv4, optionally one IPv6. */
  public_ips: string[];
}

export interface PreparedActiveDocuments {
  owner_document: BuckyOSOwnerDocument;
  names: ActiveNameMapping;
  topology: GatewayTopology;
  boot_document: BuckyOSZoneBootDocument;
  device_document: BuckyOSDeviceDocument;
  device_mini_document: BuckyOSDeviceMiniDocument;
  device_info: JsonValue;
}

export interface SignedActiveDocuments {
  boot_document: BuckyOSZoneBootDocument;
  boot_document_jwt: string;
  device_document: BuckyOSDeviceDocument;
  device_document_jwt: string;
  device_mini_document: BuckyOSDeviceMiniDocument;
  device_mini_document_jwt: string;
  zone_document: BuckyOSZoneDocument;
  zone_document_jwt: string;
}

export type DomainMode = "sn" | "manual" | "dns_api";
export type CertificateSource = "acme" | "upload";

export interface DomainSetup {
  domain: string;
  mode: DomainMode;
  certificate: CertificateSource;
  net_id: NetId;
  public_ips: string[];
  owner_name: string;
}

export interface DnsRecordSet {
  type: string;
  values: string[];
}

export interface DomainInspection {
  domain: string;
  zone_apex: string;
  is_apex: boolean;
  ns: string[];
  provider: string | null;
  provider_apex_cname: boolean;
  delegated_to_sn: boolean;
  bound: boolean | null;
  caa: { allowed: boolean; records: Array<{ name: string; value: string }> };
  existing: Array<{ name: string; type: string; values: string[] }>;
  recommended: DomainMode;
  blocked: Record<DomainMode, string | null>;
}

export interface DnsProviderField {
  id: string;
  label: string;
  secret: boolean;
  optional: boolean;
  multiline: boolean;
  options: Array<[string, string]>;
}

export interface DnsProviderInfo {
  id: string;
  name: string;
  fields: DnsProviderField[];
  token_url: string;
  ns_suffixes: string[];
  token_scope: "domain" | "account";
  apex_cname: boolean;
  tested: boolean;
  sandbox_url: string | null;
}

export interface PlannedRecord extends DnsRecordSet {
  id: "pkx" | "entry" | "wildcard" | "ns";
  phase: "ownership" | "routing";
  name: string;
}

export interface RecordConflict {
  name: string;
  existing: DnsRecordSet[];
  planned: DnsRecordSet;
}

export interface RecordPlan {
  records: PlannedRecord[];
  conflicts: RecordConflict[];
}

export interface RecordWriteResult {
  name: string;
  type: string;
  status: "written" | "unchanged" | "needs_confirm" | "failed";
  code: string | null;
  detail: string | null;
}

export interface RecordApplyResult {
  credential_id: string | null;
  results: RecordWriteResult[];
  conflicts: RecordConflict[];
}

export type CheckStatus = "pass" | "waiting" | "fail" | "skipped";

export interface CheckTarget {
  host: string;
  status: CheckStatus;
  code: string | null;
  expected: string[];
  actual: string[];
}

export interface CheckItem {
  id: "ownership" | "delegation" | "entry" | "challenge" | "caa" | "certificate";
  status: CheckStatus;
  code: string | null;
  expected: string[];
  actual: string[];
  detail: string | null;
  targets?: CheckTarget[];
}

export interface DomainCheck {
  ready: boolean;
  bound: boolean | null;
  checked_at: number;
  items: CheckItem[];
}

export interface CertificateCheckItem {
  code: string;
  level: "pass" | "warn" | "reject";
  reason: string | null;
  detail?: string | null;
}

export interface CertificateInspection {
  domain: string;
  items: CertificateCheckItem[];
  coverage: Array<{ name: string; covered_by: string | null }>;
  dns_names: string[];
  issuer: string | null;
  subject: string | null;
  not_before: number | null;
  not_after: number | null;
  publicly_trusted: boolean;
  node_time: number;
  fingerprint: string | null;
}

/** Everything the custom-domain step has learned; secrets are never part of it. */
export interface DomainState {
  inspection: DomainInspection | null;
  inspection_key: string;
  mode: DomainMode | null;
  certificate: CertificateSource;
  provider: string;
  credential_id: string | null;
  certificate_id: string | null;
  certificate_inspection: CertificateInspection | null;
  accept_warnings: boolean;
  check: DomainCheck | null;
  check_key: string;
}

export type ActiveWizzardData = {
  /** `full` | `lite`, from the node's installed product profile. */
  product_edition: string;
  /** Whether this node runs containers (false on BuckyOS Desktop Lite). */
  support_container: boolean;
  gatewy_type: GatewayType;
  port_mapping_mode: "full" | "rtcp_only";
  rtcp_port: number;
  /** What the user confirmed for a fixed public IP; empty for the other access routes. */
  public_ips: string[];
  use_self_domain: boolean;
  self_domain: string;
  domain: DomainState;
  sn_active_code: string | null;
  sn_user_name: string | null;
  sn_access_token: string | null;
  /** When `sn_access_token` was issued (ms); null forces a refresh before the next use. */
  sn_access_token_at: number | null;
  sn_refresh_token: string | null;
  enabled_features: EnabledFeatures;
  /** Sent with the registration made on this page; null for an account that already existed. */
  selected_region: string | null;
  admin_password_hash: string;
  friend_passcode: string;
  enable_guest_access: boolean;
  /** OS accounts this node can offer its owner's Agents; empty when it offers none. */
  host_accounts: string[];
  host_account: string;
  owner_document: BuckyOSOwnerDocument | null;
  evm_address: string;
  web_owner_material: WebOwnerMaterial | null;
  save_owner_mnemonic: boolean;
  device_public_key: Ed25519Jwk;
  device_private_key: string;
  is_wallet_runtime: boolean;
};

export type WizardData = ActiveWizzardData;
