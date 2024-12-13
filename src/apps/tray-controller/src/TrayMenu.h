#ifndef _TRAY_MENU_H_
#define _TRAY_MENU_H_

#include <windows.h>
#include <string>
#include <vector>
#include <map>
#include "ffi_extern.h"

class TrayMenu
{
public:
	struct ApplicationInfo {
		std::wstring name;
		std::wstring icon_path;
		std::wstring home_page_url;
		std::wstring start_cmd;
		std::wstring stop_cmd;
		bool is_running;
	};

public:
	TrayMenu(HWND hwnd, UINT_PTR menu_id_homepage, UINT_PTR menu_id_start, UINT_PTR menu_id_about, UINT_PTR menu_id_exit, UINT_PTR app_menu_id_begin);
	~TrayMenu();

	void popup(POINT &display_pos, bool is_buckyos_running);

	bool on_command(UINT_PTR menu_id);

private:

	static void list_application_callback(char is_success, ::ApplicationInfo* apps, int32_t app_count, int seq, void* user_data);
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