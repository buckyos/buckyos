import sys
import os

def publish_app(remote_repo_url):
    if remote_repo_url is None:
        print("publish app to local repo")
    else:
        print(f"publish app to {remote_repo_url}")
    pass


if __name__ == "__main__":
    args = sys.argv
    if len(args) > 2:
        print("Usage: python publish.py [local|remote]")
        sys.exit(1)

    if len(args) == 1:
        publish_app(None)
    else:
        remote_repo_url = args[1]
        publish_app(remote_repo_url)