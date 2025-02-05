import {buckyos} from 'buckyos';
// 定义事件类型
export interface LoginEventDetail {
    login_result: any;
    timestamp: number;
}

  
// 定义自定义事件
export const LOGIN_EVENT = 'onLogin';

export function get_session_account_info(): AccountInfo | null {
    let account_info = localStorage.getItem("account_info");
    if (account_info == null) {
        return null;
    }

    return JSON.parse(account_info) as AccountInfo;
}