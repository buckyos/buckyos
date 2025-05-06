

const { SignJWT, generateKeyPair,jwtVerify,importJWK, exportJWK,base64url, importPKCS8} = require('jose');

async function createAndVerifyEdDSAJWT() {
    // 生成 EdDSA 密钥对（Ed25519）
    var { privateKey, publicKey } = await generateKeyPair('EdDSA');
    var jwk = await exportJWK(publicKey);
    console.log('Public Key (JWK base64URL):',jwk.x);   
    const privateKeyDer = privateKey.export({ type: 'pkcs8',format: 'pem' });
    console.log('Private Key (DER):', privateKeyDer);    
    
    privateKey = `
 -----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIIzZ5HJjrbfQqxMOmNZRbnnR93iqgjbE8iKADMfkdn39
-----END PRIVATE KEY-----   
    `
    //MC4CAQAwBQYDK2VwBCIEIMDp9endjUnT2o4ImedpgvhVFyZEunZqG+ca0mka8oRp
    const importPrivateKey = await importPKCS8(privateKey.trim(),"Ed25519");
    const publicKeyBase64 = "NZZYu7WHLuuUQhcBAUw5HsXsq2qu4KNZ_V1E9U00KJI";
    console.log('Public Key (Base64URL):', publicKeyBase64);

    const jwt = await new SignJWT({ 'my_test_name': true,exp:1724625212})
        .setProtectedHeader({ alg: 'EdDSA' })
        .setExpirationTime('2h')
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
