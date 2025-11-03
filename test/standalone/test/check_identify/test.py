# 访问https://buckyos.ai/1.0/identifiers/self, 返回一个json
import urllib.request
import sys

import json
# 以后可以使用bucky_sdk替代单纯的requ

url = "http://test.buckyos.io/1.0/identifiers/self"
try:
    with urllib.request.urlopen(url) as response:
        if response.status == 200:
            data = response.read()
            json_data = json.loads(data)
            print("Identify info:", json_data)
            if json_data["id"] == "did:web:test.buckyos.io" and json_data["owner"] == "did:bns:devtest":
                sys.exit(0)
            else:
                print("Error: Identify info does not match expected values.")
                print("identify data:", json_data)
                sys.exit(1)
        else:
            print(f"Error: Unable to fetch identify info, status code: {response.status}")
            sys.exit(1)
except Exception as e:
    print(f"Exception occurred while fetching identify info: {e}")
    sys.exit(1)
    