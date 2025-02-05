
// 定义事件类型
export interface LoginEventDetail {
    login_result: any;
    timestamp: number;
}

  
// 定义自定义事件
export const LOGIN_EVENT = 'onLogin';
