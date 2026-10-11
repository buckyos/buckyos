# OpenDAN Agent 通知人 TODO

日期：2026-10-11

状态：占位，方案待用户专门讨论。在此之前，各处不要自行实现“Agent 主动通知人”的投递。

依赖本文的地方：

- [Goal Session TODO](opendan-goal-session-todo.md)：Goal 的 questions、提醒和需要人决定的事项。

相关的现有能力（只列出，供讨论参考，不是方案）：workflow 计划任务的 `remind` 目标（`workflow.send_message`，经 msg-center 发给 owner）；`dcrontab` CLI（目前不在 Session 的 PATH 上，owner 格式有问题）。
