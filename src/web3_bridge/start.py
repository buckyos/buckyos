#!/bin/python3

import os

current_dir = os.path.dirname(os.path.abspath(__file__))

config_file = f"{current_dir}/web3_gateway.json"

#print(f"Gateway config_file: {config_file}")
os.system(f"nohup {current_dir}/web3_gateway --config_file {config_file} --disable-buckyos> /dev/null 2>&1 &")
print("web3_gateway service started")