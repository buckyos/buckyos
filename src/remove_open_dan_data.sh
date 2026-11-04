for path in /opt/buckyos/agents /opt/buckyos/logs /opt/buckyos/data; do
  rm -rf "$path" 2>/dev/null || sudo rm -rf "$path"
done
