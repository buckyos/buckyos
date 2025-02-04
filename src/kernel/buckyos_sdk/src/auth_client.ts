import jsSHA from 'jssha';

export class AuthClient {
    zone_base_url:string;
    clientId:string;
    cookieOptions:any;
    authWindow:Window | null;
    token:string | null;

    constructor(zone_base_url:string, appId:string, token:string|null) {
        this.zone_base_url = zone_base_url;
        //this.appId = appId;
        this.clientId = appId;
        this.authWindow = null;
        this.token = token
    }

    static async hash_password(username:string,password:string,nonce:number|null=null):Promise<string> {
        const shaObj = new jsSHA("SHA-256", "TEXT", { encoding: "UTF8" });
        shaObj.update(password+username+".buckyos");
        let org_password_hash_str = shaObj.getHash("B64");
        if (nonce == null) {
            return org_password_hash_str;
        }
        const shaObj2 = new jsSHA("SHA-256", "TEXT", { encoding: "UTF8" });
        let salt = org_password_hash_str + nonce.toString();
        shaObj2.update(salt);
        let result = shaObj2.getHash("B64");
        return result;
    }

    async login(redirect_uri:string|null=null) {
        if (this.token) {
            return this.token;
        }

        try {
            const token = await this._openAuthWindow(redirect_uri);
            this.token = token;
            return token;
        } catch (error) {
            throw new Error(error || 'Login failed');
        }
    }

    async request(action:string,params:any){
        //let token = await this.login();
        //return token;
    }

    async _openAuthWindow(redirect_uri:string) : Promise<string> {
        return new Promise((resolve, reject) => {
            const width = 500;
            const height = 600;
            const left = (window.screen.width / 2) - (width / 2);
            const top = (window.screen.height / 2) - (height / 2);
            let sso_url = "http://sys." + this.zone_base_url + "/login.html";
            console.log("sso_url: ", sso_url);
            
            const authUrl = `${sso_url}?client_id=${this.clientId}&redirect_uri=${encodeURIComponent(redirect_uri)}&response_type=token`;
            alert(authUrl);
            this.authWindow = window.open(authUrl, 'BuckyOS Login', `width=${width},height=${height},top=${top},left=${left}`);

            //TODO: how to get this message?
            window.addEventListener('message', (event) => {
                console.log("message event",event);
                if (event.origin !== new URL(sso_url).origin) {
                    return;
                }

                const { token, error } = event.data;

                if (token) {
                    resolve(token);
                } else {
                    reject(error || 'BuckyOSLogin failed');
                }

                if (this.authWindow) {
                    this.authWindow.close();
                }
            }, false);
        });
    }

    getToken() {
        return this.token;
    }

    logout() {
        this.token = null;
        if (this.useCookie) {
            this.deleteCookie(this.token);
        } else {
            localStorage.removeItem(this.token!);
        }
    }

    // Utility function to set a cookie
    setCookie(name: string, value: string, options: any) {
        let cookieString = `${encodeURIComponent(name)}=${encodeURIComponent(value)}`;

        if (options.expires) {
            const expires = new Date(options.expires);
            cookieString += `; expires=${expires.toUTCString()}`;
        }
        if (options.path) {
            cookieString += `; path=${options.path}`;
        }
        if (options.domain) {
            cookieString += `; domain=${options.domain}`;
        }
        if (options.secure) {
            cookieString += `; secure`;
        }
        if (options.httpOnly) {
            cookieString += `; HttpOnly`; // Note: HttpOnly can't be set via JS, it's just for reference.
        }

        document.cookie = cookieString;
    }

    // Utility function to get a cookie
    getCookie(name) {
        const matches = document.cookie.match(new RegExp(
            `(?:^|; )${encodeURIComponent(name)}=([^;]*)`
        ));
        return matches ? decodeURIComponent(matches[1]) : null;
    }

    // Utility function to delete a cookie
    deleteCookie(name) {
        this.setCookie(name, '', { expires: 'Thu, 01 Jan 1970 00:00:00 GMT', path: '/' });
    }
}
