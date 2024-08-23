
#![allow(dead_code)]
#![allow(unused)]



//use default RBAC config to enforce the access control
//default acl config is stored in the memory,so it is not async function
pub fn enforce(userid:&str, appid:&str,res_path:&str,op_name:&str) -> bool {
    //get default enforcer obj


    return false;
}




//test
#[cfg(test)]
mod tests {
    use super::*;
    use tokio::test;
    use casbin::prelude::*;
    #[test]
    async fn test_enforce() {
        let mut e = Enforcer::new("examples/rbac_with_domains_model.conf", "examples/rbac_with_domains_policy.csv").await;
        //e.unwrap().enable_log(true);
        let en = e.unwrap();
    
        let r = en.enforce(("alice", "domain1", "data1/", "read"));
        assert_eq!(r.unwrap(), true);
    }

}