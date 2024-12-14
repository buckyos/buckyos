#pragma once

#include <stdint.h>

extern "C" {

    typedef enum {
        Running = 0,
        Stopped = 1,
        NotActive = 2,
        NotInstall = 3,
        Failed = 4,
    } BuckyStatus;

    typedef void * BuckyStatusScaner;

    BuckyStatusScaner bucky_status_scaner_scan(void (*on_status_changed_callback)(BuckyStatus new_status, BuckyStatus old_status, void* userdata), void* userdata, void* hwnd);
    void bucky_status_scaner_stop(BuckyStatusScaner scaner);

    typedef struct ApplicationInfo {
        const char* name;
        const char* icon_path;
        const char* home_page_url;
        const char* start_cmd;
        const char* stop_cmd;
        char is_running;
    } ApplicationInfo;
    
    void list_application(int32_t seq, void (*callback)(char is_success, ApplicationInfo* apps, int32_t app_count,  int32_t seq, void* user_data), void* userdata);

    typedef struct NodeInfomation {
        const char* node_id;
        const char* home_page_url;
    };

    NodeInfomation* get_node_info();
    void free_node_info(NodeInfomation* info);
}