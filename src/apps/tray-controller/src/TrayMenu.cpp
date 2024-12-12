#include "TrayMenu.h"
#include <map>
#include <set>
#include "process_kits.h"

static std::set<TrayMenu*> s_objs;

TrayMenu::TrayMenu(HWND hwnd, UINT_PTR menu_id_homepage, UINT_PTR menu_id_start, UINT_PTR menu_id_about, UINT_PTR menu_id_exit, UINT_PTR app_menu_id_begin) {
	this->m_hwnd = hwnd;
	this->m_seq = 0;
	this->m_app_list_seq = 0;
	this->m_is_popup = false;
	this->m_menu_id_homepage = menu_id_homepage;
	this->m_menu_id_start = menu_id_start;
	this->m_menu_id_about = menu_id_about;
	this->m_menu_id_exit = menu_id_exit;
	this->m_app_menu_id_begin = app_menu_id_begin;
	this->m_menu_proc_map[menu_id_homepage] = proc_open_homepage;
	this->m_menu_proc_map[menu_id_start] = proc_start;
	this->m_menu_proc_map[menu_id_about] = proc_about;
	this->m_menu_proc_map[menu_id_exit] = proc_exit;
	this->m_display_pos = POINT{ 0, 0 };

	s_objs.insert(this);
}

TrayMenu::~TrayMenu() {
	std::set<TrayMenu*>::const_iterator it = s_objs.find(this);
	if (it != s_objs.end()) {
		s_objs.erase(it);
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

void TrayMenu::list_application_callback(bool is_success, ::ApplicationInfo* apps, int32_t app_count, int seq, void* user_data) {
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

			int start_cmd_size = (int)strlen(app->start_cmd) * 3;
			LPWSTR start_cmd = (LPWSTR)malloc(start_cmd_size);
			start_cmd_size = MultiByteToWideChar(
				CP_UTF8,
				0,
				app->start_cmd,
				-1,
				start_cmd,
				start_cmd_size
			);
			start_cmd[start_cmd_size] = L'\0';

			int stop_cmd_size = (int)strlen(app->stop_cmd) * 3;
			LPWSTR stop_cmd = (LPWSTR)malloc(stop_cmd_size);
			stop_cmd_size = MultiByteToWideChar(
				CP_UTF8,
				0,
				app->stop_cmd,
				-1,
				stop_cmd,
				(int)stop_cmd_size
			);
			stop_cmd[stop_cmd_size] = L'\0';

			self->m_apps.push_back(ApplicationInfo {
				name,
				icon_path? icon_path : L"",
				home_page_url,
				start_cmd,
				stop_cmd,
				app->is_running,
			});

			free(name);
			if (icon_path) free(icon_path);
			free(home_page_url);
			free(start_cmd);
			free(stop_cmd);
		}
	}

	self->do_popup_menu();
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
	HINSTANCE handle = ShellExecute(
			NULL,
			L"open",
			L"https://www.baidu.com",
			NULL,
			NULL,
			SW_SHOWNORMAL // 窗口显示状态
		);

	CloseHandle(handle);
}

void TrayMenu::proc_start(TrayMenu* self) {
	if (self->m_is_buckyos_running) {
		std::set<std::wstring> all_process_set;
		for (int i = 0; i < sizeof(buckyos_process) / sizeof(buckyos_process[0]); i++) {
			all_process_set.insert(buckyos_process[i]);
		}

		std::map<std::wstring, DWORD> exist_process_map;
		std::set<std::wstring> not_exist_process_set;
		if (!find_process_by_name(all_process_set, exist_process_map, not_exist_process_set)) {
			MessageBoxW(self->m_hwnd, L"BuckyOS stop failed", L"BuckyOS", MB_OK);
			return;
		}

		for (std::map<std::wstring, DWORD>::const_iterator it = exist_process_map.begin(); it != exist_process_map.end(); it++) {
			kill_process_by_id(it->second);
			MessageBoxW(self->m_hwnd, L"BuckyOS stopped", L"BuckyOS", MB_OK);
		}
	}
	else {
		MessageBoxW(self->m_hwnd, L"BuckyOS started", L"BuckyOS", MB_OK);
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
					execute_cmd_hidden(app.stop_cmd.c_str());
				}
				else {
					execute_cmd_hidden(app.start_cmd.c_str());
				}
			}
			return true;
		}
	}

	return false;
}