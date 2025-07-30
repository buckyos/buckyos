import { AsyncDatabase } from "promised-sqlite3";

export class Storage {
    private db: AsyncDatabase|undefined;
    private dbPath: string;
    
    constructor(dbPath: string) {
        this.db = undefined;
        this.dbPath = dbPath;
    }

    public async init() {
        this.db = await AsyncDatabase.open(this.dbPath)
        await this.db.run(`CREATE TABLE IF NOT EXISTS "users" (
            "username"	TEXT NOT NULL,
            "private_key"	TEXT NOT NULL,
            PRIMARY KEY("username")
        )`);

        await this.db.run(`CREATE TABLE IF NOT EXISTS "versions" (
            "version"	TEXT NOT NULL,
            "os"	TEXT NOT NULL,
            "arch"	TEXT NOT NULL,
            "tested"	INTEGER NOT NULL DEFAULT 0,
            "published"	INTEGER NOT NULL DEFAULT 0,
            "url"	TEXT NOT NULL,
            PRIMARY KEY("version","os","arch")
        )`);
    }

    public async getUserPrivate(username: string): Promise<string | undefined> {
        if (!this.db) {
            throw new Error("Database not initialized");
        }
        const result = await this.db.get<any>("SELECT private_key FROM users WHERE username = ?", username);
        return result?.private_key
    }

    public async setVersionUrl(version: string, os: string, arch: string, url: string) {
        if (!this.db) {
            throw new Error("Database not initialized");
        }
        await this.db.run(
            `INSERT INTO versions (version, os, arch, url) VALUES (?, ?, ?, ?)`,
            version, os, arch, url
        );
    }

    public async setVersionTestResult(version: string, os: string, arch: string, tested: boolean) {
        if (!this.db) {
            throw new Error("Database not initialized");
        }
        await this.db.run(
            `UPDATE versions SET tested = ? WHERE version = ? AND os = ? AND arch = ?`,
            tested ? 1 : -1, version, os, arch
        );
    }

    public async setVersionPublishResult(version: string, os: string, arch: string, published: boolean) {
        if (!this.db) {
            throw new Error("Database not initialized");
        }
        await this.db.run(
            `UPDATE versions SET published = ? WHERE version = ? AND os = ? AND arch = ?`,
            published ? 1 : -1, version, os, arch
        );
    }

    public async getVersions(pageNum: number, pageSize: number, version: string| undefined, os: string[] | undefined, arch: string[] | undefined, notest: boolean, nopub: boolean) {
        if (!this.db) {
            throw new Error("Database not initialized");
        }
        let query = `SELECT * FROM versions WHERE 1=1`;
        const params: any[] = [];

        if (version) {
            query += ` AND version = ?`;
            params.push(version);
        }

        if (os && os.length > 0) {
            query += ` AND os IN (${os.map(() => '?').join(', ')})`;
            params.push(...os);
        }

        if (arch && arch.length > 0) {
            query += ` AND arch IN (${arch.map(() => '?').join(', ')})`;
            params.push(...arch);
        }

        if (notest) {
            query += ` AND tested = 0`;
        }

        if (nopub) {
            query += ` AND published = 0`;
        }

        query += ` ORDER BY version DESC`;

        if (pageSize > 0) {
            query += ` LIMIT ? OFFSET ?`;
            params.push(pageSize, (pageNum - 1) * pageSize);
        }

        return await this.db.all<any[]>(query, ...params);
    }

    public async getVersionCount(): Promise<number> {
        if (!this.db) {
            throw new Error("Database not initialized");
        }
        const result = await this.db.get<{ count: number }>("SELECT COUNT(*) as count FROM versions");
        return result.count;

    }
    public async close() {
        await this.db!.close();
    }
}