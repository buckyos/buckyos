#!/usr/bin/env python3
import argparse
import asyncio
import ssl


async def run(args):
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.minimum_version = ssl.TLSVersion.TLSv1_2
    context.load_cert_chain(args.cert, args.key)

    async def serve(reader, writer):
        remote = None
        tasks = []
        try:
            upstream, remote = await asyncio.open_connection("127.0.0.1", args.backend_port)

            async def copy(source, destination):
                while data := await source.read(65536):
                    destination.write(data)
                    await destination.drain()

            tasks = [asyncio.create_task(copy(reader, remote)), asyncio.create_task(copy(upstream, writer))]
            await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
        except (OSError, ConnectionError):
            pass
        finally:
            for task in tasks:
                task.cancel()
            await asyncio.gather(*tasks, return_exceptions=True)
            writer.close()
            if remote:
                remote.close()

    server = await asyncio.start_server(serve, args.listen, args.port, ssl=context)
    print(f"TLS test proxy listening on {args.listen}:{args.port}", flush=True)
    async with server:
        await server.serve_forever()


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--listen", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=14443)
    parser.add_argument("--backend-port", type=int, default=14020)
    parser.add_argument("--cert", required=True)
    parser.add_argument("--key", required=True)
    asyncio.run(run(parser.parse_args()))
