#!/bin/python3

import os
import sys
import subprocess

def start_node_daemon():
    suffix = '.exe' if os.name == 'nt' else ''
    new_exe = f"node_daemon.new{suffix}"
    exe = f"node_daemon{suffix}"
    old_exe = f"node_daemon.old{suffix}"

    if os.path.exists(new_exe):
        if os.path.exists(old_exe):
            os.remove(old_exe)
        os.rename(exe, old_exe)
        os.rename(new_exe, exe)

    exe_path = os.path.join(os.path.dirname(os.path.abspath(__file__)), exe)
    
    if os.name == 'nt':
        # Windows: 使用 subprocess.run
        subprocess.run([exe_path] + sys.argv[1:])
    else:
        # Linux: 使用 os.execv
        os.execv(exe_path, [exe] + sys.argv[1:])

start_node_daemon()

