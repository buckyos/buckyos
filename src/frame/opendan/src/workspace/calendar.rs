// 现状: 鼓励使用更通用的agent memory来管理agent的日程等信息，暂时不在workspace中单独设计一个日历模块
// 在workspace中，包含一个agent自己的日历，用来帮助agent进行日程管理
// 大部分情况下，该日常管理主要为agent的主人服务，但agent也可以通过日历来管理自己的日程
// 基本接口: list_events / serach_events / add_event / update_event / delete_event
// 关键参数: subjects,title, content (可选), start_time, end_time（可选）, location
// 注意处理时间的时区问题
// start_time 支持自然语言的，比如 每个工作周的第一天，如果没有假期就是周一，有假期就是周二 （通过条件区分了是单个event，还是系列event)
// subjects 应该是一个did列表，也允许用电子邮件/手机号等方式来标识一个人，通常是在contac_mgr中存在的用户
// calender的主体是agent自己,因此应该以“Agent助理的视角”来记录event,比如 一个event是周会，那么description可以是需要提前3小时提醒xxx准备会议材料，或者需要提前1小时提醒xxx准备会议室等