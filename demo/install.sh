#! /bin/bash

main() {
	# 获取操作系统信息
	if [ -f /etc/os-release ]; then
	    . /etc/os-release
	    OS=$NAME
	else
	    echo "Unable to determine operating system type."
	    exit 1
	fi

	if [ "$OS" = "Ubuntu" ]; then
	    echo "System is $OS"
	else
	    echo "This script currently only supports Ubuntu."
	    exit 1
	fi

	sudo apt-get update

	# 检查docker命令是否存在
	if ! command -v docker &> /dev/null
	then
	    echo "Installing Docker."
	    # Add Docker's official GPG key:
	    sudo apt-get update
	    sudo apt-get install ca-certificates curl -y
	    sudo install -m 0755 -d /etc/apt/keyrings
	    sudo curl -fsSL https://download.docker.com/linux/ubuntu/gpg -o /etc/apt/keyrings/docker.asc
	    sudo chmod a+r /etc/apt/keyrings/docker.asc

	    # Add the repository to Apt sources:
	    echo \
	      "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/ubuntu \
	      $(. /etc/os-release && echo "$VERSION_CODENAME") stable" | \
	      sudo tee /etc/apt/sources.list.d/docker.list > /dev/null
	    sudo apt-get update
	    sudo apt-get install -y docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin
	else
	    echo "Docker has installed."
	fi

	sudo apt-get install curl jq fuse dnsutils -y

	# buckycli="/mnt/f/work/buckyos/demo/target/release/buckycli"
	buckycli="/usr/local/bin/buckycli"
	echo "Downloading buckycli..."
	ensure sudo curl -z "$buckycli" --progress-bar -o "$buckycli" https://cache.mynode.site/buckycli
	sudo chmod +x "$buckycli"

	docker_image="harbor.mynode.site:8443/library/buckyos:latest"
# 	docker_image="buckyos:latest"

	read -p "Please enter your zone name: " zone_name < /dev/tty
	zone_cfg_mode="dns"
	zone_cfg_check=$($buckycli check_dns $zone_name)
	case $zone_cfg_check in
		"valid" )
			echo "The zone config is valid."
			zone_cfg=$($buckycli query_dns $zone_name)
			node_1=$(jq -r -n --argjson zone_cfg "$zone_cfg" '$zone_cfg.extra.etcds[0].name')
			node_2=$(jq -r -n --argjson zone_cfg "$zone_cfg" '$zone_cfg.extra.etcds[1].name')
			node_3=$(jq -r -n --argjson zone_cfg "$zone_cfg" '$zone_cfg.extra.etcds[2].name')
			echo "The name of node 1 is $node_1."
			echo "The name of node 2 is $node_2."
			echo "The name of node 3 is $node_3."
			;;
		"invalid")
			echo "Zone configuration on DNS is not available."
			read -p "Please enter the name of node 1 [default: etcd1]: " node_1 < /dev/tty
			read -p "Please enter the name of node 2 [default: etcd2]: " node_2 < /dev/tty
			read -p "Please enter the name of node 3 [default: gateway]: " node_3 < /dev/tty
			if [ -z "$node_1" ]; then
				node_1="etcd1"
			fi
			if [ -z "$node_2" ]; then
				node_2="etcd2"
			fi
			if [ -z "$node_3" ]; then
				node_3="gateway"
			fi

			while true; do
				read -p "Configure zone to dns or place it locally.[dns(d)/local(l)] [default: local]: " zone_mode < /dev/tty
				if [ -z $zone_mode ]; then
					zone_mode="local"
				fi
				case $zone_mode in
					"dns" | "d" )
						echo "You have selected dns mode."
						while true; do
							zone_cfg=$(create_zone_dns_cfg $zone_name $node_1 $node_2 $node_3)
							echo $zone_cfg > /tmp/zone_cfg.json
							$buckycli encode_dns -f /tmp/zone_cfg.json
							encoded_cfg=$($buckycli encode_dns -f /tmp/zone_cfg.json)
							echo "Please set the following content as the dns txt record of the domain name:"
							echo $encoded_cfg

							zone_cfg_check=$($buckycli check_dns $zone_name)

							case $zone_cfg_check in
								"valid" )
									echo "The zone config is valid."
									zone_cfg=$($buckycli query_dns $zone_name)
									node_1=$(jq -r -n --argjson zone_cfg "$zone_cfg" '$zone_cfg.extra.etcds[0].name')
									node_2=$(jq -r -n --argjson zone_cfg "$zone_cfg" '$zone_cfg.extra.etcds[1].name')
									node_3=$(jq -r -n --argjson zone_cfg "$zone_cfg" '$zone_cfg.extra.etcds[2].name')
									echo "The name of node 1 is $node_1."
									echo "The name of node 2 is $node_2."
									echo "The name of node 3 is $node_3."
									break;;
								"invalid" )
									echo "The zone name is invalid.Please set the following content as the dns txt record of the domain name:"
									echo $encoded_cfg
									break;;
								* ) echo "Unknown error."
									exit 1;;
							esac
						done
						zone_cfg_mode="dns"
						break
						;;
					"local" | "l" )
						echo "You have selected local mode."
						zone_cfg_mode="local"
						break
						;;
					* ) echo "Please answer dns or local.";;
				esac
			done
			;;
		* ) echo "Unknown error."
			exit 1;;
	esac

	while true; do
		echo "Please select the installation mode:"
		echo "1. All nodes are running on the current machine."
		echo "2. Each node is running on a separate machine."
		read -p "Please enter the installation mode[1/2] [default: 1]: " install_mode < /dev/tty
		if [ -z $install_mode ]; then
			install_mode="1"
		fi
		case $install_mode in
			"1" )
				echo "You have selected all mode."
				create_all

				wait_time=10
				echo "Wait ${wait_time} seconds for the zone to start"

				for ((i=1; i<=wait_time; i++))
				do
				    echo -ne "Already waited ${i} seconds\r"
				    sleep 1
				done
				import_all_config
				break;;
			"2" )
				echo "You have selected node mode."
				create_node

				ensure_etcd_cluster_health

				import_all_config
				break;;
			* ) echo "Please answer 1 or 2.";;
		esac
	done

}

create_zone_dns_cfg() {
	zone_cfg_template=$(cat <<- EOF
{
   "extra": {
       "etcds": [
           {
               "name": "$2"
           },
           {
               "name": "$3"
           },
           {
               "name": "$4"
           }
       ]
   },
   "name": "$1",
   "type": "zone",
   "version": "1.0"
}
EOF
)
    echo $zone_cfg_template
}

create_zone_local_cfg() {
	zone_cfg_template=$(cat <<- EOF
{
    "zone_id" : "$1",
    "etcd_servers":["$2","$3","$4"],
    "etcd_data_version":0
}
EOF
)
    echo $zone_cfg_template
}

create_node_identity() {
	node_identity_template=$(cat <<- EOF
	owner_zone_id = "$1"
	node_id = "$2"
	EOF
	)
	sudo echo -e "$node_identity_template" > "$3"
}

create_all() {
	while true; do
		read -p "Please enter data path [default: ~/buckyos]: " data_path < /dev/tty
		if [ -z $data_path ]; then
			data_path="~/buckyos"
		fi
		data_path=$(eval echo "$data_path")
		if sudo mkdir -p "$data_path" ; then
			echo "The data path is $data_path."
			break
		else
			echo "The data path is invalid."
		fi
	done

	ensure sudo mkdir -p "$data_path/$node_1"
	create_node_identity $zone_name $node_1 "$data_path/$node_1/node_identity.toml"
	ensure sudo mkdir -p "$data_path/$node_2"
	create_node_identity $zone_name $node_2 "$data_path/$node_2/node_identity.toml"
	ensure sudo mkdir -p "$data_path/$node_3"
	create_node_identity $zone_name $node_3 "$data_path/$node_3/node_identity.toml"

	ensure sudo mkdir -p "$data_path/$node_1/data"
	ensure sudo mkdir -p "$data_path/$node_2/data"
	ensure sudo mkdir -p "$data_path/$node_3/data"
	ensure sudo mkdir -p "$data_path/$node_1/data/gv0"
	ensure sudo mkdir -p "$data_path/$node_2/data/gv0"
	ensure sudo mkdir -p "$data_path/$node_3/data/gv0"

	ensure sudo mkdir -p "$data_path/$node_1/etcd"
	ensure sudo mkdir -p "$data_path/$node_2/etcd"
	ensure sudo mkdir -p "$data_path/$node_3/etcd"

	local zone_local_cfg=""
	local zone_local_cfg2=""
	if [ "$zone_cfg_mode" = "local" ]; then
		local zone_cfg=$(create_zone_local_cfg $zone_name $node_1 $node_2 $node_3)
		ensure sudo echo -e "$zone_cfg" > "$data_path/zone_local_cfg.json"

		zone_local_cfg="- $data_path/zone_local_cfg.json:/buckyos/${zone_name}_zone_config.json"
		zone_local_cfg2="-v $data_path/zone_local_cfg.json:/buckyos/${zone_name}_zone_config.json"
	fi

	docker_compose_template=$(cat <<- EOF
networks:
  buckyos:
    driver: bridge

services:
  $node_1:
    image: $docker_image
    container_name: $node_1
    networks:
      - buckyos
    devices:
      - /dev/fuse:/dev/fuse
    cap_add:
      - SYS_ADMIN
    security_opt:
      - apparmor=unconfined
    ports:
      - "139:139"
      - "445:445"
    volumes:
      - $data_path/$node_1/node_identity.toml:/buckyos/node_identity.toml
      - $data_path/$node_1/data:/buckyos/data
      - $data_path/$node_1/etcd:/buckyos/$node_1.etcd
      $zone_local_cfg
    tty: true
    stdin_open: true
    restart: always

  $node_2:
    image: $docker_image
    container_name: $node_2
    networks:
      - buckyos
    devices:
      - /dev/fuse:/dev/fuse
    cap_add:
      - SYS_ADMIN
    security_opt:
      - apparmor=unconfined
    volumes:
      - $data_path/$node_2/node_identity.toml:/buckyos/node_identity.toml
      - $data_path/$node_2/data:/buckyos/data
      - $data_path/$node_2/etcd:/buckyos/$node_2.etcd
      $zone_local_cfg
    tty: true
    stdin_open: true
    restart: always

  $node_3:
    image: $docker_image
    container_name: $node_3
    networks:
      - buckyos
    devices:
      - /dev/fuse:/dev/fuse
    cap_add:
      - SYS_ADMIN
    security_opt:
      - apparmor=unconfined
    ports:
      - "2379:2379"
    volumes:
      - $data_path/$node_3/node_identity.toml:/buckyos/node_identity.toml
      - $data_path/$node_3/data:/buckyos/data
      - $data_path/$node_3/etcd:/buckyos/$node_3.etcd
      $zone_local_cfg
    tty: true
    stdin_open: true
    restart: always
EOF
)
	ensure sudo echo -e "$docker_compose_template" > "$data_path/docker-compose.yml"

	if sudo docker compose version >/dev/null 2>&1; then
		current_dir=$(pwd)
		ensure cd "$data_path"
		ensure sudo docker compose up -d
		ensure cd "$current_dir"
    elif sudo docker-compose --version >/dev/null 2>&1; then
		current_dir=$(pwd)
		ensure cd "$data_path"
		ensure sudo docker-compose up -d
		ensure cd "$current_dir"
    else
	    sudo docker network create buckyos
	    sudo docker pull harbor.mynode.site:8443/library/buckyos
	    sudo docker run --restart=always -d --device /dev/fuse --cap-add SYS_ADMIN --security-opt apparmor=unconfined $zone_local_cfg2 -v $data_path/$node_1/node_identity.toml:/buckyos/node_identity.toml -v "$data_path/$node_1/data":/buckyos/data -v $data_path/$node_1/etcd:/buckyos/$node_1.etcd --name $node_1 -p 139:139 -p 445:445 --network buckyos $docker_image
	    sudo docker run --restart=always -d --device /dev/fuse --cap-add SYS_ADMIN --security-opt apparmor=unconfined $zone_local_cfg2 -v $data_path/$node_2/node_identity.toml:/buckyos/node_identity.toml -v "$data_path/$node_2/data":/buckyos/data -v $data_path/$node_2/etcd:/buckyos/$node_2.etcd --name $node_2 --network buckyos $docker_image
	    sudo docker run --restart=always -d --device /dev/fuse --cap-add SYS_ADMIN --security-opt apparmor=unconfined $zone_local_cfg2 -v $data_path/$node_3/node_identity.toml:/buckyos/node_identity.toml -v "$data_path/$node_3/data":/buckyos/data -v $data_path/$node_3/etcd:/buckyos/$node_3.etcd --name $node_3 -p 2379:2379 --network buckyos $docker_image
    fi
}

ips=""
ip_mode=""
create_node() {
	local local_node=""
	ensure_node_dns $node_1
	local node_1_ips=$ips
	local node_1_ip_mode=$ip_mode
	if [ -z "$local_node" ]; then
		local_node=$(is_local_host $node_1 $node_1_ips)
	fi
	ensure_node_dns $node_2
	local node_2_ips=$ips
	local node_2_ip_mode=$ip_mode
	if [ -z "$local_node" ]; then
		local_node=$(is_local_host $node_2 $node_2_ips)
	fi
	ensure_node_dns $node_3
	local node_3_ips=$ips
	local node_3_ip_mode=$ip_mode
	if [ -z "$local_node" ]; then
		local_node=$(is_local_host $node_3 $node_3_ips)
	fi

	while true; do
		if [ "$local_node" == "" ]; then
			read -p "Please enter node_name[$node_1/$node_2/$node_3]: " cur_node < /dev/tty
		else
			read -p "Please enter node_name[default: $local_node]: " cur_node < /dev/tty
			if [ -z "$cur_node"]; then
				cur_node=$local_node
			fi
		fi
		if [ "$cur_node" = "$node_1" ] || [ "$cur_node" = "$node_2" ] || [ "$cur_node" = "$node_3" ]; then
			echo "The node name is $cur_node."
			break
		else
			if [ -z "$local_node" ]; then
				echo "The node name must be one of the following[$node_1/$node_2/$node_3]."
			else
				echo "The node name must be $local_node."
			fi
		fi
	done

	while true; do
		read -p "Please enter data path [default: ~/buckyos]: " data_path < /dev/tty
		if [ -z $data_path ]; then
			data_path="~/buckyos"
		fi
		data_path=$(eval echo "$data_path")
		if sudo mkdir -p "$data_path" ; then
			echo "The data path is $data_path."
			break
		else
			echo "The data path is invalid."
		fi
	done

	ensure sudo mkdir -p "$data_path/$cur_node"
	create_node_identity $zone_name $cur_node "$data_path/$cur_node/node_identity.toml"
	ensure sudo mkdir -p "$data_path/$cur_node/data"
	ensure sudo mkdir -p "$data_path/$cur_node/data/gv0"
	ensure sudo mkdir -p "$data_path/$cur_node/etcd"

	local zone_local_cfg=""
	if [ "$zone_cfg_mode" = "local" ]; then
		local zone_cfg=$(create_zone_local_cfg $zone_name $node_1 $node_2 $node_3)
		ensure sudo echo -e "$zone_cfg" > "$data_path/zone_local_cfg.json"

		zone_local_cfg="-v $data_path/zone_local_cfg.json:/buckyos/${zone_name}_zone_config.json"
	fi

	local node_1_host=""
	if [[ "$node_1_ip_mode" == "manual" && "$cur_node" != "$node_1" ]]; then
		local ip_list=($node_1_ips)
	    node_1_host="--add-host $node_1:${ip_list[0]}"
	fi
	local node_2_host=""
	if [[ "$node_2_ip_mode" == "manual" && "$cur_node" != "$node_2" ]]; then
		local ip_list=($node_2_ips)
	    node_2_host="--add-host $node_2:${ip_list[0]}"
	fi
	local node_3_host=""
	if [[ "$node_3_ip_mode" == "manual" && "$cur_node" != "$node_3" ]]; then
		local ip_list=($node_3_ips)
	    node_3_host="--add-host $node_3:${ip_list[0]}"
	fi
	ensure sudo docker pull $docker_image
 	#echo "sudo docker run -d --device /dev/fuse --cap-add SYS_ADMIN --security-opt apparmor=unconfined --restart=always $node_1_host $node_2_host $node_3_host $zone_local_cfg -v "$data_path/$cur_node/node_identity.toml":/buckyos/node_identity.toml -v "$data_path/$cur_node/data":/buckyos/data -v $data_path/$cur_node/etcd:/buckyos/$cur_node.etcd  --name buckyos -h $cur_node -p 24008:24008 -p 24007:24007 -p 49152-49162:49152-49162 -p 139:139 -p 445:445 -p 2379:2379 -p 2380:2380 $docker_image"
	ensure sudo docker run -d --device /dev/fuse --cap-add SYS_ADMIN --security-opt apparmor=unconfined --restart=always $node_1_host $node_2_host $node_3_host $zone_local_cfg -v "$data_path/$cur_node/node_identity.toml":/buckyos/node_identity.toml -v "$data_path/$cur_node/data":/buckyos/data -v $data_path/$cur_node/etcd:/buckyos/$cur_node.etcd  --name buckyos -h $cur_node -p 24008:24008 -p 24007:24007 -p 49152-49162:49152-49162 -p 139:139 -p 445:445 -p 2379:2379 -p 2380:2380 $docker_image
}

ensure_node_dns() {
	local node_name=$1
	echo "Checking the domain $node_name IP Address..."
	ip_mode="dns"
	ips=$(nslookup $node_name | grep 'Address:' | tail -n +2 | awk '{print $2}')
	if [ -z "$ips" ]; then
		while true; do
			read -p "The domain $node_name can't resolve IP address.Do you want to configure dns or enter the ip address manually?[dns(d)/manual(m)] [default: manual]: " dns_mode < /dev/tty
			if [ -z $dns_mode ]; then
				dns_mode="manual"
			fi
			case $dns_mode in
				"dns" | "d" )
					while true; do
						ips=$(nslookup $node_name | grep 'Address:' | tail -n +2 | awk '{print $2}')
						if [ -z "$ips" ]; then
				            echo "The domain $node_name is not configured with DNS."
				        else
				            break
				        fi
					done
					break;;
				"manual" | "m")
					while true; do
						read -p "Please enter an IP address: " ip < /dev/tty

		                if is_valid_ip "$ip"; then
		                    ips=$ip
		                    break
		                else
		                    echo "The IP address $ip is invalid."
		                fi
					done
					ip_mode="manual"
					break;;
				* ) echo "Please answer dns or manual.";;
			esac
		done
	fi
}

is_local_host() {
	local node_name=$1
	local domain_ips=($2)
	local local_ips=($(hostname -I))
	for domain_ip in "${domain_ips[@]}"; do
		for local_ip in "${local_ips[@]}"; do
			if [ "$domain_ip" = "$local_ip" ]; then
				echo "$node_name"
				return
			fi
		done
	done
	echo ""
}

import_all_config() {
	zone_node_config_template=$(cat <<- EOF
{
  "$node_1": {
    "services": {
      "glusterfs": {
        "target_state": "Running",
        "pkg_id": "glusterfs",
        "version": "*",
        "operations": {
	        "status": {
	            "command": "status.sh",
	            "params": [
	                "gv0",
	                "/mnt/glusterfs"
	            ]
	        },
	        "start": {
	            "command": "start.sh",
	            "params": [
	                "$node_1",
	                "gv0",
	                "/buckyos/data/gv0",
	                "/mnt/glusterfs",
	                "$node_2 $node_3",
	                "create_volume"
	            ]
	        },
	        "stop": {
	            "command": "stop.sh",
	            "params": [
	                "gv0",
	                "/mnt/glusterfs"
	            ]
	        },
	        "deploy": {
	            "command": "deploy.sh",
	            "params": [
	                "--gluster"
	            ]
	        }
        }
      },
     "samba": {
       "target_state": "Running",
       "pkg_id": "smb_service",
       "version": "*",
       "operations": {
         "deploy": {
           "command": "deploy.sh"
         },
         "status": {
           "command": "status.sh",
           "params": ["--status"]
         },
         "start": {
           "command": "start.sh",
           "params": ["/mnt/glusterfs"]
         }
       }
     }
    }
  },
  "$node_2": {
    "services": {
      "glusterfs": {
        "target_state": "Running",
        "pkg_id": "glusterfs",
        "version": "*",
        "operations": {
	        "status": {
	            "command": "status.sh",
	            "params": [
	                "gv0",
	                "/mnt/glusterfs"
	            ]
	        },
	        "start": {
	            "command": "start.sh",
	            "params": [
	                "$node_2",
	                "gv0",
	                "/buckyos/data/gv0",
	                "/mnt/glusterfs",
	                "$node_1 $node_3"
	            ]
	        },
	        "stop": {
	            "command": "stop.sh",
	            "params": [
	                "gv0",
	                "/mnt/glusterfs"
	            ]
	        },
	        "deploy": {
	            "command": "deploy.sh",
	            "params": [
	                "--gluster"
	            ]
	        }
        }
      }
    }
  },
  "$node_3": {
    "services": {
      "glusterfs": {
        "target_state": "Running",
        "pkg_id": "glusterfs",
        "version": "*",
        "operations": {
	        "status": {
	            "command": "status.sh",
	            "params": [
	                "gv0",
	                "/mnt/glusterfs"
	            ]
	        },
	        "start": {
	            "command": "start.sh",
	            "params": [
	                "$node_3",
	                "gv0",
	                "/buckyos/data/gv0",
	                "/mnt/glusterfs",
	                "$node_1 $node_2"
	            ]
	        },
	        "stop": {
	            "command": "stop.sh",
	            "params": [
	                "gv0",
	                "/mnt/glusterfs"
	            ]
	        },
	        "deploy": {
	            "command": "deploy.sh",
	            "params": [
	                "--gluster"
	            ]
	        }
        }
      }
    }
  }
}
EOF
)
	ensure sudo echo -e "$zone_node_config_template" > "$data_path/zone_node_config.yml"
	ensure $buckycli import_zone_config -f "$data_path/zone_node_config.yml"
}

ensure_etcd_cluster_health() {
	while true; do
		local cluster_status=$($buckycli check_etcd_cluster)
		if [ "$cluster_status" = "The etcd cluster is healthy" ]; then
			echo "The etcd cluster is healthy."
			break
		else
			echo "The etcd cluster is unhealthy.Please check the other two node statuses."
			sleep 1
		fi
	done
}

need_cmd() {
    if ! check_cmd "$1"; then
        err "need '$1' (command not found)"
    fi
}

check_cmd() {
    command -v "$1" > /dev/null 2>&1
}

# Run a command that should never fail. If the command fails execution
# will immediately terminate with an error showing the failing
# command.
ensure() {
    if ! "$@"; then echo "command failed: $*"; fi
}

# This is just for indicating that commands' results are being
# intentionally ignored. Usually, because it's being executed
# as part of error handling.
ignore() {
    "$@"
}

is_valid_ip() {
    local ip=$1
    local valid_ip_regex="^([0-9]{1,3}\.){3}[0-9]{1,3}$"

    if [[ $ip =~ $valid_ip_regex ]]; then
        # Split the IP address into its components
        IFS='.' read -r -a octets <<< "$ip"

        # Check each octet to ensure it is between 0 and 255
        for octet in "${octets[@]}"; do
            if (( octet < 0 || octet > 255 )); then
                return 1
            fi
        done

        return 0
    else
        return 1
    fi
}

main "$@" || exit 1
