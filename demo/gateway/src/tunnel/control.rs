/*
tunnel的控制协议

tunnel分为control和data两种类型
tunnel建立后，接收控制包如下

len: u16
package: data in json format

控制协议分为如下类型：

// 建立tunnel，tunnel建立后，client需要立刻发送build包，用以确定该tunnel的信息
{
cmd: build,
type: control,
device-id: string,
seq: uint,
}

{
cmd: build,
type: data,
device-id: string,
seq: uint,
}

// 被动端通知主动端，建立tunnel，目前应该只是数据tunnel
{
cmd: req-build,
type: data,
seq: uint
}
*/

use super::tunnel::Tunnel;
use crate::error::*;

use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use std::str::FromStr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Clone)]
pub enum ControlCmd {
    Build,
    ReqBuild,
}

impl ControlCmd {
    pub fn as_str(&self) -> &str {
        match self {
            ControlCmd::Build => "build",
            ControlCmd::ReqBuild => "req-build",
        }
    }
}

impl FromStr for ControlCmd {
    type Err = GatewayError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "build" => Ok(ControlCmd::Build),
            "req-build" => Ok(ControlCmd::ReqBuild),
            _ => Err(GatewayError::InvalidFormat(s.to_owned())),
        }
    }
}

impl std::fmt::Debug for ControlCmd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl Serialize for ControlCmd {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ControlCmd {
    fn deserialize<D>(deserializer: D) -> Result<ControlCmd, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        ControlCmd::from_str(&s).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone)]
pub enum TunnelUsage {
    Control,
    Data,
}

impl TunnelUsage {
    pub fn as_str(&self) -> &str {
        match self {
            TunnelUsage::Control => "control",
            TunnelUsage::Data => "data",
        }
    }
}

impl FromStr for TunnelUsage {
    type Err = GatewayError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "control" => Ok(TunnelUsage::Control),
            "data" => Ok(TunnelUsage::Data),
            _ => Err(GatewayError::InvalidFormat(s.to_owned())),
        }
    }
}

impl Debug for TunnelUsage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl Serialize for TunnelUsage {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for TunnelUsage {
    fn deserialize<D>(deserializer: D) -> Result<TunnelUsage, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        TunnelUsage::from_str(&s).map_err(serde::de::Error::custom)
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ControlPackage {
    pub cmd: ControlCmd,
    pub usage: TunnelUsage,
    pub device_id: Option<String>,
    pub seq: u32,
}

impl ControlPackage {
    pub fn new(cmd: ControlCmd, usage: TunnelUsage, device_id: Option<String>, seq: u32) -> Self {
        Self {
            cmd,
            usage,
            device_id,
            seq,
        }
    }

    pub fn from_json(json: &str) -> GatewayResult<Self> {
        serde_json::from_str(json).map_err(|e| {
            error!("Error parsing control package: {}", e);
            GatewayError::InvalidFormat(json.to_owned())
        })
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap()
    }
}

pub struct ControlPackageTransceiver {}

impl ControlPackageTransceiver {

    pub async fn read_package(tunnel: &mut Box<dyn Tunnel>) -> GatewayResult<ControlPackage> {
        // use read_exact to read exactly 2 bytes
        let mut len_buf = [0u8; 2];
        tunnel.read_exact(&mut len_buf).await.map_err(|e| {
            error!("Error reading control package length: {}", e);
            e
        })?;

        let len = u16::from_be_bytes(len_buf);
        let mut buf = vec![0u8; len as usize];
        tunnel.read_exact(&mut buf).await.map_err(|e| {
            error!("Error reading control package data: {}", e);
            e
        })?;

        let data = std::str::from_utf8(&buf).map_err(|e| {
            error!("Error parsing control package data: {}", e);
            GatewayError::InvalidFormat("build".to_owned())
        })?;

        let package = ControlPackage::from_json(data)?;

        Ok(package)
    }

    pub async fn write_package(
        tunnel: &mut Box<dyn Tunnel>,
        package: ControlPackage,
    ) -> GatewayResult<()> {
        let data = package.to_json();
        let len = data.len() as u16;
        let len_buf = len.to_be_bytes();
        tunnel.write_all(&len_buf).await.map_err(|e| {
            error!("Error writing control package length: {}", e);
            e
        })?;

        tunnel.write_all(data.as_bytes()).await.map_err(|e| {
            error!("Error writing control package data: {}", e);
            e
        })?;

        Ok(())
    }
}
