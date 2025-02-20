#include <windows.h>
#include <vector>
#include "SystemState.h"
#include "../ffi_extern.h"
#include "process_kits.h"

extern "C" void entry();

int WINAPI wWinMain(HINSTANCE hInstance, HINSTANCE, LPWSTR, int nShowCmd) {
	entry();
	return 0;
}


BuckyStatusScaner bucky_status_scaner_scan(void (*on_status_changed_callback)(BuckyStatus new_status, BuckyStatus old_status, void* userdata), void* userdata, void* hwnd) {
	HWND w_hwnd = (HWND)hwnd;
	return (BuckyStatusScaner)(new SystemState(on_status_changed_callback, userdata, w_hwnd));
}

void bucky_status_scaner_stop(BuckyStatusScaner scaner) {
	SystemState *ptr = (SystemState*)scaner;
	delete ptr;
}

void list_application(int32_t seq, void (*callback)(char is_success, ApplicationInfo* apps, int32_t app_count,  int32_t seq, void* user_data), void* userdata) {
	std::vector<ApplicationInfo> apps;
	{
		ApplicationInfo app;
		app.id = "app 1";
		app.name = "app 1";
        app.icon_path = NULL;
        app.home_page_url = "https://www.qq.com";
		app.is_running = true;
		apps.push_back(app);
	}
	{
		ApplicationInfo app;
		app.id = "app 1";
		app.name = "app 2";
        app.icon_path = NULL;
        app.home_page_url = "https://www.qq.com";
		app.is_running = false;
		apps.push_back(app);
	}

	callback(true, apps.data(), apps.size(), seq, userdata);
}

NodeInfomation info = NodeInfomation {
	NULL,
	"http://www.baidu.com"
};

NodeInfomation* get_node_info() {
	return &info;
}

void free_node_info(NodeInfomation* info) {

}

void start_buckyos() {
	MessageBoxW(NULL, L"BuckyOS started", L"BuckyOS", MB_OK);
}

void stop_buckyos() {
	std::set<std::wstring> all_process_set;
	for (int i = 0; i < sizeof(buckyos_process) / sizeof(buckyos_process[0]); i++) {
		all_process_set.insert(buckyos_process[i]);
	}

	std::map<std::wstring, DWORD> exist_process_map;
	std::set<std::wstring> not_exist_process_set;
	if (!find_process_by_name(all_process_set, exist_process_map, not_exist_process_set)) {
		MessageBoxW(NULL, L"BuckyOS stop failed", L"BuckyOS", MB_OK);
		return;
	}

	for (std::map<std::wstring, DWORD>::const_iterator it = exist_process_map.begin(); it != exist_process_map.end(); it++) {
		kill_process_by_id(it->second);
		MessageBoxW(NULL, L"BuckyOS stopped", L"BuckyOS", MB_OK);
	}
}

void start_app(char* app_id) {
	MessageBoxA(NULL, "app started", app_id, MB_OK);
}

void stop_app(char* app_id) {
	MessageBoxA(NULL, "app stopped", app_id, MB_OK);
}