import socket

def start_udp_server(host='0.0.0.0', port=8888):
    """
    启动一个UDP服务器，监听指定地址和端口
    参数:
        host: 监听地址，默认0.0.0.0表示监听所有网络接口
        port: 监听端口，默认8888
    """
    # 创建UDP套接字
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)

    try:
        # 绑定地址和端口
        sock.bind((host, port))
        print(f"UDP服务器已启动，正在监听 {host}:{port}")

        while True:
            # 接收客户端数据（最大1024字节）
            data, client_addr = sock.recvfrom(1024)
            print(f"收到来自 {client_addr} 的数据: {data.decode('utf-8')}")

            # 将数据原样返回给客户端（也可以处理后再发送）
            sock.sendto(data, client_addr)

    except KeyboardInterrupt:
        print("\n服务器正在关闭...")
    finally:
        sock.close()
        print("服务器已关闭")

if __name__ == '__main__':
    start_udp_server()
