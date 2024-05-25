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

	sudo apt-get install curl jq -y

	# buckycli="/mnt/f/work/buckyos/demo/target/release/buckycli"
	buckycli="/usr/local/bin/buckycli"
	echo "Downloading buckycli..."
	ensure sudo curl -z "$buckycli" --progress-bar -o "$buckycli" https://cache.mynode.site/buckycli
	sudo chmod +x "$buckycli"

	docker_image="harbor.mynode.site:8443/library/buckyos:latest"
# 	docker_image="buckyos:latest"

	read -p "Please enter your zone name: " zone_name < /dev/tty

	while true; do
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
			"invalid")
				echo "The zone name is invalid."
				read -p "Please enter the name of node 1: " node_1 < /dev/tty
				read -p "Please enter the name of node 2: " node_2 < /dev/tty
				read -p "Please enter the name of node 3: " node_3 < /dev/tty
				zone_cfg=$(create_zone_cfg $zone_name $node_ $node_2 $node_3)
				echo $zone_cfg > /tmp/zone_cfg.json
				$buckycli encode_dns -f /tmp/zone_cfg.json
				encoded_cfg=$($buckycli encode_dns -f /tmp/zone_cfg.json)
				echo $encoded_cfg
				;;
			* ) echo "Unknown error."
				exit 1;;
		esac
	done

	while true; do
		echo "Please select the installation mode:"
		echo "1. All nodes are running on the current machine."
		echo "2. Each node is running on a separate machine."
		read -p "Please enter the installation mode[1/2]: " install_mode < /dev/tty
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
				break;;
			* ) echo "Please answer 1 or 2.";;
		esac
	done

}
create_zone_cfg() {
	zone_cfg_template="{
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
                      "
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
		read -p "Please enter data path: " data_path < /dev/tty
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

	ensure sudo mkdir -p "$data_path/$node_1/etcd"
	ensure sudo mkdir -p "$data_path/$node_2/etcd"
	ensure sudo mkdir -p "$data_path/$node_3/etcd"

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
    ports:
      - "139:139"
      - "445:445"
    volumes:
      - $data_path/$node_1/node_identity.toml:/buckyos/node_identity.toml
      - $data_path/$node_1/data:/buckyos/data
      - $data_path/$node_1/etcd:/buckyos/$node_1.etcd
    tty: true
    stdin_open: true
    restart: always

  $node_2:
    image: $docker_image
    container_name: $node_2
    networks:
      - buckyos
    volumes:
      - $data_path/$node_2/node_identity.toml:/buckyos/node_identity.toml
      - $data_path/$node_2/data:/buckyos/data
      - $data_path/$node_2/etcd:/buckyos/$node_2.etcd
    tty: true
    stdin_open: true
    restart: always

  $node_3:
    image: $docker_image
    container_name: $node_3
    networks:
      - buckyos
    ports:
      - "2379:2379"
    volumes:
      - $data_path/$node_3/node_identity.toml:/buckyos/node_identity.toml
      - $data_path/$node_3/data:/buckyos/data
      - $data_path/$node_3/etcd:/buckyos/$node_3.etcd
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
	    docker network create buckyos
	    docker pull harbor.mynode.site:8443/library/buckyos
	    docker run --restart=always -d -v $data_path/$node_1/node_identity.toml:/buckyos/node_identity.toml -v "$data_path/$node_1/data":/buckyos/data -v $data_path/$node_1/etcd:/var/lib/etcd --name $node_1 -p 139:139 -p 445:445 --network buckyos $docker_image
	    docker run --restart=always -d -v $data_path/$node_2/node_identity.toml:/buckyos/node_identity.toml -v "$data_path/$node_2/data":/buckyos/data -v $data_path/$node_2/etcd:/var/lib/etcd --name $node_2 --network buckyos $docker_image
	    docker run --restart=always -d -v $data_path/$node_3/node_identity.toml:/buckyos/node_identity.toml -v "$data_path/$node_3/data":/buckyos/data -v $data_path/$node_3/etcd:/var/lib/etcd --name $node_3 -p 2379:2379 --network buckyos $docker_image
    fi
}

create_node() {
	while true; do
		read -p "Please enter node_name[$node_1/$node_2/$node_3]: " cur_node < /dev/tty
		if [ "$cur_node" = "$node_1" ] || [ "$cur_node" = "$node_2" ] || [ "$cur_node" = "$node_3" ]; then
			echo "The node name is $cur_node."
			break
		else
			echo "The node name must be one of the following[$node_1/$node_2/$node_3]."
		fi
	done

	while true; do
		read -p "Please enter data path: " data_path < /dev/tty
		data_path=$(eval echo "$data_path")
		if mkdir -p "$data_path" ; then
			echo "The data path is $data_path."
			break
		else
			echo "The data path is invalid."
		fi
	done

	ensure mkdir -p "$data_path/$cur_node"
	create_node_identity $zone_name $cur_node "$data_path/$node_1/node_identity.toml"
	ensure mkdir -p "$data_path/$cur_node/data"
	ensure mkdir -p "$data_path/$cur_node/etcd"

	ensure sudo docker pull $docker_image
	ensure sudo docker run -d --init --restart=always -v "$data_path/$node_1/node_identity.toml":/buckyos/node_identity.toml -v "$data_path/$cur_node/data":/buckyos/data -v $data_path/$cur_node/etcd:/var/lib/etcd  --name gateway -p 2379:2379 -p 2380:2380 $docker_image
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
	                "/buckyos/data",
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
           "command": "deploy.sh",
           "params": ["/mnt/glusterfs"]
         },
         "status": {
           "command": "status.sh",
           "params": ["--status"]
         },
         "start": {
           "command": "start.sh",
           "params": []
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
	                "/buckyos/data",
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
	                "/buckyos/data",
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
    if ! "$@"; then err "command failed: $*"; fi
}

# This is just for indicating that commands' results are being
# intentionally ignored. Usually, because it's being executed
# as part of error handling.
ignore() {
    "$@"
}

main "$@" || exit 1
