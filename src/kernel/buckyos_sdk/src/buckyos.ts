

export class BuckyOS {
    constructor({ ssoUrl, clientId, redirectUri, tokenKey = 'sso_token', useCookie = false, cookieOptions = null }) {
        this.ssoUrl = ssoUrl;
        this.clientId = clientId;
        this.redirectUri = redirectUri;
        this.tokenKey = tokenKey;
        this.useCookie = useCookie;
        this.cookieOptions = cookieOptions;
        this.authWindow = null;
        this.token = this.loadToken();
    }

    async login(parms) {
        if (this.token) {
            return this.token;
        }

        try {
            const token = await this._openAuthWindow();
            this.token = token;
            this.saveToken(token);
            return token;
        } catch (error) {
            throw new Error(error || 'Login failed');
        }
    }

    _openAuthWindow() {
        return new Promise((resolve, reject) => {
            const width = 500;
            const height = 600;
            const left = (window.screen.width / 2) - (width / 2);
            const top = (window.screen.height / 2) - (height / 2);

            const authUrl = `${this.ssoUrl}?client_id=${this.clientId}&redirect_uri=${encodeURIComponent(this.redirectUri)}&response_type=token`;

            this.authWindow = window.open(authUrl, 'SSO Login', `width=${width},height=${height},top=${top},left=${left}`);

            window.addEventListener('message', (event) => {
                if (event.origin !== new URL(this.ssoUrl).origin) {
                    return;
                }

                const { token, error } = event.data;

                if (token) {
                    resolve(token);
                } else {
                    reject(error || 'Login failed');
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

    saveToken(token) {
        if (this.useCookie) {
            this.setCookie(this.tokenKey, token, this.cookieOptions);
        } else {
            localStorage.setItem(this.tokenKey, token);
        }
    }

    loadToken() {
        if (this.useCookie) {
            return this.getCookie(this.tokenKey);
        } else {
            return localStorage.getItem(this.tokenKey);
        }
    }

    logout() {
        this.token = null;
        if (this.useCookie) {
            this.deleteCookie(this.tokenKey);
        } else {
            localStorage.removeItem(this.tokenKey);
        }
    }

    // Utility function to set a cookie
    setCookie(name, value, options = {}) {
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
