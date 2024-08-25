

const { SignJWT, generateKeyPair,jwtVerify,importJWK, exportJWK,base64url, importPKCS8} = require('jose');

async function createAndVerifyEdDSAJWT() {
    // 生成 EdDSA 密钥对（Ed25519）
    //const { privateKey, publicKey } = await generateKeyPair('EdDSA');
    //var jwk = await exportJWK(publicKey);
    //console.log('Public Key (JWK):',JSON.stringify(jwk));   
    //const privateKeyDer = privateKey.export({ type: 'pkcs8',format: 'pem' });
    //console.log('Private Key (DER):', privateKeyDer);    
    privateKey = `
 -----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIMDp9endjUnT2o4ImedpgvhVFyZEunZqG+ca0mka8oRp
-----END PRIVATE KEY-----   
    `
    const importPrivateKey = await importPKCS8(privateKey.trim(),"Ed25519");
    const publicKeyBase64 = "gubVIszw-u_d5PVTh-oc8CKAhM9C-ne5G_yUK5BDaXc";
    console.log('Public Key (Base64URL):', publicKeyBase64);

    const jwt = await new SignJWT({ 'my_test_name': true,exp:1724625212})
        .setProtectedHeader({ alg: 'EdDSA' })
        .sign(importPrivateKey);

    console.log('JWT:', jwt);

    // 验证JWT
    const publicJWK = {
        kty: 'OKP',
        crv: 'Ed25519',
        x: publicKeyBase64,
    };

    const importedPublicKey = await importJWK(publicJWK);
    const { payload, protectedHeader } = await jwtVerify(jwt, importedPublicKey,{});

    console.log('JWT 验证成功');
    console.log('Protected Header:', protectedHeader);
    console.log('Payload:', payload);
}

createAndVerifyEdDSAJWT();
