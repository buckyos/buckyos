import os

sn_server = "root@sn.buckyos.ai"
# 更新 web3_gateway
os.system(f"scp ./web3_gateway {sn_server}:/opt/web3_bridge/web3_gateway")
# 修改权限为可执行
os.system(f"ssh {sn_server} 'chmod +x /opt/web3_bridge/web3_gateway'")








