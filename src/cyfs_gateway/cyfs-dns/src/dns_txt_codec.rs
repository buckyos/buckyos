use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use crate::{NSErrorCode, NSResult};
use crate::error::{into_ns_err, ns_err};

pub struct DnsTxtCodec;

impl DnsTxtCodec {
    pub fn encode(data: &str, split_len: usize) -> NSResult<Vec<String>> {
        if data.len() < split_len {
            return Ok(vec![data.to_string()]);
        }

        let mut result = Vec::new();
        let mut start = 0;
        if split_len < 10 {
            ns_err!(NSErrorCode::DnsTxtEncodeError, "split_len must be greater than 10");
        }
        let split_len = split_len - 10;
        let mut seq = 1;
        while start < data.len() {
            let end = if start + split_len > data.len() {
                data.len()
            } else {
                start + split_len
            };
            let txt = &data[start..end];
            result.push(format!("seg{:0>3}:{}", seq, txt));
            seq += 1;
            start = end;
        }
        Ok(result)
    }

    pub fn decode(mut txts: Vec<String>) -> NSResult<String> {
        let data = if txts.len() == 1 {
            txts[0].clone()
        } else {
            txts.sort();
            let mut data = String::new();
            for (seq, txt) in txts.iter().enumerate() {
                if txt.starts_with(format!("seg{:0>3}:", seq + 1).as_str()) {
                    let mut parts = txt.split(':');
                    let _ = parts.next().unwrap();
                    data.push_str(parts.next().unwrap());
                }
            }
            data
        };
        Ok(data)
    }
}

#[test]
fn test_dns_txt_codec() {
    let test = "lksjdfljaseirouasdodnfkasjdlfjwo3riuoaiusdfoajsdlfkjawleiraosduifgoasdkfjlaksdjflasdjfl";
    let mut chunk_list = DnsTxtCodec::encode(test, 20).unwrap();
    let t = chunk_list[0].clone();
    let t2 = chunk_list[2].clone();
    chunk_list[0] = t2;
    chunk_list[2] = t;
    let result = DnsTxtCodec::decode(chunk_list).unwrap();
    DnsTxtCodec::decode(vec![]).unwrap();

    assert_eq!(result, test);
}
