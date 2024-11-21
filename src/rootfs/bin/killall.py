
import os

if os.system("killall node_daemon") != 0:
    print("node_daemon not running")
else:
    print("node_daemon killed")

if os.system("killall scheduler") != 0:
    print("scheduler not running")
else:
    print("scheduler killed")

if os.system("killall verify_hub") != 0:
    print("verify_hub not running")
else:
    print("verify_hub killed")

if os.system("killall system_config") != 0:
    print("system_config not running")
else:
    print("system_config killed")

if os.system("killall cyfs_gateway") != 0:
    print("cyfs_gateway not running")
else:
    print("cyfs_gateway killed")


