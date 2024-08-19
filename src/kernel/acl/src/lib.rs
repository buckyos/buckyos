
#![allow(dead_code)]
#![allow(unused)]



//use default acl config to enforce the access control
//default acl config is stored in the memory,so it is not async function
pub fn enforce(userid:&str,res_path:&str,op_name:&str) -> bool {
    //get default enforcer obj


    return false;
}


//test
#[cfg(test)]
mod tests {
    use super::*;
    use casbin::prelude::*;
    use casbin::{CoreApi, MemoryAdapter};
    #[test]
    fn test_enforce() {
        ()
    }

}