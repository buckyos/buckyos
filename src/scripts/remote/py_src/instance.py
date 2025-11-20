"""
实例化命令入口
"""
import sys
import os

current_dir = os.path.dirname(os.path.abspath(__file__))
sys.path.append(current_dir)
import instance_mgr
import util


def instance_all():
    """实例化所有节点"""
    manager = instance_mgr.InstanceManager(util.CONFIG_BASE)
    manager.instance_all()


def instance_node(node_id: str):
    """实例化单个节点"""
    manager = instance_mgr.InstanceManager(util.CONFIG_BASE)
    manager.instance_node(node_id)


def main():
    """主函数"""
    if len(sys.argv) < 2:
        print("Usage: instance.py <node_id>")
        print("Usage: instance.py --all")
        sys.exit(1)
    
    target = sys.argv[1]
    
    if target == "--all":
        instance_all()
    else:
        instance_node(target)


if __name__ == "__main__":
    main()

