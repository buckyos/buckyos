# rtcp 协议

rtcp协议可以在A<->B 之间的网络连通受限的情况下，
rtcp的默认端口是2980，使用tcp协议。
- 在A<->B之间udp被封锁的情况下，可以互相使用对方提供的udp服务 
- 在A可以直连B的2980端口，而B无法直连A的2980端口时 (A在NAT后），支持B访问A提供的tcp服务
- B只开放了2980端口，通过rtcp协议，支持A访问B的全部服务

rtcp还强制实现了A<->B之间的通信加密，通过rtcp访问服务，即使服务协议本身是明文的(http),也可以保障在网络传输中是加密的
rtcp是强身份的，A和B之间都必须互相先信任对方的公钥

## 基本流程

### Step1 建立Tunnel 

A: Tcp.connect(B,2980)
A: Send Tunnel Hello
B: Send Tunnel HelloAck

只要Tunnel建立，那么这个Tunnel对B和A来说就是等效的,不同在于 主动发起连接成功的一方，can_direct = true, 总是用Open逻辑来连接对方的服务，而另一面的can_direct = false,总是用ROpen来连接对面的服务

### Step2 A连接B运行在3200端口上的服务
A: Send Stream Open(127.0.0.1, 3200, sessionid)
A': session_streamA = Tcp.connect(B,2980), Send StreamHello(sessionid)
B': session_stream_real = Tcp.connect(127.0.0.1,3200)
B': aes_copy_stream(session_stream_real,session_streamA,sessionid)
B: Send Stream OpenResp(sessionid)

### Step3 B连接A运行在3300端口上的服务
B: Send Stream ROpen(127.0.0.1,3300,sessionid)
A': session_steam_real = Tcp.connect(127.0.0.1,3300)
A': session_streamA = Tcp.connect(B,2980),Send StreamHello(sessionid)
A: Send Stream RopenResp(127.0.0.1,3300,sessionid)
A': aes_copy_stream(session_steam_real,session_streamA,sessionid)
B:  session_stream = accept from 2980 and first package is StreamHello(sessionid)

