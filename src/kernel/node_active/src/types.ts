

export type WalletUser = {
  user_name: string;
  user_id?: string;
  public_key?:  JsonValue;
  sn_username?: string;
};


export type StepKey = "gateway" | "domain" | "security" | "review" | "success";

export enum GatewayType {
  BuckyForward = "BuckyForward",//nat  (none)
  PortForward = "PortForward",//port forward (portmap)
  //WANDynamic = "WANDynamic",//dynamic wan (wan_dyn)
  WAN = "WAN",//static wan (wan)
}

export type JsonValue = Record<string, any>;

export type ActiveConfig = {
sn_base_host: string;
http_schema: "http" | "https";
};

export type ActiveWizzardData = {
  gatewy_type: GatewayType;
  // is_direct_connect: boolean;
  port_mapping_mode: "full" | "rtcp_only";
  rtcp_port: number;

  use_self_domain: boolean;
  self_domain: string;
  zone_config_jwt: string;

  sn_url: string;
  web3_base_host: string;
  sn_active_code: string | null;//钱包模式为null
  sn_user_name: string | null;//钱包模式不会为null

  admin_password_hash: string;
  friend_passcode: string;
  enable_guest_access: boolean;

  owner_public_key: JsonValue;
  owner_private_key: string | null;//钱包模式为null

  device_public_key: JsonValue;
  device_private_key: string;

  is_wallet_runtime: boolean;
  owner_user_name: string;//did:bns:$owner_user_name

}

// 类型别名：用于组件中的向导数据，与 ActiveWizzardData 相同
export type WizardData = ActiveWizzardData;


