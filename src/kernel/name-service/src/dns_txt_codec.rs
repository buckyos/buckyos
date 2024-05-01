use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use crate::{NSErrorCode, NSResult};
use crate::error::{into_ns_err, ns_err};

pub struct DnsTxtCodec;

impl DnsTxtCodec {
    pub fn encode(data: &[u8], split_len: usize) -> NSResult<Vec<String>> {
        let base64_str = STANDARD.encode(data);
        if base64_str.len() < split_len {
            return Ok(vec![base64_str]);
        }

        let mut result = Vec::new();
        let mut start = 0;
        if split_len < 10 {
            ns_err!(NSErrorCode::DnsTxtEncodeError, "split_len must be greater than 10");
        }
        let split_len = split_len - 10;
        let mut seq = 1;
        while start < base64_str.len() {
            let end = if start + split_len > base64_str.len() {
                base64_str.len()
            } else {
                start + split_len
            };
            let txt = &base64_str[start..end];
            result.push(format!("seg{:0>3}:{}", seq, txt));
            seq += 1;
            start = end;
        }
        Ok(result)
    }

    pub fn decode(mut txts: Vec<String>) -> NSResult<Vec<u8>> {
        let base64_str = if txts.len() == 1 {
            txts[0].clone()
        } else {
            txts.sort();
            let mut base64_str = String::new();
            for (seq, txt) in txts.iter().enumerate() {
                if txt.starts_with(format!("seg{:0>3}:", seq + 1).as_str()) {
                    let mut parts = txt.split(':');
                    let _ = parts.next().unwrap();
                    base64_str.push_str(parts.next().unwrap());
                }
            }
            base64_str
        };
        let data = STANDARD.decode(&base64_str).map_err(into_ns_err!(NSErrorCode::Failed, "Failed to decode base64"))?;
        Ok(data)
    }
}

#[test]
fn test_dns_txt_codec() {
    let test = "lksjdfljaseirouasdodnfkasjdlfjwo3riuoaiusdfoajsdlfkjawleiraosduifgoasdkfjlaksdjflasdjfl";
    let mut chunk_list = DnsTxtCodec::encode(test.as_bytes(), 20).unwrap();
    let t = chunk_list[0].clone();
    let t2 = chunk_list[2].clone();
    chunk_list[0] = t2;
    chunk_list[2] = t;
    let data = DnsTxtCodec::decode(chunk_list).unwrap();
    DnsTxtCodec::decode(vec![]).unwrap();
    let result = String::from_utf8_lossy(&data).to_string();

    assert_eq!(result, test);
}
