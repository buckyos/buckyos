import "./components/bs-title-bar";
import {i18next, updateElementAndShadowRoots} from './i18n';
import {buckyos} from 'buckyos';
import { LOGIN_EVENT, LoginEventDetail, get_session_account_info } from './utils/account';
import "./dlg/app_setting";
import { get_app_list } from './utils/app_mgr';

//setting.html?item=$setting1_setting2_setting3&param1=value1&param2=value2

async function show_default_page() {
    console.log("show_default_page");
    let content_div = document.getElementById("main-content");
    if (content_div == null) {
        console.error("content_div is null");
        return;
    }
    
    content_div.innerHTML = "";
    let appSettingDialog = document.createElement('app-setting-dialog');
    appSettingDialog.id = "app-setting-dialog";

    let apps = await get_app_list();
    appSettingDialog.setApps(apps);
    content_div.appendChild(appSettingDialog);
}

function show_sub_setting_page(setting_id: string) {
    console.log("show_setting_page: ", setting_id);
}

function show_setting_page(setting_id: string | null,full_url_string: string,need_update_url: boolean = false) {
    console.log("show_setting_page: ", setting_id,full_url_string); 
    if (need_update_url) {
        //更新url
        window.history.pushState({}, "", full_url_string);
    }

    if(setting_id == null) {
        //显示默认页面
        show_default_page();
    } else {
        //显示指定页面
        show_sub_setting_page(setting_id);
    }
}


window.onload = async () => {
    console.log("setting.ts onload");
    updateElementAndShadowRoots(document);
    await buckyos.initBuckyOS("control_panel");
    //判断登陆状态，如果未登录，则跳转到登录页面
    let account_info = await buckyos.login(true);
    if (account_info == null) {
        console.log("account_info is null, will redirect to login page");
        alert("请先登录");
        window.location.href = "./login_index.html";
        return;
    }
    //读取页面的访问参数 ($setting=setting_id),直接显示合适的配置页面
    let url_params = new URLSearchParams(window.location.search);
    let setting_id = url_params.get("setting");
    show_setting_page(setting_id,window.location.href,false);
   
}
