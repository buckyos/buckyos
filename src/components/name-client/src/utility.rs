use crate::NSResult;
use crate::NSError;

/// 从完整域名中提取一级域名
/// 
/// # Arguments
/// * `name` - 完整域名
/// * `known_domains` - 已知的域名列表,如果匹配到直接返回
pub fn extract_root_domain(name: &str, known_domains: &[String]) -> NSResult<String> {
    // 1. 先检查是否匹配已知域名
    for domain in known_domains {
        if name == domain || name.ends_with(&format!(".{}", domain)) {
            return Ok(domain.clone());
        }
    }

    // 2. 如果没有匹配到,走原来的解析逻辑
    let parts: Vec<&str> = name.rsplitn(3, '.').collect();
    if parts.len() >= 2 {
        let mut domain = format!("{}.{}", parts[1], parts[0]);
        
        if parts.len() == 3 && SPECIAL_TLDS.contains(&format!("{}.{}", parts[1], parts[0]).as_str()) {
            domain = format!("{}.{}.{}", parts[2], parts[1], parts[0]);
        }
        
        Ok(domain)
    } else {
        Err(NSError::Failed("无效的域名格式".to_string()))
    }
}

/// 特殊的顶级域名列表
const SPECIAL_TLDS: &[&str] = &[
    // 英国
    "co.uk", "com.uk", "org.uk", "net.uk", "me.uk", "ltd.uk", "plc.uk", 
    "sch.uk", "nhs.uk", "ac.uk", "gov.uk", "mod.uk", "police.uk",
    
    // 中国
    "com.cn", "org.cn", "net.cn", "gov.cn", "edu.cn", "ac.cn", "mil.cn",
    "gd.cn", "bj.cn", "sh.cn", "tj.cn", "cq.cn", "he.cn", "sx.cn", "nm.cn",
    "ln.cn", "jl.cn", "hl.cn", "js.cn", "zj.cn", "ah.cn", "fj.cn", "jx.cn",
    "sd.cn", "ha.cn", "hb.cn", "hn.cn", "gd.cn", "gx.cn", "hi.cn", "sc.cn",
    "gz.cn", "yn.cn", "xz.cn", "sn.cn", "gs.cn", "qh.cn", "nx.cn", "xj.cn",
    
    // 澳大利亚
    "com.au", "net.au", "org.au", "edu.au", "gov.au", "asn.au", "id.au",
    "info.au", "conf.au", "act.au", "nsw.au", "nt.au", "qld.au", "sa.au",
    "tas.au", "vic.au", "wa.au",
    
    // 日本
    "co.jp", "ne.jp", "ac.jp", "go.jp", "or.jp", "ad.jp", "ed.jp",
    "gr.jp", "lg.jp", "geo.jp",
    
    // 韩国
    "co.kr", "ne.kr", "or.kr", "re.kr", "pe.kr", "go.kr", "mil.kr",
    "ac.kr", "hs.kr", "ms.kr", "es.kr", "sc.kr", "kg.kr", "seoul.kr",
    "busan.kr", "daegu.kr", "incheon.kr", "gwangju.kr", "daejeon.kr", "ulsan.kr",
    
    // 台湾
    "com.tw", "org.tw", "gov.tw", "edu.tw", "net.tw", "idv.tw",
    "game.tw", "ebiz.tw", "club.tw", "mil.tw",
    
    // 香港
    "com.hk", "org.hk", "edu.hk", "gov.hk", "net.hk", "idv.hk",
    
    // 新加坡
    "com.sg", "org.sg", "edu.sg", "gov.sg", "net.sg", "per.sg"
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_root_domain() {
        // 测试普通二级域名
        assert_eq!(
            extract_root_domain("www.example.com", &[]).unwrap(),
            "example.com"
        );
        
        // 测试特殊三级域名
        assert_eq!(
            extract_root_domain("test.example.co.uk", &[]).unwrap(),
            "example.co.uk"
        );
        
        // 测试中国域名
        assert_eq!(
            extract_root_domain("www.example.com.cn", &[]).unwrap(),
            "example.com.cn"
        );
        
        // 测试无效域名
        assert!(extract_root_domain("invalid", &[]).is_err());
    }
}
