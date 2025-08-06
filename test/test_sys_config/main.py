import subprocess
import json
import sys
import os

# 获取并解析命令返回的json
def extract_json_from_output(command_output):
    """从命令输出中提取并解析JSON对象."""
    try:
        # 查找第一个 '{' 作为JSON开始的标志
        json_start_index = command_output.find('{')
        if json_start_index == -1:
            print("错误：在输出中未找到JSON的起始 '{'", file=sys.stderr)
            return None
        # 尝试找到匹配的最后一个 '}'
        # 这是一个简化的方法，对于嵌套复杂的非JSON内容可能不够鲁棒
        # 更可靠的方法是逐层匹配括号，或者如果JSON总是在特定标记后出现，则使用该标记
        # 例如，如果JSON总是在 "value:\n" 之后开始：
        # value_marker = "value:\n"
        # if value_marker in command_output:
        #     json_start_index = command_output.find(value_marker) + len(value_marker)
        # else:
        #     print(f"错误：在输出中未找到标记 '{value_marker}'", file=sys.stderr)
        #     return None

        # 从找到的 '{' 开始，尝试解析直到有效的JSON结束
        # 为了处理JSON后的其他文本（如示例中的 "version: 0"），我们需要更精确地找到JSON的结束
        # 一个简单的方法是找到第一个 '{' 和最后一个 '}' 之间的内容
        # 但这假设JSON对象本身不包含 '{' 或 '}' 字符在字符串值之外，或者它们是正确配对的
        
        # 寻找最后一个 '}'
        # 注意：如果JSON字符串内部有 '}'，这可能会提前结束
        # 更健壮的方法是尝试从 json_start_index 开始逐步解析
        json_end_index = command_output.rfind('}')
        if json_end_index == -1 or json_end_index < json_start_index:
            print("错误：在输出中未找到有效的JSON结束 '}'", file=sys.stderr)
            return None
        
        json_str_candidate = command_output[json_start_index : json_end_index + 1]
        
        parsed_json = json.loads(json_str_candidate)
        return parsed_json
    except json.JSONDecodeError as e:
        print(f"JSON解析错误: {e}", file=sys.stderr)
        print(f"尝试解析的字符串: '{json_str_candidate[:100]}...'", file=sys.stderr) # 打印部分字符串以供调试
        return None
    except Exception as e:
        print(f"提取JSON时发生未知错误: {e}", file=sys.stderr)
        return None



def main():
    boot_config()
    devices()
    nodes()
    services()
    system()
    users()

def boot_config():
    stdout = buckycli(["--get", "boot/config"])
    json_data = extract_json_from_output(stdout)
    # print(json_data)
    # print(f"@context    {json_data['@context']}")
    # print(f"ID          {json_data['id']}")
    # print(f"OOD         {json_data['oods']}")
    # print(f"SN          {json_data['sn']}")
    # print(f"owner       {json_data['owner']}")
    assert json_data['@context'] == "https://www.w3.org/ns/did/v1", "@context error"
    assert json_data['id'] == "did:bns:bob", "ID error"
    assert json_data['oods'][0] == "ood1", "OOD error"
    assert json_data['sn'] == "sn.buckyos.io", "sn error"
    assert json_data['owner'] == "did:bns:bobdev", "owner error"


def devices():
    stdout = buckycli(["--list", "devices"])
    # json_data = extract_json_from_output(result.stdout)
    # 将stdout按行分割成数组并去掉第一行
    # 分割并过滤掉第一行和空行
    devices = [line for line in stdout.split('\n')[1:] if line.strip()]
    print("")
    print("-------------------")
    print("-------------------")
    
    # device下层的固定key: doc, info
    for device in devices:
        print(f"current device: {device}")
        doc = buckycli(["--get", f"devices/{device}/doc"])
        assert doc != "", "doc error"
        info = buckycli(["--get", f"devices/{device}/info"])
        json_data = extract_json_from_output(info)
        # print(json_data)
        assert json_data['@context'] == "https://www.w3.org/ns/did/v1", "@context error"
        assert json_data['id'] == "did:dev:iSMKakFEGzGAxLTlaB5TkqZ6d4wurObr-BpaQleoE2M", "ID error"
        assert json_data['device_type'] == "node", "device_type error"
        assert json_data['name'] == "ood1", "name error"
        assert json_data['sys_hostname'] == 'nodeB1', 'sys_hostname error'
        assert isinstance(json_data['verificationMethod'],list),"verificationMethod error"
        assert 'base_os_info' in json_data, "base_os_info error"
        assert 'cpu_info' in json_data, "cpu_info error"
        assert 'gpu_info' in json_data, "gpu_info error"
        assert 'total_mem' in json_data, "total_mem error"
        assert 'ip' in json_data, "ip error"
        # print(stdout)
        # items = [line for line in stdout.split('\n')[1:] if line.strip()]
        # for item in items:
        #     stdout = buckycli(["--get", f"devices/{device}/{item}"])
        #     # print(f"{"devices/{device}/{item}"}:")
        #     print(stdout)
        #     print("-------------------")

def nodes():
    stdout = buckycli(["--list", "nodes"])
    nodes = [line for line in stdout.split('\n')[1:] if line.strip()]
    print("")
    print("-------------------")
    print("-------------------")
    # node下层的固定key: config, gateway_config
    for node in nodes:
        print(f"current node: {node}")
        config = buckycli(["--get", f"nodes/{node}/config"])
        config_json = extract_json_from_output(config)
        # print(f"config: {config}")
        assert 'apps' in config_json, "apps error"
        assert 'revision' in config_json, "revision error"
        # assert 'state' in config_json, "state error"
        assert 'frame_services' in config_json, "frame_services error"
        assert 'is_running' in config_json, "is_running error"
        assert 'kernel' in config_json, "kernel error"
        assert 'scheduler' in config_json['kernel'], "scheduler error"
        assert 'verify-hub' in config_json['kernel'], "verify-hub error"

        gateway_config = buckycli(["--get", f"nodes/{node}/gateway_config"])
        gateway_config_json = extract_json_from_output(gateway_config)
        # print(f"gateway_config: {gateway_config_json}")
        assert 'dispatcher' in gateway_config_json, "dispatcher error"
        assert 'tcp://0.0.0.0:443' in gateway_config_json['dispatcher'], "dispatcher error"
        assert 'tcp://0.0.0.0:80' in gateway_config_json['dispatcher'], "dispatcher error"
        assert 'inner_services' in gateway_config_json, "inner_services error"
        assert 'zone_provider' in gateway_config_json['inner_services'], "inner_services error"
        assert 'servers' in gateway_config_json, "servers error"
        assert 'zone_gateway' in gateway_config_json['servers'], "servers error"


# services: gateway repo-service scheduler verify-hub
def services():
    print("")
    print("-------------------")
    print("-------------------")
    gateway_settings = buckycli(["--get", f"services/gateway/settings"])
    gateway_settings_json = extract_json_from_output(gateway_settings)
    assert 'shortcuts' in gateway_settings_json, "shortcuts error"

    repo_service_config = buckycli(["--get", f"services/repo-service/config"])
    repo_service_settings = buckycli(["--get", f"services/repo-service/settings"])

    repo_service_config_json = extract_json_from_output(repo_service_config)
    assert 'name' in repo_service_config_json, "name error"
    assert 'pkg_id' in repo_service_config_json, "pkg_id error"
    assert 'node_list' in repo_service_config_json, "node_list error"
    assert 'port' in repo_service_config_json, "port error"
    assert 'vendor_did' in repo_service_config_json, "vendor_did error"
    assert repo_service_config_json['service_type'] == "frame", "service_type error"

    repo_service_settings_json = extract_json_from_output(repo_service_settings)
    assert 'remote_source' in repo_service_settings_json, "remote_source error"

    scheduler_config = buckycli(["--get", f"services/scheduler/config"])
    scheduler_config_json = extract_json_from_output(scheduler_config)
    assert 'name' in scheduler_config_json, "name error"
    assert 'pkg_id' in scheduler_config_json, "pkg_id error"
    assert 'node_list' in scheduler_config_json, "node_list error"
    assert 'port' in scheduler_config_json, "port error"
    assert 'vendor_did' in scheduler_config_json, "vendor_did error"
    assert scheduler_config_json['service_type'] == "kernel", "service_type error"


    verify_hub_config = buckycli(["--get", f"services/verify-hub/config"])
    verify_hub_config_json = extract_json_from_output(verify_hub_config)
    assert 'name' in verify_hub_config_json, "name error"
    assert 'pkg_id' in verify_hub_config_json, "pkg_id error"
    assert 'node_list' in verify_hub_config_json, "node_list error"
    assert 'port' in verify_hub_config_json, "port error"
    assert 'vendor_did' in verify_hub_config_json, "vendor_did error"
    assert verify_hub_config_json['service_type'] == "kernel", "service_type error" 

    verify_hub_settings = buckycli(["--get", f"services/verify-hub/settings"])
    #verify_hub_settings_json = extract_json_from_output(verify_hub_settings)
    # for service in services:
    #     print(f"service: {service}")
    #     stdout = buckycli(["--get", f"services/{service}/config"])
    #     print(stdout)
    #     setting = buckycli(["--get", f"services/{service}/settings"])
    #     print(setting)
        # items = [line for line in stdout.split('\n')[1:] if line.strip()]
        # for item in items:
        #     stdout = buckycli(["--get", f"services/{service}/{item}"])
        #     print(stdout)
        #     print("-------------------")

def system():
    stdout = buckycli(["--list", "system"])
    keys = [line for line in stdout.split('\n')[1:] if line.strip()]
    # system/rbac: [base_policy, model, policy]
    print("")
    print("-------------------")
    print("-------------------")
    base_policy = buckycli(["--get", f"system/rbac/base_policy"])
    base_policy_rules = []
    base_policy_group_rules =[]
    for line in base_policy.split('\n'):
        if line == "":
            continue
        if line.startswith('p'):
            base_policy_rules.append(line)
        if line.startswith('g'):
            base_policy_group_rules.append(line)
    test_permission_rules(base_policy_rules, base_policy_group_rules)

    model = buckycli(["--get", f"system/rbac/model"])
    result = isToml(model)
    assert result == True, "system/rbac/model is not toml"

    policy = buckycli(["--get", f"system/rbac/policy"])
    policy_rules = []
    policy_group_rules =[]
    for line in policy.split('\n'):
        if line == "":
            continue
        if line.startswith('p'):
            policy_rules.append(line)
        if line.startswith('g'):
            policy_group_rules.append(line)
    test_permission_rules(policy_rules, policy_group_rules)

    system_pkgs = buckycli(["--get", f"system/system_pkgs"])
    

    return

    for key in keys:
        stdout = buckycli(["--list", f"system/{key}"])
        items = [line for line in stdout.split('\n')[1:] if line.strip()]
        for item in items:
            stdout = buckycli(["--get", f"system/{key}/{item}"])
            # print(stdout)
            # print("-------------------")

def users():
    stdout = buckycli(["--list", "users"])
    users = [line for line in stdout.split('\n')[1:] if line.strip()]
    print("")
    print("-------------------")
    print("-------------------")
    for user in users:
        print(f"user: {user}")
        if user == "root":
            settings = buckycli(["--get", f"users/root/settings"])
            print(f"setting: {settings}")
            print("-------------------")
            continue
        apps = buckycli(["--list", f"users/{user}/apps"])
        doc = buckycli(["--get", f"users/{user}/doc"])
        settings = buckycli(["--get", f"users/{user}/settings"])
        print(f"apps: {apps}")
        print(f"doc: {doc}")
        print(f"setting: {settings}")
        print("-------------------")


def buckycli(cmd: list[str]):
    base = ["/opt/buckyos/bin/buckycli/buckycli", "sys_config"]
    cmd = base + cmd
    env_vars = os.environ.copy()
    env_vars['BUCKY_LOG'] = 'off'
    print(f"cmd: {" ".join(cmd)}")
    result = subprocess.run(cmd,
        capture_output=True,
        text=True,
        check=False,
        env=env_vars)
    if result.returncode!= 0:
        print(f"(stderr):\n{result.stderr}", file=sys.stderr)
        sys.exit(1)
    # print(f"run `buckycli sys_config --list devices` OK, stdout: {result.returncode}")
    return result.stdout

def isToml(content: str):
    try:
        # 简单验证TOML格式的基本结构
        lines = content.strip().split('\n')
        current_section = None
        for line in lines:
            line = line.strip()
            if not line or line.startswith('#'):
                continue
            # 检查section标记 [section]
            if line.startswith('[') and line.endswith(']'):
                current_section = line[1:-1]
                continue
                
            # 检查key-value对
            if '=' in line:
                key, value = line.split('=', 1)
                key = key.strip()
                value = value.strip()
                
                # key必须存在
                if not key:
                    return False
    except ValueError as e:
        return False
    return True


def test_permission_rules(rules, group_rules):
    # 测试所有权限规则
    for rule in rules:
        parts = rule.strip().split(',')
        assert len(parts) >= 4, f"规则格式错误: {rule}"
        assert parts[0] == "p", f"规则类型错误: {parts[0]}"
        assert parts[1].strip(), f"主体不能为空: {rule}"
        assert parts[2].strip().startswith(("kv://", "dfs://", "ndn://")), f"资源路径格式错误: {parts[2]}"
        assert "read" in parts[3] or "write" in parts[3], f"权限类型错误: {parts[3]}"
        assert parts[4].strip() == "allow" or parts[4].strip() == "deny", f"操作结果错误: {parts[4]}"
    # 测试所有组规则
    for rule in group_rules:
        parts = rule.strip().split(',')
        assert len(parts) == 3, f"组规则格式错误: {rule}"
        assert parts[0] == "g", f"组规则类型错误: {parts[0]}"
        assert parts[1].strip(), f"组成员不能为空: {rule}"
        assert parts[2].strip(), f"组不能为空: {rule}"



if __name__ == "__main__":
    main()
