
import './dlg/login-form';
import { i18next, updateElementAndShadowRoots} from './i18n';
import {buckyos} from 'buckyos';
import { LOGIN_EVENT, LoginEventDetail } from './utils/account';


window.addEventListener(LOGIN_EVENT, (event: CustomEvent<LoginEventDetail>) => {
    console.log("login success: ", event.detail);
    window.location.href = 'index.html';
});

window.onload = async () => {
    i18next.on('initialized', () => {
        updateElementAndShadowRoots(document);
    });

    await buckyos.initBuckyOS("control_panel");    
    
}