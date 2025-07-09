import { env, argv, exit } from "node:process";
import * as crypto from "node:crypto";
import * as secp256k1 from "secp256k1";

function prepare(content: any, username: string, privateKey: Buffer) {
    const data = JSON.stringify(content);
    const msg = crypto.createHash('sha256').update(data).digest();

    let sign = secp256k1.ecdsaSign(msg, privateKey).signature;

    return {
        content: content,
        username: username,
        signature: Buffer.from(sign).toString('hex'),
    }
}

let username = env['USERNAME'];
let pk = env['PRIVATE_KEY'];
if (!username || !pk) {
    console.error("Usage: set USERNAME and PRIVATE_KEY environment variables");
    process.exit(1);
}

let privateKey = Buffer.from(pk, 'hex');
if (privateKey.length !== 32) {
    console.error("Private key must be 32 bytes long");
    process.exit(1);
}


console.log("Username:", username);
let endpoint = env['ENDPOINT'] || 'http://localhost:9800';

async function postData(path: string, content: any) {
    let body = prepare(content, username!, privateKey);
    let resp = await fetch(endpoint+path, {
        method: 'POST',
        headers: {
            'Content-Type': 'application/json',
        },
        body: JSON.stringify(body),
    });
    console.log("Response status:", resp.status);
    let respData = await resp.text();
    console.log("Response data:", respData);
}

async function setUrl() {
    let version = argv[3];
    let os = argv[4];
    let arch = argv[5];
    let url = argv[6];
    if (!version || !os || !arch || !url) {
        console.error("Usage: node client.js seturl <version> <os> <arch> <url>");
        process.exit(1);
    }

    let content = {
        version: version,
        os: os,
        arch: arch,
        url: url,
    }

    await postData("/version/url", content);
}

async function setTest() {
    let version = argv[3];
    let os = argv[4];
    let arch = argv[5];
    let tested = argv[6] === "true";
    if (!version || !os || !arch) {
        console.error("Usage: node client.js settest <version> <os> <arch> <true|false>");
        process.exit(1);
    }

    let content = {
        version: version,
        os: os,
        arch: arch,
        tested: tested,
    }

    await postData("/version/test", content);
}

async function setPublish() {
    let version = argv[3];
    let os = argv[4];
    let arch = argv[5];
    let published = argv[6] === "true";
    if (!version || !os || !arch) {
        console.error("Usage: node client.js setpublish <version> <os> <arch> <true|false>");
        process.exit(1);
    }

    let content = {
        version: version,
        os: os,
        arch: arch,
        published: published,
    }

    await postData("/version/publish", content);
}

async function test_auth() {
    let content = {
        msg: "this is a test message",
    }

    await postData("/version/auth", content)
}

async function run() {
    let method = argv[2];
    if (method === "seturl") {
        await setUrl();
    } else if (method === "settest") {
        await setTest();
    } else if (method === "setpublish") {
        await setPublish();
    } else if (method == "auth") {
        await test_auth();
    } else {
        console.error("Usage: node client.js <seturl|settest|setpublish>");
        process.exit(1);
    }
}

run().then(() => {
    exit(0);
});

