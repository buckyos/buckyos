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

export type StepKey =
  | "gateway"
  | "domain"
  | "security"
  | "ai_provider"
  | "jarvis_msg_tunnel"
  | "review"
  | "success";

export enum GatewayType {
  BuckyForward = "BuckyForward",
  PortForward = "PortForward",
  WAN = "WAN",
}

export type JsonValue = Record<string, any>;

export type ActiveConfig = {
  sn_base_host: string;
  http_schema: "http" | "https";
  ai_provider_tutorial_url?: string;
  telegram_bot_api_token_tutorial_url?: string;
  telegram_account_id_tutorial_url?: string;
};

export type AIProviderConfig = {
  openai_api_token: string;
  claude_api_token: string;
  google_api_token: string;
  openrouter_api_token: string;
  glm_api_token: string;
};

export type JarvisMsgTunnelConfig = {
  telegram_bot_api_token: string;
  telegram_account_id: string;
};

export type EnabledFeatures = {
  llm_router: boolean;
};

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

export interface GatewayTopology {
  net_id: "nat" | "wan_dyn" | "portmap" | "wan";
  rtcp_port: number;
  support_container: boolean;
  uses_sn_relay: boolean;
  sn_url: string;
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

export type DomainBindingState =
  | { state: "unused" }
  | { state: "checking"; domain: string }
  | {
      state: "challenge";
      domain: string;
      record_name: string;
      value: string;
      reason: string;
    }
  | { state: "verified"; domain: string; verified_at: number };

export type ActiveWizzardData = {
  gatewy_type: GatewayType;
  port_mapping_mode: "full" | "rtcp_only";
  rtcp_port: number;
  use_self_domain: boolean;
  self_domain: string;
  domain_binding: DomainBindingState;
  sn_active_code: string | null;
  sn_user_name: string | null;
  sn_access_token: string | null;
  sn_refresh_token: string | null;
  enabled_features: EnabledFeatures;
  admin_password_hash: string;
  friend_passcode: string;
  enable_guest_access: boolean;
  owner_document: BuckyOSOwnerDocument | null;
  evm_address: string;
  web_owner_material: WebOwnerMaterial | null;
  device_public_key: Ed25519Jwk;
  device_private_key: string;
  prepared_documents: PreparedActiveDocuments | null;
  signed_documents: SignedActiveDocuments | null;
  is_wallet_runtime: boolean;
  ai_provider_config: AIProviderConfig;
  jarvis_msg_tunnel_config: JarvisMsgTunnelConfig;
};

export type WizardData = ActiveWizzardData;
