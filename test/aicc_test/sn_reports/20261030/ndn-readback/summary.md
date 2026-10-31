# NDN 制品回读诊断

本轮真实模型调用：0。使用现有 `cyfs-gateway` 的 `cyfs-dir` 服务和临时 Gateway 路由，GET 同一 Named Store 内的既有测试对象；没有修改 NDN 代码。34 个对象全部 HTTP 200、非空，总字节数 36,267,601。临时路由和辅助进程已清理。

33 个对象来自已成功任务的制品引用，另 1 个是 GPT-5.5 前置生成在归属登记失败前写入的图片。这不是重新执行原来未到达的 img2img/edit/extend 调用，也没有完成全部制品内容质量复核；不据此改写原始 T2 轮次。

| 来源 | 对象 | HTTP | 字节数 | SHA256 |
|---|---|---:|---:|---|
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-4o-mini-tts.audio.tts | `cyfile:484be99bdcbb7e90cc7bc67e23334188f26437aa5cbdfcea4f47ed819cd8f472` | 200 | 48768 | `4a0ca1165128a6f8ddd13ac2f07e1fc884980744a22fb42be72ae14f93d54a26` |
| continuation:image.txt2img:gpt-5.4-mini@sn-us-full-20261030-t2 | `cyfile:5525707d4da11fbe356f8a9fa3ccdb36ede3887f5d4b0ca97d436d1285ae9dd1` | 200 | 1092711 | `7ec8659ec31b4c26ac86f6aa39f1dbc2eb34faa184f7376334e2f0b0cc3c66d2` |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-mini.image.txt2img | `cyfile:fc71dcf44e87c0b193f1f34e9adb8b569425a19b91affddbb4683f593f0bafa7` | 200 | 1110076 | `54143a850c2c65f899577e59579c7057791fbeb03fd5f1790cf516263fc203ff` |
| continuation:image.txt2img:gpt-5.4-nano@sn-us-full-20261030-t2 | `cyfile:c36f0da1280a5f39dd0dc52b58b70de71e62e400b891f643f4687a90e2120fae` | 200 | 1189018 | `e959af6e6412894f49f950b8e67fd8aad04b5197d8e0331670c6706a10e1d2cc` |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-nano.image.txt2img | `cyfile:08f8ee344aa6771c6d1426737203b1f810616b1f45444a1abb69ccc5e480b50a` | 200 | 1171092 | `81d053daded9638573981254b4d9b38458eead859e39eb59d754bae9a118c0c7` |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4-pro.image.txt2img | `cyfile:419e6f295fd748b347fbe1c32979efe86fa00ed772f52d4fe6c21fcb11928069` | 200 | 1069761 | `189ce58b32c272acb018e42fbfb6fa98ce04c7613e5abda9475bad63a01ae9c4` |
| continuation:image.txt2img:gpt-5.4@sn-us-full-20261030-t2 | `cyfile:07056366c7fabd3bd415f2bbcdc113ce743fa2b6bf61f3ccf0631ae769ca47a9` | 200 | 1183911 | `3ae415cf8c1a892664fa1c2828d879595f00efa6ffc345b7f62fe9cc1ec282bd` |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.4.image.txt2img | `cyfile:2451f3bfe5c15230ce0855b98910b36cda257bbf1976d7222f8b87e97919d87b` | 200 | 1188347 | `c19615f053b26d717204c791be2256da3b661c1e6e01c982f7373f1772ed590f` |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.5-pro.image.txt2img | `cyfile:45de3b169edd00604b7d196cf25b589cee24d843806ecdc10c4da5f1131eaf24` | 200 | 1064952 | `8c8da3ff3642122075c59a559bb0695eef4ade4fc191be5556eaea8a30f07cce` |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.5.image.txt2img | `cyfile:ee23aecc24d0bb52453c63f211d38f9d771d8c7cc901dd9a6ca0b9f03dbe523e` | 200 | 1093219 | `e85a62d68ca2825fc4ee318763dc57ed597ae98e8858709ec36317a51b12df6f` |
| continuation:image.txt2img:gpt-5.6-luna@sn-us-full-20261030-t2 | `cyfile:80d1d4dca91e5527945d1bea1df37d08055705b458b0e3708b2c8d3ca17598e0` | 200 | 1056558 | `393399475bcdd341965647823077902eeda3622ac4eb74e2f3fdb0197428e297` |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-luna.image.txt2img | `cyfile:769bcb4f62d1bf02cec9b97b5a3b958dc2e420e119fbdaedc4e9fc3c8bb21037` | 200 | 1117457 | `cabb1eefe89f318526b400e071dbd5ae9707a31e714ef6b0f96314fdc14e4bd2` |
| continuation:image.txt2img:gpt-5.6-sol@sn-us-full-20261030-t2 | `cyfile:1793bd546c1212ede2d9c9eaedec679a9664b6a804267d51fd8db0edfc22ef2a` | 200 | 1099130 | `1be038adec7448f19082f86fe72340efd12462b1f5851921357f2cc229ac64a8` |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-sol.image.txt2img | `cyfile:59f4910f457d4cc49fda89eee26000ca3991441ee8f70f4910abb509a419dd79` | 200 | 1075912 | `bc4f4665ed7b94c07a3ee399e7c721e9a69f3fc1060402c0e379da8464e4592b` |
| continuation:image.txt2img:gpt-5.6-terra@sn-us-full-20261030-t2 | `cyfile:c9a04199470d133150805f73f16375aac13ad36ff76820b881b16739681c6704` | 200 | 1085555 | `4b91aba0ca86c3a00b04b53ecc439fd5499e16641c2f19dcdce112cc46ec3c17` |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-5.6-terra.image.txt2img | `cyfile:33f68109041223257c0a45b6dacbd6dd4a1250854c7337a7b6172433c4d6b23b` | 200 | 1096495 | `ffd303a92c12562d90c4ef46ee2110089047bbc9dbffe7d4b632101daa905941` |
| continuation:image.txt2img:gpt-6-astra@sn-us-full-20261030-t2 | `cyfile:47e742ac6024808a28659e2beb5f6989b4f0f80532dd4d78f0f43829b6a13fad` | 200 | 1046590 | `1303d8143b3505b0236338045c59b7c16eabe23441acdab3240f95da894cca15` |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-astra.image.txt2img | `cyfile:8507e390a976d17e7368232cc01e21f5670fa38cec8bde4da307f79c197183ea` | 200 | 1183136 | `cfe77dfa3d2a8d272802179406287953d9fb0aa89fe870ce20f4e2b0d0be36ac` |
| continuation:image.txt2img:gpt-6-luna@sn-us-full-20261030-t2 | `cyfile:699d551107cc4117c8fc96d9209c887b3dd2d38eb1dc16c4ba2d5b3e7e2c6662` | 200 | 1117555 | `56f13dffc229e9d40532e6ad595b3e344c9f0550dea5bee6a4af0464a7f9c95d` |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-luna.image.txt2img | `cyfile:7597313ca520dc29537174d43ee79e673d29a66b7de5dcc23f13a27b8a6a0653` | 200 | 1173665 | `2a458b4800e0fc0ef505b0bfd8c2344c83a878481d3dca24136c9848c137c504` |
| continuation:image.txt2img:gpt-6-sol@sn-us-full-20261030-t2 | `cyfile:69a5c399ddb48b5f83eae9985e3cccf162c75acc7603df4ad70d12612ab268f7` | 200 | 1065978 | `8f371068be9eaf6f6fa5fb0f3fd8399d01f70de7052b84d212c9e2f2efe04944` |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6-sol.image.txt2img | `cyfile:62b00c43a0aec6311d0990d1c9d60575a892faa1ee4bf1e293e3bee13a56e4dd` | 200 | 1160908 | `7fe56f086f10a2c9657e711cd5d36751dcc86a08985c421117df447631f82131` |
| continuation:image.txt2img:gpt-6.1-sol@sn-us-full-20261030-t2 | `cyfile:c31fa4f7e03551ca730ae6fe9be7f88fa190a99d329ace99b683bbdc09a669e6` | 200 | 1204585 | `762fda4820f67d63ee94d6de803a5b10b0b49e66a39614aab5e8d75c1f3f081f` |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6.1-sol.image.txt2img | `cyfile:a1fa2f17c1c5cfe903f19a7d8e92e8588c7d858cad4609c3cfbbdfbf5a444873` | 200 | 1183905 | `c0c4d579e701c5e19bfee97bb201dd267f422448d6856449e2713db3dac1c3ee` |
| continuation:image.txt2img:gpt-image-2.5-flare@sn-us-full-20261030-t2 | `cyfile:90138813777200501686d4e35fd7cf98eb36402ddc4fda8ac3131d4871d8fc04` | 200 | 1082740 | `358cc4254f84dfa9e9c28e01b2e3f0b16f71eb635bc48baa7b2cfefd95a14e00` |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.5-flare.image.inpaint | `cyfile:5ade9c34054da884bfe30da60a1d4170f082ba03b5f754109991e21d807e7d21` | 200 | 515145 | `fbbabe18f48b29de3992c9b90090863c1f6fd09c9451a6dbdfe0c54dedc40ab2` |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.5-flare.image.txt2img | `cyfile:8d9e8bbfa9de234657e580e53b530c2e4eb38df6291479b249adfd9ad8fb75a2` | 200 | 1167346 | `7d778a133cef9f76d82b96d53f4400d452c3efe4a90964092a07d7ca21be8477` |
| continuation:image.txt2img:gpt-image-2.5-sunburst@sn-us-full-20261030-t2 | `cyfile:ae456a0cb35ca8df242fee3f3e9526e8bf5541be4c591fee869def46ad63821a` | 200 | 1140491 | `5550e7cf87c85651aaca0e77845ef9a0e6674f6bd89d6a0fa5e59d0f235e3814` |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.5-sunburst.image.inpaint | `cyfile:ef930dd1ff5d3de388ddcdbbf562f58d5cce5a3bddaf62473fa26e8dbd2ab248` | 200 | 506468 | `af5ebbfc1f7ce5c48ed3c3dab1025e9db003c16071039cf0ce41ea72846a54ad` |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.5-sunburst.image.txt2img | `cyfile:4b4b5e092e3be6275e563f9ea4f5db17913e372902423ca31028ec0700b8bbdf` | 200 | 1172479 | `4f8318b5587bb84c807729f0320158380e6405f2aa86ded70b40b7b573e94ff4` |
| continuation:image.txt2img:gpt-image-2@sn-us-full-20261030-t2 | `cyfile:8c592f0e3f7e9875428d22c47ed843707f92b45b6706b43d3114bb080cc6a105` | 200 | 1178312 | `785f5ad78fb43d757f7c0f3b8496a05633b72756b632ae6ca755c97774c75b43` |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.image.inpaint | `cyfile:9d761261592219cfb21fc0de571df2feeb1a7399d1d0b96b5ae52b0325f1abdc` | 200 | 1360045 | `285126b9bfcf28dfed199c49e0b48a36759a735700d59cab2b695ae066581988` |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.image.txt2img | `cyfile:57b1d43f30668ecdae83c321862c326db3a930290c98fe912282e0aeb8648ff8` | 200 | 1179840 | `dd696551985c23166a81221613723491e419385ca56c3067e72754f817aea8ef` |
| failed-scope-write | `cyfile:690660ffcdb80fcf8cc8a8285ae4cd902188e4d8a85e11cb18af79e1e6d8f1fe` | 200 | 1085451 | `bfb255ddb762685fa2bb29bc8feb86ff4fbc028a7be7883e4848a77dee49ccba` |
