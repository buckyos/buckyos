import { ActiveWizzardData, GatewayType, SN_API_URL, WEB3_BASE_HOST } from "../active_lib";

export type WizardData = ActiveWizzardData & {
  port_mapping_mode: "full" | "rtcp_only";
  rtcp_port?: number;
};

export type StepKey = "gateway" | "domain" | "security" | "review" | "success";

export const createInitialWizardData = (): WizardData => ({
  gatewy_type: GatewayType.BuckyForward,
  is_direct_connect: false,
  sn_active_code: "",
  sn_user_name: "",
  sn_url: SN_API_URL,
  web3_base_host: WEB3_BASE_HOST,
  use_self_domain: false,
  self_domain: "",
  admin_password_hash: "",
  friend_passcode: "",
  enable_guest_access: false,
  owner_public_key: "",
  owner_private_key: "",
  zone_config_jwt: "",
  port_mapping_mode: "full",
  rtcp_port: 2980,
});

export { GatewayType };
