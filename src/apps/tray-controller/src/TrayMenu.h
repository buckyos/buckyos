#ifndef _TRAY_MENU_H_
#define _TRAY_MENU_H_

#include <windows.h>
#include <string>
#include <vector>
#include <map>

struct ApplicationInfo {
	std::wstring name;
	std::wstring icon_path;
	std::wstring home_page_url;
	std::wstring start_cmd;
	std::wstring stop_cmd;
	bool is_running;
};

class TrayMenu
{
public:
	TrayMenu(HWND hwnd, UINT_PTR menu_id_homepage, UINT_PTR menu_id_start, UINT_PTR menu_id_about, UINT_PTR menu_id_exit, UINT_PTR app_menu_id_begin);
	~TrayMenu();

	void popup(POINT &display_pos, bool is_buckyos_running);

	bool on_command(UINT_PTR menu_id);

private:

	void list_application(int seq, void (*callback)(bool is_success, std::vector<ApplicationInfo> &apps, int seq, void* user_data), void* userdata);
	static void list_application_callback(bool is_success, std::vector<ApplicationInfo>& apps, int seq, void* user_data);
	static void CALLBACK timer_proc(HWND, UINT, UINT_PTR idEvent, DWORD);
	void do_popup_menu();

	static void proc_open_homepage(TrayMenu* self);
	static void proc_start(TrayMenu* self);
	static void proc_about(TrayMenu* self);
	static void proc_exit(TrayMenu* self);

private:
	HWND m_hwnd;
	int m_seq;
	POINT m_display_pos;
	bool m_is_buckyos_running;
	std::vector<ApplicationInfo> m_apps;
	UINT_PTR m_timerId;

	int m_app_list_seq;
	std::vector<ApplicationInfo> m_menu_apps;
	bool m_is_buckyos_running_menu;
	bool m_is_popup;

	UINT_PTR m_app_menu_id_begin;
	std::map<UINT_PTR, void (*)(TrayMenu*)> m_menu_proc_map;
	UINT_PTR m_menu_id_homepage;
	UINT_PTR m_menu_id_start;
	UINT_PTR m_menu_id_stop;
	UINT_PTR m_menu_id_about;
	UINT_PTR m_menu_id_exit;
};

#endif