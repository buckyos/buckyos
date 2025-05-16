#!/usr/bin/python3

import os
import sys

args = sys.argv
print(f"args: {args}")
current_dir = os.path.dirname(os.path.abspath(__file__))

config_file = f"{current_dir}/web3_gateway.json"
if "debug" in args:
    os.system(f"{current_dir}/web3_gateway --config_file {config_file} --debug")
else:
    os.system(f"nohup {current_dir}/web3_gateway --config_file {config_file} > /dev/null 2>&1 &")
print("web3_gateway service started")