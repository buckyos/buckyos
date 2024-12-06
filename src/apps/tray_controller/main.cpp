#include <string.h>
#include <windows.h>
#include <shellapi.h>
#include "TrayMenu.h"

#define WM_TRAYICON (WM_USER + 1)
#define ID_TRAY_APP_ICON 1001
#define ID_TRAY_EXIT 1002
#define ID_TRAY_ABOUT 1003
#define ID_TRAY_HOMEPAGE 1004
#define ID_TRAY_START 1005
#define ID_TRAY_APP_SUBMENU_BEGIN 1006

HINSTANCE hInst;
NOTIFYICONDATA nid;

TrayMenu* g_menu;
bool g_is_buckyos_running = false;

LRESULT CALLBACK WindowProc(HWND hwnd, UINT uMsg, WPARAM wParam, LPARAM lParam) {
    switch (uMsg) {
    case WM_CREATE:
        // ³õÊ¼»¯ÍÐÅÌÍ¼±ê
        nid.cbSize = sizeof(NOTIFYICONDATA);
        nid.hWnd = hwnd;
        nid.uID = ID_TRAY_APP_ICON;
        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        nid.uCallbackMessage = WM_TRAYICON;
        nid.hIcon = LoadIcon(NULL, IDI_APPLICATION);
        wcscpy_s(nid.szTip, L"BuckyOS Controller");
        Shell_NotifyIcon(NIM_ADD, &nid);
        break;

    case WM_TRAYICON:
        if (lParam == WM_RBUTTONUP) {
            POINT pt;
            GetCursorPos(&pt);
            g_menu->popup(pt, g_is_buckyos_running);
            g_is_buckyos_running = !g_is_buckyos_running;
        }
        break;

    case WM_COMMAND:
        g_menu->on_command(LOWORD(wParam));
        break;

    case WM_DESTROY:
        Shell_NotifyIcon(NIM_DELETE, &nid);
        PostQuitMessage(0);
        break;

    default:
        return DefWindowProc(hwnd, uMsg, wParam, lParam);
    }
    return 0;
}

int WINAPI wWinMain(HINSTANCE hInstance, HINSTANCE, LPWSTR, int nShowCmd) {
    hInst = hInstance;
    WNDCLASS wc = {};
    wc.lpfnWndProc = WindowProc;
    wc.hInstance = hInstance;
    wc.lpszClassName = L"BuckyOSController";

    RegisterClass(&wc);

    HWND hwnd = CreateWindowExW(
        0, L"BuckyOSController", L"BuckyOS Controller", 0,
        CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT,
        NULL, NULL, hInstance, NULL
    );

    g_menu = new TrayMenu(hwnd, ID_TRAY_HOMEPAGE, ID_TRAY_START, ID_TRAY_ABOUT, ID_TRAY_EXIT, ID_TRAY_APP_SUBMENU_BEGIN);

    MSG msg;
    while (GetMessage(&msg, NULL, 0, 0)) {
        TranslateMessage(&msg);
        DispatchMessage(&msg);
    }

    return 0;
}