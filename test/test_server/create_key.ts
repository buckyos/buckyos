import { randomBytes } from 'node:crypto';
import { existsSync } from 'node:fs';
import { argv, exit } from 'node:process';
import * as secp256k1 from 'secp256k1';

import * as sqlite3 from 'sqlite3';

let username = argv[2];
if (!username) {
    console.error("Usage: node create_key.js <username>");
    exit(1);
}

while (true) {
    let privateKey = randomBytes(32);
    if (secp256k1.privateKeyVerify(privateKey)) {
        console.log(`Username: ${username}`);
        console.log(`Private Key: ${privateKey.toString('hex')}`);

        if (existsSync('server.db')) {
            console.log("insert to server db...")
            let db = new sqlite3.Database('server.db', (err) => {
                if (err) {
                    console.error("Error opening database:", err.message);
                    exit(1);
                }
            });

            db.run(`INSERT INTO users (username, private_key) VALUES (?, ?)`, 
                [username, privateKey.toString('hex')],
                function(err) { 
                    if (err) {
                        console.error("Error inserting data:", err.message);
                        exit(1);
                    } else {
                        console.log(`User ${username} created successfully.`);
                    }
                });
        }
        break;
    }
}