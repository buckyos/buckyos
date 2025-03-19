import socket
import ipaddress
import concurrent.futures
import os
import netifaces

PORT = 3180

def get_network_from_ip():
    """
    根据本机 IP 地址确定当前局域网网段
    """
    interfaces = netifaces.interfaces()
    for iface in interfaces:
        addrs = netifaces.ifaddresses(iface)
        if netifaces.AF_INET in addrs:
            for addr in addrs[netifaces.AF_INET]:
                ip = addr['addr']
                netmask = addr['netmask']
                if ip and netmask:
                    ip_network = ipaddress.IPv4Network(f"{ip}/{netmask}", strict=False)
                    return str(ip_network)
    return None

def scan_ip(ip):
    """
    扫描指定 IP 的指定端口是否开放
    """
    #print(f"扫描 {ip} 的端口 {PORT}")
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.settimeout(1)  # 设置超时时间为 1 秒
        try:
            s.connect((ip, PORT))
            print(f"IP {ip} 的端口 {PORT} 可用")
            return ip
        except (socket.timeout, socket.error):
            return None

def main():
    # 获取局域网网段
    print("获取局域网网段")
    network = get_network_from_ip()
    if not network:
        print("无法获取局域网网段")
        return

    # 获取所有可能的 IP 地址
    print("获取所有可能的 IP 地址")
    ip_network = ipaddress.ip_network(network, strict=False)
    ip_list = [str(ip) for ip in ip_network.hosts()]

    available_ips = []
    # 使用多线程加快扫描速度
    print("使用多线程扫描...")
    with concurrent.futures.ThreadPoolExecutor(max_workers=50) as executor:
        future_to_ip = {executor.submit(scan_ip, ip): ip for ip in ip_list}
        for future in concurrent.futures.as_completed(future_to_ip):
            result = future.result()
            if result:
                available_ips.append(result)

    if available_ips:
        print("\n以下设备的端口 3180 可用:")
        for ip in available_ips:
            print(f"http://{ip}:3180/index.html")
    else:
        print("\n没有设备的端口 3180 可用")

if __name__ == "__main__":
    main()
