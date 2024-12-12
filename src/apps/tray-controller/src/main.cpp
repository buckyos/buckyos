#include <windows.h>
#include <vector>
#include "SystemState.h"
#include "ffi_extern.h"

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

void list_application(int32_t seq, void (*callback)(bool is_success, ApplicationInfo* apps, int32_t app_count,  int32_t seq, void* user_data), void* userdata) {
	std::vector<ApplicationInfo> apps;
	{
		ApplicationInfo app;
		app.name = "app 1";
        app.icon_path = NULL;
        app.home_page_url = "https://www.qq.com";
        app.start_cmd = "notepad.exe";
        app.stop_cmd = "notepad.exe";
		app.is_running = true;
		apps.push_back(app);
	}
	{
		ApplicationInfo app;
		app.name = "app 2";
        app.icon_path = NULL;
        app.home_page_url = "https://www.qq.com";
        app.start_cmd = "notepad.exe";
        app.stop_cmd = "notepad.exe";
		app.is_running = false;
		apps.push_back(app);
	}

	callback(true, apps.data(), apps.size(), seq, userdata);
}