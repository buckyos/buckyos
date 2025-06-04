#!/bin/bash

SCRIPT_OWN_DIR=$(cd -- "$(dirname -- "$0")" &> /dev/null && pwd)
multipass transfer $SCRIPT_OWN_DIR/main.py nodeB1:/home/ubuntu/main.py


multipass exec nodeB1 -- python3 /home/ubuntu/main.py