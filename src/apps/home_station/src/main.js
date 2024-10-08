// 引入 http 模块
const http = require('http');

// 创建服务器
const server = http.createServer((req, res) => {
  // 设置响应头，表示内容类型为纯文本
  res.writeHead(200, { 'Content-Type': 'text/plain' });
  
  // 根据请求的 URL 返回不同的响应
  if (req.url === '/') {
    res.write('Hello, buckyos!');
  } else if (req.url === '/about') {
    res.write('This is a simple Node.js web server');
  } else {
    res.write('404: Not Found');
  }
  
  // 结束响应
  res.end();
});

// 设置服务器监听的端口
const port = 20080;
server.listen(port, "0.0.0.0", () => {
  console.log(`Server running at http://0.0.0.0:${port}`);
});
