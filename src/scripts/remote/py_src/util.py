
import subprocess
import re






def get_multipass_ip(instance_name):
    try:
        # 执行 multipass info 命令并捕获输出
        result = subprocess.run(
            ["multipass", "info", instance_name],
            capture_output=True,
            text=True,
            check=True  # 检查命令是否执行成功
        )
        
        # 匹配 IPv4 地址（包含多行的情况）
        ip_pattern = r"IPv4:\s+((?:\d+\.\d+\.\d+\.\d+\s*)+)"
        match = re.search(ip_pattern, result.stdout)
        
        if match:
            # 提取所有 IPv4 地址并整理为列表
            ips = [ip.strip() for ip in match.group(1).split()]
            return ips
        else:
            return "未找到 IPv4 地址"
            
    except subprocess.CalledProcessError as e:
        return f"错误：实例 '{instance_name}' 不存在或未运行"
    except Exception as e:
        return f"未知错误：{str(e)}"