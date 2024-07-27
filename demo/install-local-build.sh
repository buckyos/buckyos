cargo build --release

sudo docker build -t buckyos-build .

sudo ./install.sh --cli=target/release/buckycli --docker_image=buckyos-build