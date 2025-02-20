#include "TrayMenu.h"
#include <map>
#include <set>

static std::set<TrayMenu*> s_objs;
static std::map<HWND, TrayMenu*> s_hwnd_objs;

#define MSG_POPUP_MENU (WM_USER + 1)
#define ID_TRAY_EXIT (WM_USER + 2)
#define ID_TRAY_ABOUT (WM_USER + 3)
#define ID_TRAY_HOMEPAGE (WM_USER + 4)
#define ID_TRAY_START (WM_USER + 5)
#define ID_TRAY_APP_SUBMENU_BEGIN (WM_USER + 6)

TrayMenu::TrayMenu(HWND hwnd) {
	this->m_seq = 0;
	this->m_app_list_seq = 0;
	this->m_is_popup = false;
	this->m_menu_id_homepage = ID_TRAY_HOMEPAGE;
	this->m_menu_id_start = ID_TRAY_START;
	this->m_menu_id_about = ID_TRAY_ABOUT;
	this->m_menu_id_exit = ID_TRAY_EXIT;
	this->m_app_menu_id_begin = ID_TRAY_APP_SUBMENU_BEGIN;
	this->m_menu_proc_map[ID_TRAY_HOMEPAGE] = proc_open_homepage;
	this->m_menu_proc_map[ID_TRAY_START] = proc_start;
	this->m_menu_proc_map[ID_TRAY_ABOUT] = proc_about;
	this->m_menu_proc_map[ID_TRAY_EXIT] = proc_exit;
	this->m_display_pos = POINT{ 0, 0 };

	s_objs.insert(this);

	WNDCLASSEX wc = { 0 };
	wc.cbSize = sizeof(WNDCLASSEX);
	wc.lpfnWndProc = TrayMenuWndProc;
	wc.hInstance = GetModuleHandle(NULL);
	wc.lpszClassName = L"tray-menu";

	RegisterClassEx(&wc);

	HWND hwndChild = CreateWindowEx(
		0,
		wc.lpszClassName,
		L"",
		WS_OVERLAPPED,
		0, 0, 0, 0,
		hwnd,
		NULL,
		GetModuleHandle(NULL),
		NULL
	);
	this->m_hwnd = hwndChild;

	s_hwnd_objs.insert(std::pair<HWND, TrayMenu*>(hwndChild, this));
}

TrayMenu::~TrayMenu() {
	std::set<TrayMenu*>::const_iterator it = s_objs.find(this);
	if (it != s_objs.end()) {
		s_objs.erase(it);
	}
	std::map<HWND, TrayMenu*>::const_iterator hwnd_it = s_hwnd_objs.find(this->m_hwnd);
	if (hwnd_it != s_hwnd_objs.end()) {
		s_hwnd_objs.erase(hwnd_it);
		DestroyWindow(this->m_hwnd);
	}
}

void TrayMenu::popup(POINT& display_pos, bool is_buckyos_running) {
	this->m_seq++;
	this->m_is_popup = false;
	this->m_display_pos.x = display_pos.x;
	this->m_display_pos.y = display_pos.y;
	this->m_is_buckyos_running = is_buckyos_running;

	list_application(this->m_seq, list_application_callback, (void*)this);
}

void TrayMenu::list_application_callback(char is_success, ::ApplicationInfo* apps, int32_t app_count, int seq, void* user_data) {
	TrayMenu* self = (TrayMenu*)user_data;
	std::set<TrayMenu*>::const_iterator it = s_objs.find(self);
	if (it == s_objs.end()) {
		return;
	}

	if (is_success && seq > self->m_app_list_seq) {
		self->m_apps.clear();
		for (int i = 0; i < app_count; i++) {
			::ApplicationInfo* app = &apps[i];

			int name_size = (int)strlen(app->name) * 3;
			LPWSTR name = (LPWSTR)malloc(name_size);
			name_size = MultiByteToWideChar(
				CP_UTF8,
				0,
				app->name,
				-1,
				name,
				name_size
			);
			name[name_size] = L'\0';

			LPWSTR icon_path = NULL;
			if (app->icon_path) {
				int icon_path_size = (int)strlen(app->icon_path) * 3;
				icon_path = (LPWSTR)malloc(icon_path_size);
				icon_path_size = MultiByteToWideChar(
					CP_UTF8,
					0,
					app->icon_path,
					-1,
					icon_path,
					icon_path_size
				);
				icon_path[icon_path_size] = L'\0';
			}

			int home_page_url_size = (int)strlen(app->home_page_url) * 3;
			LPWSTR home_page_url = (LPWSTR)malloc(home_page_url_size);
			home_page_url_size = MultiByteToWideChar(
				CP_UTF8,
				0,
				app->home_page_url,
				-1,
				home_page_url,
				home_page_url_size
			);
			home_page_url[home_page_url_size] = L'\0';

			// int start_cmd_size = (int)strlen(app->start_cmd) * 3;
			// LPWSTR start_cmd = (LPWSTR)malloc(start_cmd_size);
			// start_cmd_size = MultiByteToWideChar(
			// 	CP_UTF8,
			// 	0,
			// 	app->start_cmd,
			// 	-1,
			// 	start_cmd,
			// 	start_cmd_size
			// );
			// start_cmd[start_cmd_size] = L'\0';

			// int stop_cmd_size = (int)strlen(app->stop_cmd) * 3;
			// LPWSTR stop_cmd = (LPWSTR)malloc(stop_cmd_size);
			// stop_cmd_size = MultiByteToWideChar(
			// 	CP_UTF8,
			// 	0,
			// 	app->stop_cmd,
			// 	-1,
			// 	stop_cmd,
			// 	(int)stop_cmd_size
			// );
			// stop_cmd[stop_cmd_size] = L'\0';

			self->m_apps.push_back(ApplicationInfo {
				app->id,
				name,
				icon_path? icon_path : L"",
				home_page_url,
				app->is_running == 1,
			});

			free(name);
			if (icon_path) free(icon_path);
			free(home_page_url);
		}
	}

	PostMessageW(self->m_hwnd, MSG_POPUP_MENU, 0, (LPARAM)self);
}

void TrayMenu::do_popup_menu() {
	if (this->m_is_popup) {
		return;
	}

	this->m_is_popup = true;
	this->m_menu_apps = this->m_apps;
	this->m_is_buckyos_running_menu = this->m_is_buckyos_running;

	HMENU hMenu = CreatePopupMenu();

	InsertMenuW(hMenu, -1, MF_BYPOSITION, this->m_menu_id_homepage, L"Home page");

	UINT_PTR app_submenu_id = this->m_app_menu_id_begin;
	for (int i = 0; i < this->m_menu_apps.size(); i++) {
		ApplicationInfo& app = this->m_menu_apps.at(i);
		HMENU hSubMenu = CreatePopupMenu();
		InsertMenuW(hSubMenu, -1, MF_BYPOSITION, app_submenu_id++, L"Home page");
		if (app.is_running) {
			InsertMenuW(hSubMenu, -1, MF_BYPOSITION, app_submenu_id++, L"Stop");
		}
		else {
			InsertMenuW(hSubMenu, -1, MF_BYPOSITION, app_submenu_id++, L"Start");
		}
		InsertMenu(hMenu, -1, MF_BYPOSITION | MF_POPUP, (UINT_PTR)hSubMenu, app.name.c_str());
	}

	if (this->m_is_buckyos_running) {
		InsertMenuW(hMenu, -1, MF_BYPOSITION, this->m_menu_id_start, L"Stop");
	} else {
		InsertMenuW(hMenu, -1, MF_BYPOSITION, this->m_menu_id_start, L"Start");
	}

	InsertMenuW(hMenu, -1, MF_BYPOSITION, this->m_menu_id_about, L"About");
	InsertMenuW(hMenu, -1, MF_BYPOSITION, this->m_menu_id_exit, L"Exit");
	SetForegroundWindow(this->m_hwnd);
	TrackPopupMenu(hMenu, TPM_BOTTOMALIGN | TPM_LEFTALIGN, this->m_display_pos.x, this->m_display_pos.y, 0, this->m_hwnd, NULL);

	DestroyMenu(hMenu);
}

void TrayMenu::proc_open_homepage(TrayMenu* self) {
    NodeInfomation* node_info = get_node_info();
	
	if (node_info->home_page_url) {
		HINSTANCE handle = ShellExecuteA(
				NULL,
				"open",
				node_info->home_page_url,
				NULL,
				NULL,
				SW_SHOWNORMAL
			);
		CloseHandle(handle);
		
	}

    free_node_info(node_info);
}

void TrayMenu::proc_start(TrayMenu* self) {
	if (self->m_is_buckyos_running) {
		stop_buckyos();
	}
	else {
		start_buckyos();
	}
}

void TrayMenu::proc_about(TrayMenu* self) {
	MessageBoxW(self->m_hwnd, L"BuckyOS about", L"BuckyOS", MB_OK);
}

void TrayMenu::proc_exit(TrayMenu* self) {
	PostQuitMessage(0);
}

bool TrayMenu::on_command(UINT_PTR menu_id) {
	void (*proc)(TrayMenu*) = this->m_menu_proc_map[menu_id];
	if (proc) {
		proc(this);
		return true;
	}
	else {
		if (menu_id >= this->m_app_menu_id_begin && menu_id < this->m_app_menu_id_begin + this->m_menu_apps.size() * 2) {
			ApplicationInfo& app = this->m_menu_apps.at((menu_id - this->m_app_menu_id_begin) / 2);
			int app_cmd = (menu_id - this->m_app_menu_id_begin) % 2;
			if (app_cmd == 0) {
				HINSTANCE handle = ShellExecute(
					NULL,
					L"open",
					app.home_page_url.c_str(),
					NULL,
					NULL,
					SW_SHOWNORMAL // 窗口显示状态
				);

				CloseHandle(handle);
			} else if (app_cmd == 1) {
				if (app.is_running) {
					stop_app((char*)app.id.data());
				}
				else {
					start_app((char*)app.id.data());
				}
			}
			return true;
		}
	}

	return false;
}

LRESULT CALLBACK TrayMenu::TrayMenuWndProc(HWND hwnd, UINT msg, WPARAM wParam, LPARAM lParam) {
	switch (msg) {
		case MSG_POPUP_MENU:
			{
				TrayMenu* self = (TrayMenu*)lParam;
				self->do_popup_menu();
			}
			break;
		case WM_COMMAND:
			{
				std::map<HWND, TrayMenu*>::const_iterator it = s_hwnd_objs.find(hwnd);
				if (it != s_hwnd_objs.end()) {
					TrayMenu* self = it->second;
					self->on_command(LOWORD(wParam));
				}
			}
			break;
		default:
			return DefWindowProc(hwnd, msg, wParam, lParam);
	}
	return 0;
}
