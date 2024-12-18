
use std::ops::Range;
use serde::{Serialize,Deserialize};
use crate::{ObjId,ChunkId,NdnResult,NdnError};


#[derive(Debug, Clone,Eq, PartialEq)]
pub enum LinkData {
    SameAs(ObjId),//Same ChunkId
    //ComposedBy(ChunkId,ObjMapId),// Base ChunkId + Diff Action Items
    PartOf(ChunkId,Range<u64>), //Object Id + Range
    //IndexOf(ObjId,u64),//Object Id + Index
}

impl Serialize for LinkData {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for LinkData {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        LinkData::from_string(&s).map_err(serde::de::Error::custom)
    }
}

impl LinkData {
    pub fn to_string(&self)->String {
        match self {
            LinkData::SameAs(obj_id) => format!("same->{}",obj_id.to_string()),
            LinkData::PartOf(chunk_id,range) => {
                let range_str = format!("{}..{}",range.start,range.end);
                format!("part_of->{}@{}",range_str,chunk_id.to_string())
            }
        }
    }

    pub fn from_string(link_str:&str)->NdnResult<Self> {
        let parts = link_str.split("->").collect::<Vec<&str>>();
        if parts.len() != 2 {
            return Err(NdnError::InvalidLink(format!("invalid link string:{}",link_str)));
        }
        let link_type = parts[0];
        let link_data = parts[1];

        match link_type {
            "same" => Ok(LinkData::SameAs(ObjId::new(link_data)?)),
            "part_of" => {
                let parts = link_data.split("@").collect::<Vec<&str>>();
                if parts.len() != 2 {
                    return Err(NdnError::InvalidLink(format!("invalid link string:{}",link_str)));
                }
                let range = parts[0].split("..").collect::<Vec<&str>>();
                if range.len() != 2 {
                    return Err(NdnError::InvalidLink(format!("invalid range string:{}",parts[1])));
                }
                let start = range[0].parse::<u64>().unwrap();
                let end = range[1].parse::<u64>().unwrap();
                Ok(LinkData::PartOf(ChunkId::new(parts[1])?,Range{start,end}))
            }
            _ => Err(NdnError::InvalidLink(format!("invalid link type:{}",link_type))),
        }
    }
}

pub struct ObjectLink {
    pub source:ObjId,
    pub link_data:LinkData,
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_link_data() {
        let link_data = LinkData::SameAs(ObjId::new("test:123").unwrap());
        let link_str = link_data.to_string();
        println!("link_str {}",link_str);
        let link_data2 = LinkData::from_string(&link_str).unwrap();
        assert_eq!(link_data,link_data2);

        let chunk_id = ChunkId::new("sha256:1234567890").unwrap();
        let link_data = LinkData::PartOf(chunk_id,Range{start:0,end:100});
        let link_str = link_data.to_string();
        println!("link_str {}",link_str);
        let link_data2 = LinkData::from_string(&link_str).unwrap();
        assert_eq!(link_data,link_data2);
    }
}