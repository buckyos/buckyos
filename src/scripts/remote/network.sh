#!/bin/bash

# sudo ip link add br-sn type bridge
# sudo ip link set br-sn up
# sudo ip addr add 192.0.2.254/24 dev br-sn



multipass list
$NODE_B1_IP=$(multipass info nodeB1 | grep IPv4 | awk '{print $2}')
$NODE_A2_IP=$(multipass info nodeA2 | grep IPv4 | awk '{print $2}')




multipass exec nodeA2 -- sudo ufw deny from $NODE_B1_IP
multipass exec nodeA2 -- sudo ufw enable

multipass exec nodeB1 -- sudo ufw deny from $NODE_A2_IP
multipass exec nodeB1 -- sudo ufw enable