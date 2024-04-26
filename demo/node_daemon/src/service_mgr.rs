use crate::run_item::*;
use async_trait::async_trait;
use serde_json::Value;
pub struct ServiceItem {
    name: String,
    version: String,
    pkg_id:String,

}

impl ServiceItem {
    pub fn new(name: String, version: String, pkg_id:String) -> Self {
        ServiceItem {
            name,
            version,
            pkg_id,
        }
    }

    pub fn get_script_path(&self,script_name:&str)->Result<String>{
        //media_info = env.load_pkg(&self.name)
        //script_path = media_info.folder + "/" + script_name
        //return script_path
        unimplemented!();
    }
}

#[async_trait]
impl RunItemControl for ServiceItem {
    fn get_item_name(&self) -> String {
        self.name.clone()
    }

    async fn deploy(&self,params:Option<&RunItemParams>) -> Result<()> {
        //media_info = env.load_pkg(&self.name)
        //deploy_sh_file = media_info.folder + "/deploy.sh"
        //run_cmd(deploy_sh_file)
        Ok(())
    }

    async fn remove(&self,params:Option<&RunItemParams>) -> Result<()> {
        Ok(())
    }

    async fn update(&self,params:Option<&RunItemParams>) -> Result<String> {
        Ok(String::from("1.0.1"))
    }

    async fn start(&self,params:Option<&RunItemParams>) -> Result<()> {
        let scrpit_path = self.get_script_path("start.sh");
        //先通过环境变量设置一些参数
        //run scrpit_path 参数1，参数2
        Ok(())
    }

    async fn stop(&self,params:Option<&RunItemParams>) -> Result<()> {
        let scrpit_path = self.get_script_path("stop.sh");
        //先通过环境变量设置一些参数
        //run scrpit_path 参数1，参数2
    }

    async fn get_state(&self,params:Option<&RunItemParams>) -> Result<RunItemState> {
        //pkg_media_info= env.load_pkg(&self.pkg_id)
        //if pkg_media_info.is_none(){
        //    return RunItemState::NotExist
        //}

        let scrpit_path = self.get_script_path("get_state.sh");
        //先通过环境变量设置一些参数
        //run scrpit_path 参数1，参数2
        //根据返回值判断状态
        Ok(())
    }
}

pub async fn create_service_item_from_config(service_cfg: &str) -> Result<ServiceItem> {
    //parse servce_cfg to json
    //create ServiceItem from josn
    //return ServiceItem
    unimplemented!();
}