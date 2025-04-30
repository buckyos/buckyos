import os

remote_server = "root@192.168.64.5"

app_loader = {}
app_loader["name"] = "app_loader"
app_loader["path"] = "./rootfs/bin/app_loader"
app_loader["remote_path"] = "/opt/buckyos/bin/app_loader/"
app_loader["is_dir"] = True

node_daemon = {}
node_daemon["name"] = "node_daemon"
node_daemon["path"] = "./rootfs/bin/node_daemon"
node_daemon["remote_path"] = "/opt/buckyos/bin/node_daemon/"
node_daemon["is_dir"] = True

push_pkgs = [node_daemon]

for pkg in push_pkgs:
    temp_path = "/tmp/buckyos/" + pkg["name"]
    if pkg["is_dir"]:
        print(f"rm -rf {temp_path}")
        os.system(f"ssh {remote_server} 'rm -rf {temp_path}'")
        print(f"scp -r {pkg['path']} {remote_server}:{temp_path}")
        os.system(f"scp -r {pkg['path']} {remote_server}:{temp_path}")
        print(f"chmod 777 -R {temp_path}")
        os.system(f"ssh {remote_server} 'chmod 777 -R {temp_path}'")
        print(f"sudo cp {temp_path}/* {pkg['remote_path']}")
        os.system(f"ssh {remote_server} 'sudo cp {temp_path}/* {pkg['remote_path']}'")
    else:
        print(f"rm -f {temp_path}")
        os.system(f"ssh {remote_server} 'rm -f {temp_path}'")
        print(f"scp {pkg['path']} {remote_server}:{temp_path}")
        os.system(f"scp {pkg['path']} {remote_server}:{temp_path}")
        print(f"chmod +x {temp_path}")
        os.system(f"ssh {remote_server} 'chmod +x {temp_path}'")
        print(f"sudo mv {temp_path} {pkg['remote_path']}")
        os.system(f"ssh {remote_server} 'sudo mv {temp_path} {pkg['remote_path']}'")








