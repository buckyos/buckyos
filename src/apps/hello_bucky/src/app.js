const http = require('http');

const server = http.createServer((req, res) => {
    res.writeHead(200, {'Content-Type': 'text/plain'});
    res.end('hello BuckyOS!\n');
});

server.listen(80, '0.0.0.0', () => {
    console.log('hello BuckyOS http server running at http://0.0.0.0:80/');
}); 
