#!/bin/bash

echo "Number of arguments: $#"

count=1

for arg in "$@"
do
	echo -e "$arg" > "/etc/nginx/conf.d/server_$count.conf"
	count=$((count + 1))
done

sudo systemctl start nginx
