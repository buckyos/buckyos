from http.server import BaseHTTPRequestHandler, HTTPServer
import json

class CustomHTTPHandler(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.send_header('Content-type', 'application/json')
        self.end_headers()

        response = {
            'message': 'GET请求成功',
            'path': self.path,
            'method': 'GET'
        }
        self.wfile.write(json.dumps(response).encode())

    def do_POST(self):
        content_length = int(self.headers['Content-Length'])
        post_data = self.rfile.read(content_length)

        self.send_response(201)
        self.send_header('Content-type', 'application/json')
        self.end_headers()

        response = {
            'message': 'POST请求成功',
            'path': self.path,
            'received_data': post_data.decode(),
            'method': 'POST'
        }

        self.wfile.write(json.dumps(response).encode())

if __name__ == '__main__':
    server_address = ('', 8000)
    httpd = HTTPServer(server_address, CustomHTTPHandler)
    print("服务器运行在 http://localhost:8000")
    httpd.serve_forever()
