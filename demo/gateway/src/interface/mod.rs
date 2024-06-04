
/*
Register services via http protocol

/service/upstream
/service/proxy/socks5
/service/proxy/forward

POST /service/upstream
{
    "id": "id",
    "protocol": "tcp",
    "addr": "127.0.0.1",
    "port": 2000,
    "type": "tcp"
}

Delete /service/upstream
{
    "id": "id"
}

POST /service/proxy/socks5
{
    "id": "id",
    "addr": "127.0.0.1",
    "port": 2000,
    "type": "socks5"
}

Delete /service/proxy/socks5
{
    "id": "id"
}

POST /service/proxy/forward
{
    "id": "id",
    "addr": "127.0.0.1",
    "port": 2000,
    "protocol": "tcp",
    "target_device": "device_id",
    "target_port": 2000
    "type": "forward"
}

Delete /service/proxy/forward
{
    "id": "id"
}
*/

mod server;
