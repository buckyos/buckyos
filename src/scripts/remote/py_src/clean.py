import argparse
import sys
import subprocess







def cleanInstances():
    question = "是否清除sn，nodeA2，nodeB1实例"
    answer = input(f"{question} (y/n): ").lower()
    if answer in ('y', 'yes'):
        try:
            # 调用 multipass 执行清除操作
            subprocess.run(['multipass', 'delete', 'sn', 'nodeA2', 'nodeB1', '--purge'], check=True)
            print("Multipass 实例已成功清除。")
        except subprocess.CalledProcessError as e:
            print(f"清除 Multipass 实例时出错: {e}")
        except FileNotFoundError:
            print("未找到 multipass 命令，请检查是否已安装。")
    else:
        sys.exit(0)
