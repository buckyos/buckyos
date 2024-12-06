#include "TrayMenu.h"

TrayMenu::TrayMenu(HWND hwnd, UINT_PTR menu_id_homepage, UINT_PTR menu_id_start, UINT_PTR menu_id_about, UINT_PTR menu_id_exit, UINT_PTR app_menu_id_begin) {
	this->m_hwnd = hwnd;
	this->m_seq = 0;
	this->m_app_list_seq = 0;
	this->m_is_popup = false;
	this->m_timerId = 0;
	this->m_menu_id_homepage = menu_id_homepage;
	this->m_menu_id_start = menu_id_start;
	this->m_menu_id_about = menu_id_about;
	this->m_menu_id_exit = menu_id_exit;
	this->m_app_menu_id_begin = app_menu_id_begin;
	this->m_menu_proc_map[menu_id_homepage] = proc_open_homepage;
	this->m_menu_proc_map[menu_id_start] = proc_start;
	this->m_menu_proc_map[menu_id_about] = proc_about;
	this->m_menu_proc_map[menu_id_exit] = proc_exit;
}

void TrayMenu::popup(POINT& display_pos, bool is_buckyos_running) {
	this->m_seq++;
	this->m_is_popup = false;
	this->m_display_pos.x = display_pos.x;
	this->m_display_pos.y = display_pos.y;
	this->m_is_buckyos_running = is_buckyos_running;

	this->list_application(this->m_seq, list_application_callback, (void*)this);

	if (this->m_timerId != 0) {
		KillTimer(this->m_hwnd, this->m_timerId);
	}
	this->m_timerId = SetTimer(this->m_hwnd, (UINT_PTR)this, 5000, timer_proc);
}

void CALLBACK TrayMenu::timer_proc(HWND hwnd, UINT, UINT_PTR idEvent, DWORD) {
	KillTimer(hwnd, idEvent);
	TrayMenu* self = (TrayMenu*)idEvent;
	self->m_timerId = 0;
	self->do_popup_menu();
}

void TrayMenu::list_application(int seq, void (*callback)(bool is_success, std::vector<ApplicationInfo>& apps, int seq, void* user_data), void* userdata) {
	std::vector<ApplicationInfo> apps;
	{
		ApplicationInfo app;
		app.name = L"app 1";
		app.is_running = true;
		apps.push_back(app);
	}
	{
		ApplicationInfo app;
		app.name = L"app 2";
		app.is_running = false;
		apps.push_back(app);
	}
	callback(true, apps, seq, userdata);
}

void TrayMenu::list_application_callback(bool is_success, std::vector<ApplicationInfo>& apps, int seq, void* user_data) {
	TrayMenu* self = (TrayMenu*)user_data;
	if (is_success && seq > self->m_app_list_seq) {
		self->m_apps.clear();
		for (int i = 0; i < apps.size(); i++) {
			self->m_apps.push_back(apps[i]);
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
	MessageBoxW(self->m_hwnd, L"BuckyOS homepage", L"BuckyOS", MB_OK);
}

void TrayMenu::proc_start(TrayMenu* self) {
	if (self->m_is_buckyos_running) {
		MessageBoxW(self->m_hwnd, L"BuckyOS stoped", L"BuckyOS", MB_OK);
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
				MessageBoxW(this->m_hwnd, (app.name + L" homepage").c_str(), app.name.c_str(), MB_OK);
			} else if (app_cmd == 1) {
				if (app.is_running) {
					MessageBoxW(this->m_hwnd, (app.name + L" stopped").c_str(), app.name.c_str(), MB_OK);
				}
				else {
					MessageBoxW(this->m_hwnd, (app.name + L" started").c_str(), app.name.c_str(), MB_OK);
				}
			}
			return true;
		}
	}

	return false;
}