#include <string.h>
#include <windows.h>
#include <shellapi.h>

#include "TrayMenu.h"
#include "ffi_extern.h"

#define WM_TRAYICON (WM_USER + 1)
#define ID_TRAY_APP_ICON 1001
#define ID_TRAY_EXIT 1002
#define ID_TRAY_ABOUT 1003
#define ID_TRAY_HOMEPAGE 1004
#define ID_TRAY_START 1005
#define ID_TRAY_APP_SUBMENU_BEGIN 1006

HINSTANCE hInst;
NOTIFYICONDATA g_tray_icon_nid;

TrayMenu* g_menu;
BuckyStatusScaner g_system_state;
BuckyStatus g_bucky_status = BuckyStatus::Stopped;

void on_status_changed_callback(BuckyStatus new_status, BuckyStatus old_status, void* userdata);

LRESULT CALLBACK WindowProc(HWND hwnd, UINT uMsg, WPARAM wParam, LPARAM lParam) {
    switch (uMsg) {
    case WM_CREATE:
        g_tray_icon_nid.cbSize = sizeof(NOTIFYICONDATA);
        g_tray_icon_nid.hWnd = hwnd;
        g_tray_icon_nid.uID = ID_TRAY_APP_ICON;
        g_tray_icon_nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        g_tray_icon_nid.uCallbackMessage = WM_TRAYICON;
        g_tray_icon_nid.hIcon = LoadIcon(NULL, IDI_APPLICATION);
        wcscpy_s(g_tray_icon_nid.szTip, L"BuckyOS Controller");
        Shell_NotifyIcon(NIM_ADD, &g_tray_icon_nid);
        break;

    case WM_TRAYICON:
        if (lParam == WM_RBUTTONUP) {
            POINT pt;
            GetCursorPos(&pt);
            g_menu->popup(pt, g_bucky_status == BuckyStatus::Running);
        }
        break;

    case WM_COMMAND:
        g_menu->on_command(LOWORD(wParam));
        break;

    case WM_DESTROY:
        Shell_NotifyIcon(NIM_DELETE, &g_tray_icon_nid);
        delete g_menu;
        bucky_status_scaner_stop(g_system_state);
        break;

    default:
        return DefWindowProc(hwnd, uMsg, wParam, lParam);
    }
    return 0;
}

extern "C" void entry() {
    hInst = GetModuleHandle(NULL);
    WNDCLASS wc = {};
    wc.lpfnWndProc = WindowProc;
    wc.hInstance = hInst;
    wc.lpszClassName = L"BuckyOSController";

    RegisterClass(&wc);

    HWND hwnd = CreateWindowExW(
        0, L"BuckyOSController", L"BuckyOS Controller", 0,
        CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT,
        NULL, NULL, hInst, NULL
    );

    g_menu = new TrayMenu(hwnd, ID_TRAY_HOMEPAGE, ID_TRAY_START, ID_TRAY_ABOUT, ID_TRAY_EXIT, ID_TRAY_APP_SUBMENU_BEGIN);
    g_system_state = bucky_status_scaner_scan(on_status_changed_callback, NULL, hwnd);

    MSG msg;
    while (GetMessage(&msg, NULL, 0, 0)) {
        TranslateMessage(&msg);
        DispatchMessage(&msg);
    }
}

void on_status_changed_callback(BuckyStatus new_status, BuckyStatus old_status, void* userdata) {
    LPWSTR strIconId = IDI_APPLICATION;
    switch (new_status) {
    case BuckyStatus::Running:
        strIconId = IDI_APPLICATION;
        break;
    case BuckyStatus::Stopped:
        strIconId = IDI_HAND;
        break;
    case BuckyStatus::NotInstall:
        strIconId = IDI_QUESTION;
        break;
    case BuckyStatus::Failed:
        strIconId = IDI_ASTERISK;
        break;
    }
    g_tray_icon_nid.hIcon = LoadIcon(NULL, strIconId);
    Shell_NotifyIcon(NIM_MODIFY, &g_tray_icon_nid);
}

// extern "C" void entry() {}