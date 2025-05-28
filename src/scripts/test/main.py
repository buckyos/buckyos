
# 步骤
# scp 测试文件到 nodeB1
# 执行 python main
# asset 执行结果
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
  # /opt/buckyos/bin/buckycli/buckycli sys_config --get boot/config
    boot_config()
    device()

def boot_config():
    stdout = buckycli(["--get", "boot/config"])
    json_data = extract_json_from_output(stdout)
    # print(json_data)
    print(f"@context    {json_data['@context']}")
    print(f"ID          {json_data['id']}")
    print(f"OOD         {json_data['oods']}")
    print(f"SN          {json_data['sn']}")
    print(f"owner       {json_data['owner']}")


def device():
    stdout = buckycli(["--list", "devices"])
    # json_data = extract_json_from_output(result.stdout)
    # 将stdout按行分割成数组并去掉第一行
    # 分割并过滤掉第一行和空行
    devices = [line for line in stdout.split('\n')[1:] if line.strip()]
    for device in devices:
        print(f"device: {device}")
        stdout = buckycli(["--list", f"devices/{device}"])
        print(stdout)

    


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
    


if __name__ == "__main__":
    main()
