import {buckyos} from 'buckyos';

interface UserSambaInfo {
    is_enable: boolean;
    password: string;
}

window.onload = async () => {

    await buckyos.initBuckyOS("control_panel");
    let account_info = await buckyos.login(true);
    if (account_info == null) {
        alert("请先登录");
        window.location.href = "./login_index.html";
        return;
    }
    


    const source_url = document.referrer;
    const parsedUrl = new URL(window.location.href);
    var url_appid:string|null = parsedUrl.searchParams.get('client_id');
    console.log("url_appid: ", url_appid);


    if (url_appid == null) {
        alert("client_id(appid) is null");
        window.close();
        return;
    }

    let token = localStorage.getItem("token");
    let username = localStorage.getItem("username");
    if (token == undefined || token == "" || username == undefined || username == "") {
        //跳转到登录页
        window.location.href = "./login.html?client_id=" + url_appid;
    }



    let system_config_client = buckyos.getServiceRpcClient("system_config");
    let samba_info: UserSambaInfo | null = null;
    try {
        samba_info = JSON.parse(await system_config_client.call("sys_config_get", {"key": `users/${username}/samba/settings`}));
    } catch (e) {
    }
    if (samba_info == null) {
        samba_info = {
            is_enable: false,
            password: ""
        };
    }

    const checkbox = document.getElementById('chk-samba');
    const passwordInput = document.getElementById('txt-samba-password');
    const setButton = document.getElementById('btn-set-samba-password');

    checkbox!.checked = samba_info.is_enable;
    passwordInput!.diabled = !samba_info.is_enable;
    passwordInput!.value = samba_info.password;
    // 监听checkbox状态变化
    checkbox!.addEventListener('change', function() {
        passwordInput!.disabled = !this.checked; // 根据checkbox状态启
    });

    setButton!.addEventListener('click', async () => {
        let password = (document.getElementById('txt-samba-password') as HTMLInputElement).value;
        let is_enable = (document.getElementById('chk-samba') as HTMLInputElement).checked;
        if (is_enable) {
            if (password == undefined || password == "") {
                alert("password is null");
                return;
            }
        } else {
            password = "";
        }
        try {
            await system_config_client.call("sys_config_set", {"key": `users/${username}/samba/settings`, "value": JSON.stringify({"is_enable": is_enable, "password": password})});
            alert("Set success");
        } catch (e) {
            alert("Set failed");
        }
    });
};
