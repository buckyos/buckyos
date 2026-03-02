
# Control panel

原型参考：
https://spray-jargon-85834573.figma.site/

https://hug-reach-51789548.figma.site







# Service
cargo run -p control_panel

## Files integration

- Files 页面属于 Control Panel Web 的内嵌模块（不是独立前端工程）。
- Files API 由 `control_panel` 服务统一提供（`/api/*`），并在后端转发到内嵌 file manager。
- 本地前端开发目录：`src/frame/control_panel/web`。
