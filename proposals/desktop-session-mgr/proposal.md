

功能： 
username + ui_session_id + state_key => ui_state_value (可以读写)
也可以进一步通过json_path来读写ui_state中的部分数据

list all ui_session_id for a username


保存在system_config中
/config/users/$userid/settings/ui/$ui_session_id/$state_key => $state_value

实际使用的$state_key 是：
- appearance
- window_layout
- app_items_layout
- widgets_layout
