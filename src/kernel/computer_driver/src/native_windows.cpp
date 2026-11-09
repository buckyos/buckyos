#define NOMINMAX
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <wtsapi32.h>
#include <shellscalingapi.h>
#include <objidl.h>
#include <gdiplus.h>
#include <sddl.h>
#include <objbase.h>
#include <oleauto.h>
#include <dwmapi.h>
#include <uiautomation.h>
#include <atomic>
#include <condition_variable>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <cmath>
#include <deque>
#include <functional>
#include <map>
#include <set>
#include <memory>
#include <mutex>
#include <string>
#include <vector>

struct BuckyDesktop {
    uint32_t uid, session, display, width, height, ready, capture, input, locked;
    int32_t focus_pid;
    uint32_t focus_on_display, physical_input_down;
    uint64_t focus, events, hardware_events, layout;
    double x, y, logical_width, logical_height;
};

static_assert(sizeof(BuckyDesktop) == 112, "desktop ABI mismatch");

struct DpiScope {
    DPI_AWARENESS_CONTEXT previous = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    ~DpiScope() { if (previous) SetThreadDpiAwarenessContext(previous); }
};

struct Runtime {
    std::atomic<uint64_t> events{0}, hardware{0}, desktop_epoch{1}, focus_epoch{1}, heartbeat{0};
    std::atomic<bool> healthy{false}, capturing{false};
    std::atomic<bool> physical[256]{};
    std::once_flag start;
    HANDLE initialized = CreateEventW(nullptr, TRUE, FALSE, nullptr);
    HWND window = nullptr;
    ULONG_PTR marker = reinterpret_cast<ULONG_PTR>(this) ^ GetCurrentProcessId() ^ static_cast<ULONG_PTR>(GetTickCount64());
    std::mutex pending_mutex, state_mutex;
    std::vector<INPUT> pending;
    uint64_t last_state = UINT64_MAX;
    uint64_t heartbeat_limit = 250;
    ULONG_PTR gdiplus = 0;
    std::vector<unsigned char> user;
    DWORD integrity = 0;
};

static Runtime &runtime() { static Runtime *value = new Runtime; return *value; }
static uint64_t hash(uint64_t value, uint64_t field) { return (value ^ field) * 1099511628211ULL; }
static thread_local int (*dispatch_guard)(const void *, const BuckyDesktop *) = nullptr;
static thread_local const void *dispatch_guard_context = nullptr;

extern "C" void bucky_desktop_set_guard(int (*guard)(const void *, const BuckyDesktop *), const void *context) {
    dispatch_guard = guard;
    dispatch_guard_context = context;
}

static uint64_t configured_heartbeat_limit() {
    wchar_t value[32]{};
    DWORD type = 0, length = sizeof(value);
    LSTATUS status = RegGetValueW(HKEY_CURRENT_USER, L"Control Panel\\Desktop", L"LowLevelHooksTimeout",
        RRF_RT_REG_SZ | RRF_RT_REG_DWORD, &type, value, &length);
    if (status == ERROR_FILE_NOT_FOUND) return 250;
    if (status != ERROR_SUCCESS) return 0;
    uint64_t timeout = 0;
    if (type == REG_DWORD && length == sizeof(DWORD)) {
        DWORD number = 0;
        std::memcpy(&number, value, sizeof(number));
        timeout = number;
    } else if (type == REG_SZ && value[0] >= L'0' && value[0] <= L'9') {
        wchar_t *end = nullptr;
        timeout = std::wcstoull(value, &end, 10);
        if (!end || *end) return 0;
    } else return 0;
    return timeout / 2 < 250 ? timeout / 2 : 250;
}

static bool advance_monitor_heartbeat(std::atomic<uint64_t> &heartbeat, std::atomic<bool> &healthy,
                                      uint64_t now, uint64_t limit) {
    uint64_t previous = heartbeat.exchange(now);
    if (previous && (!limit || now < previous || now - previous >= limit)) return healthy.exchange(false);
    return false;
}

static bool token_identity(HANDLE process, std::vector<unsigned char> &sid, DWORD &integrity) {
    HANDLE token = nullptr;
    if (!OpenProcessToken(process, TOKEN_QUERY, &token)) return false;
    DWORD length = 0;
    GetTokenInformation(token, TokenUser, nullptr, 0, &length);
    std::vector<unsigned char> user(length);
    bool valid = length && GetTokenInformation(token, TokenUser, user.data(), length, &length);
    if (valid) {
        PSID source = reinterpret_cast<TOKEN_USER *>(user.data())->User.Sid;
        sid.resize(GetLengthSid(source));
        valid = CopySid(static_cast<DWORD>(sid.size()), sid.data(), source) != FALSE;
    }
    length = 0;
    GetTokenInformation(token, TokenIntegrityLevel, nullptr, 0, &length);
    std::vector<unsigned char> level(length);
    valid = valid && length && GetTokenInformation(token, TokenIntegrityLevel, level.data(), length, &length);
    if (valid) {
        PSID source = reinterpret_cast<TOKEN_MANDATORY_LABEL *>(level.data())->Label.Sid;
        UCHAR count = *GetSidSubAuthorityCount(source);
        valid = count != 0;
        if (valid) integrity = *GetSidSubAuthority(source, count - 1);
    }
    CloseHandle(token);
    return valid;
}

static bool process_allowed(DWORD pid) {
    if (!pid) return false;
    DWORD session = 0, ours = 0;
    if (!ProcessIdToSessionId(pid, &session) || !ProcessIdToSessionId(GetCurrentProcessId(), &ours) || session != ours) return false;
    HANDLE process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid);
    if (!process) return false;
    std::vector<unsigned char> sid;
    DWORD integrity = 0;
    bool valid = token_identity(process, sid, integrity);
    CloseHandle(process);
    auto &r = runtime();
    return valid && !r.user.empty() && EqualSid(sid.data(), r.user.data()) && integrity <= r.integrity;
}

static bool object_named(HANDLE object, const wchar_t *expected) {
    wchar_t name[256]{};
    DWORD length = 0;
    return object && GetUserObjectInformationW(object, UOI_NAME, name, sizeof(name), &length) && _wcsicmp(name, expected) == 0;
}

static std::wstring session_text(DWORD session, WTS_INFO_CLASS field) {
    LPWSTR text = nullptr;
    DWORD length = 0;
    if (!WTSQuerySessionInformationW(WTS_CURRENT_SERVER_HANDLE, session, field, &text, &length)) return {};
    std::wstring result = text && length >= sizeof(wchar_t) ? text : L"";
    WTSFreeMemory(text);
    return result;
}

static bool session_owned(DWORD session) {
    auto user = session_text(session, WTSUserName);
    auto domain = session_text(session, WTSDomainName);
    if (user.empty() || runtime().user.empty()) return false;
    auto account = domain.empty() ? user : domain + L"\\" + user;
    DWORD sid_length = 0, domain_length = 0;
    SID_NAME_USE use;
    LookupAccountNameW(nullptr, account.c_str(), nullptr, &sid_length, nullptr, &domain_length, &use);
    if (!sid_length) return false;
    std::vector<unsigned char> sid(sid_length);
    std::vector<wchar_t> resolved_domain(domain_length);
    return LookupAccountNameW(nullptr, account.c_str(), sid.data(), &sid_length, resolved_domain.data(), &domain_length, &use)
        && EqualSid(sid.data(), runtime().user.data());
}

static LRESULT CALLBACK keyboard_hook(int code, WPARAM message, LPARAM data) {
    if (code == HC_ACTION) {
        auto &r = runtime();
        const auto *key = reinterpret_cast<KBDLLHOOKSTRUCT *>(data);
        r.events.fetch_add(1);
        if (!(key->flags & LLKHF_INJECTED) || key->dwExtraInfo != r.marker) {
            r.hardware.fetch_add(1);
            if (key->vkCode < 256) r.physical[key->vkCode].store(message == WM_KEYDOWN || message == WM_SYSKEYDOWN);
        }
    }
    return CallNextHookEx(nullptr, code, message, data);
}

static LRESULT CALLBACK mouse_hook(int code, WPARAM message, LPARAM data) {
    if (code == HC_ACTION) {
        auto &r = runtime();
        const auto *mouse = reinterpret_cast<MSLLHOOKSTRUCT *>(data);
        r.events.fetch_add(1);
        if (!(mouse->flags & LLMHF_INJECTED) || mouse->dwExtraInfo != r.marker) {
            r.hardware.fetch_add(1);
            switch (message) {
                case WM_LBUTTONDOWN: r.physical[VK_LBUTTON].store(true); break;
                case WM_LBUTTONUP: r.physical[VK_LBUTTON].store(false); break;
                case WM_RBUTTONDOWN: r.physical[VK_RBUTTON].store(true); break;
                case WM_RBUTTONUP: r.physical[VK_RBUTTON].store(false); break;
                case WM_MBUTTONDOWN: r.physical[VK_MBUTTON].store(true); break;
                case WM_MBUTTONUP: r.physical[VK_MBUTTON].store(false); break;
                case WM_XBUTTONDOWN: r.physical[HIWORD(mouse->mouseData) == XBUTTON1 ? VK_XBUTTON1 : VK_XBUTTON2].store(true); break;
                case WM_XBUTTONUP: r.physical[HIWORD(mouse->mouseData) == XBUTTON1 ? VK_XBUTTON1 : VK_XBUTTON2].store(false); break;
            }
        }
    }
    return CallNextHookEx(nullptr, code, message, data);
}

static void CALLBACK window_event(HWINEVENTHOOK, DWORD event, HWND, LONG, LONG, DWORD, DWORD) {
    if (event == EVENT_SYSTEM_DESKTOPSWITCH) runtime().desktop_epoch.fetch_add(1);
    else runtime().focus_epoch.fetch_add(1);
}

static LRESULT CALLBACK monitor_window(HWND window, UINT message, WPARAM wparam, LPARAM lparam) {
    auto &r = runtime();
    if ((message == WM_TIMER || message == WM_APP)
        && advance_monitor_heartbeat(r.heartbeat, r.healthy, GetTickCount64(), r.heartbeat_limit)) r.desktop_epoch.fetch_add(1);
    if (message == WM_WTSSESSION_CHANGE || message == WM_DISPLAYCHANGE || message == WM_SETTINGCHANGE || message == WM_DPICHANGED)
        r.desktop_epoch.fetch_add(1);
    return DefWindowProcW(window, message, wparam, lparam);
}

static DWORD WINAPI monitor_thread(void *) {
    auto &r = runtime();
    DpiScope dpi;
    auto instance = GetModuleHandleW(nullptr);
    WNDCLASSW cls{};
    cls.lpfnWndProc = monitor_window;
    cls.hInstance = instance;
    cls.lpszClassName = L"BuckyDesktopInputMonitor";
    RegisterClassW(&cls);
    r.window = CreateWindowExW(0, cls.lpszClassName, L"", WS_POPUP, 0, 0, 0, 0, nullptr, nullptr, instance, nullptr);
    r.heartbeat.store(GetTickCount64());
    HHOOK keyboard = SetWindowsHookExW(WH_KEYBOARD_LL, keyboard_hook, instance, 0);
    HHOOK mouse = SetWindowsHookExW(WH_MOUSE_LL, mouse_hook, instance, 0);
    HWINEVENTHOOK desktop = SetWinEventHook(EVENT_SYSTEM_DESKTOPSWITCH, EVENT_SYSTEM_DESKTOPSWITCH, nullptr, window_event, 0, 0, WINEVENT_OUTOFCONTEXT);
    HWINEVENTHOOK foreground = SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, nullptr, window_event, 0, 0, WINEVENT_OUTOFCONTEXT);
    HWINEVENTHOOK focus = SetWinEventHook(EVENT_OBJECT_FOCUS, EVENT_OBJECT_FOCUS, nullptr, window_event, 0, 0, WINEVENT_OUTOFCONTEXT);
    bool notifications = r.window && WTSRegisterSessionNotification(r.window, NOTIFY_FOR_THIS_SESSION);
    for (int key = 1; key < 256; ++key) {
        if (key != VK_SHIFT && key != VK_CONTROL && key != VK_MENU) r.physical[key].store((GetAsyncKeyState(key) & 0x8000) != 0);
    }
    UINT interval = static_cast<UINT>(r.heartbeat_limit / 3 < 50 ? r.heartbeat_limit / 3 : 50);
    r.healthy.store(dpi.previous && keyboard && mouse && desktop && foreground && focus && notifications
        && r.heartbeat_limit && GetTickCount64() - r.heartbeat.load() < r.heartbeat_limit && SetTimer(r.window, 1, interval, nullptr));
    SetEvent(r.initialized);
    MSG message;
    while (GetMessageW(&message, nullptr, 0, 0) > 0) {
        TranslateMessage(&message);
        DispatchMessageW(&message);
    }
    r.healthy.store(false);
    if (keyboard) UnhookWindowsHookEx(keyboard);
    if (mouse) UnhookWindowsHookEx(mouse);
    if (desktop) UnhookWinEvent(desktop);
    if (foreground) UnhookWinEvent(foreground);
    if (focus) UnhookWinEvent(focus);
    if (notifications) WTSUnRegisterSessionNotification(r.window);
    if (r.window) DestroyWindow(r.window);
    return 0;
}

static bool initialize() {
    auto &r = runtime();
    std::call_once(r.start, [&r] {
        if (!r.initialized) return;
        if (!token_identity(GetCurrentProcess(), r.user, r.integrity)) { SetEvent(r.initialized); return; }
        Gdiplus::GdiplusStartupInput options;
        if (Gdiplus::GdiplusStartup(&r.gdiplus, &options, nullptr) != Gdiplus::Ok) r.gdiplus = 0;
        r.heartbeat_limit = configured_heartbeat_limit();
        HANDLE thread = CreateThread(nullptr, 0, monitor_thread, nullptr, 0, nullptr);
        if (thread) CloseHandle(thread);
        else SetEvent(r.initialized);
    });
    return r.initialized && WaitForSingleObject(r.initialized, 1500) == WAIT_OBJECT_0;
}

struct Layout { uint64_t value = 1469598103934665603ULL; RECT primary{}; bool valid = true; };

static BOOL CALLBACK monitor_layout(HMONITOR monitor, HDC, LPRECT, LPARAM context) {
    auto &layout = *reinterpret_cast<Layout *>(context);
    MONITORINFOEXW info{};
    info.cbSize = sizeof(info);
    DEVICE_SCALE_FACTOR scale;
    DEVMODEW mode{};
    mode.dmSize = sizeof(mode);
    if (!GetMonitorInfoW(monitor, &info) || FAILED(GetScaleFactorForMonitor(monitor, &scale)) || !EnumDisplaySettingsW(info.szDevice, ENUM_CURRENT_SETTINGS, &mode)) {
        layout.valid = false;
        return FALSE;
    }
    if (info.dwFlags & MONITORINFOF_PRIMARY) layout.primary = info.rcMonitor;
    uint64_t fields[] = {static_cast<uint64_t>(info.rcMonitor.left), static_cast<uint64_t>(info.rcMonitor.top),
        static_cast<uint64_t>(info.rcMonitor.right), static_cast<uint64_t>(info.rcMonitor.bottom), info.dwFlags,
        mode.dmPelsWidth, mode.dmPelsHeight, mode.dmDisplayOrientation, static_cast<uint64_t>(scale)};
    for (auto value : fields) layout.value = hash(layout.value, value);
    for (auto character : info.szDevice) layout.value = hash(layout.value, character);
    return TRUE;
}

static bool input_desktop() {
    if (!object_named(GetProcessWindowStation(), L"WinSta0") || !object_named(GetThreadDesktop(GetCurrentThreadId()), L"Default")) return false;
    HDESK input = OpenInputDesktop(0, FALSE, DESKTOP_READOBJECTS);
    bool valid = object_named(input, L"Default");
    if (input) CloseDesktop(input);
    BOOL receives_input = FALSE;
    DWORD length = 0;
    return valid && GetUserObjectInformationW(GetThreadDesktop(GetCurrentThreadId()), UOI_IO, &receives_input, sizeof(receives_input), &length) && receives_input;
}

static void probe(BuckyDesktop &out) {
    out = {};
    DpiScope dpi;
    if (!initialize()) return;
    auto &r = runtime();
    DWORD_PTR ignored = 0;
    bool monitor_live = r.healthy.load() && r.window
        && SendMessageTimeoutW(r.window, WM_APP, 0, 0, SMTO_ABORTIFHUNG, 200, &ignored) && r.healthy.load();
    DWORD session = 0;
    if (!ProcessIdToSessionId(GetCurrentProcessId(), &session)) return;
    out.session = session;
    out.display = 1;
    LPWSTR information = nullptr;
    DWORD length = 0;
    bool active = false, unlocked = false;
    if (WTSQuerySessionInformationW(WTS_CURRENT_SERVER_HANDLE, session, WTSSessionInfoEx, &information, &length)) {
        auto *info = reinterpret_cast<WTSINFOEXW *>(information);
        if (length >= sizeof(WTSINFOEXW) && info->Level == 1) {
            active = info->Data.WTSInfoExLevel1.SessionState == WTSActive;
            unlocked = info->Data.WTSInfoExLevel1.SessionFlags == WTS_SESSIONSTATE_UNLOCK;
        }
        WTSFreeMemory(information);
    }
    bool desktop = input_desktop();
    out.locked = !unlocked || !desktop;
    out.ready = dpi.previous && monitor_live && session != 0 && active && unlocked && desktop && session_owned(session);
    uint64_t state = hash(hash(hash(session, active), unlocked), desktop);
    {
        std::lock_guard<std::mutex> lock(r.state_mutex);
        if (r.last_state != state) { r.last_state = state; r.desktop_epoch.fetch_add(1); }
    }
    Layout layout;
    if (!EnumDisplayMonitors(nullptr, nullptr, monitor_layout, reinterpret_cast<LPARAM>(&layout)) || !layout.valid) out.ready = 0;
    out.x = layout.primary.left;
    out.y = layout.primary.top;
    out.width = static_cast<uint32_t>(layout.primary.right - layout.primary.left);
    out.height = static_cast<uint32_t>(layout.primary.bottom - layout.primary.top);
    out.logical_width = out.width;
    out.logical_height = out.height;
    out.layout = hash(layout.value, r.desktop_epoch.load());
    if (!out.width || !out.height) out.ready = 0;
    out.capture = out.ready && r.gdiplus != 0;
    HWND foreground = GetForegroundWindow();
    DWORD pid = 0;
    DWORD thread = GetWindowThreadProcessId(foreground, &pid);
    out.focus_pid = static_cast<int32_t>(pid);
    GUITHREADINFO gui{};
    gui.cbSize = sizeof(gui);
    if (thread && GetGUIThreadInfo(thread, &gui) && gui.hwndFocus) {
        out.focus = hash(hash(hash(1469598103934665603ULL, reinterpret_cast<uintptr_t>(foreground)), reinterpret_cast<uintptr_t>(gui.hwndFocus)), r.focus_epoch.load());
        RECT rect{};
        if (GetWindowRect(gui.hwndFocus, &rect)) out.focus_on_display = rect.left >= layout.primary.left && rect.top >= layout.primary.top
            && rect.right <= layout.primary.right && rect.bottom <= layout.primary.bottom && rect.right > rect.left && rect.bottom > rect.top;
    }
    out.input = out.ready && foreground && process_allowed(pid);
    out.events = r.events.load();
    out.hardware_events = r.hardware.load();
    for (int key = 1; key < 256; ++key) if (r.physical[key].load()) out.physical_input_down = 1;
}

extern "C" void bucky_desktop_probe(BuckyDesktop *out) {
    if (!out) return;
    try { probe(*out); } catch (...) { *out = {}; }
}

extern "C" void bucky_desktop_free(void *bytes) { std::free(bytes); }

struct Capture {
    HANDLE done = CreateEventW(nullptr, TRUE, FALSE, nullptr);
    BuckyDesktop desktop{};
    std::vector<unsigned char> bytes;
    int result = 3;
    ~Capture() { if (done) CloseHandle(done); }
};

static bool encode_capture(Capture &capture) {
    auto &desktop = capture.desktop;
    HDC screen = GetDC(nullptr);
    if (!screen) return false;
    HDC memory = CreateCompatibleDC(screen);
    HBITMAP bitmap = CreateCompatibleBitmap(screen, static_cast<int>(desktop.width), static_cast<int>(desktop.height));
    HGDIOBJ previous = memory && bitmap ? SelectObject(memory, bitmap) : nullptr;
    bool valid = previous && BitBlt(memory, 0, 0, static_cast<int>(desktop.width), static_cast<int>(desktop.height), screen,
        static_cast<int>(desktop.x), static_cast<int>(desktop.y), SRCCOPY | CAPTUREBLT);
    if (previous) SelectObject(memory, previous);
    if (memory) DeleteDC(memory);
    ReleaseDC(nullptr, screen);
    if (valid) {
        Gdiplus::Bitmap image(bitmap, nullptr);
        IStream *stream = nullptr;
        valid = image.GetLastStatus() == Gdiplus::Ok && SUCCEEDED(CreateStreamOnHGlobal(nullptr, TRUE, &stream));
        if (valid) {
            CLSID png = {0x557cf406, 0x1a04, 0x11d3, {0x9a, 0x73, 0x00, 0x00, 0xf8, 0x1e, 0xf3, 0x2e}};
            STATSTG stats{};
            valid = image.Save(stream, &png) == Gdiplus::Ok && SUCCEEDED(stream->Stat(&stats, STATFLAG_NONAME))
                && stats.cbSize.QuadPart > 0 && stats.cbSize.QuadPart <= 16 * 1024 * 1024;
            if (valid) {
                LARGE_INTEGER start{};
                ULONG read = 0;
                capture.bytes.resize(static_cast<size_t>(stats.cbSize.QuadPart));
                valid = SUCCEEDED(stream->Seek(start, STREAM_SEEK_SET, nullptr))
                    && SUCCEEDED(stream->Read(capture.bytes.data(), static_cast<ULONG>(capture.bytes.size()), &read)) && read == capture.bytes.size();
            }
            stream->Release();
        }
    }
    if (bitmap) DeleteObject(bitmap);
    return valid;
}

static DWORD WINAPI capture_thread(void *parameter) {
    std::unique_ptr<std::shared_ptr<Capture>> owner(static_cast<std::shared_ptr<Capture> *>(parameter));
    auto capture = *owner;
    try {
        DpiScope dpi;
        BuckyDesktop before{}, after{};
        probe(before);
        if (before.capture && before.session == capture->desktop.session && before.layout == capture->desktop.layout && encode_capture(*capture)) {
            probe(after);
            if (after.capture && after.session == before.session && after.layout == before.layout) capture->result = 0;
        }
    } catch (...) { capture->result = 3; }
    runtime().capturing.store(false);
    SetEvent(capture->done);
    return 0;
}

extern "C" int bucky_desktop_capture(uint32_t display, uint32_t width, uint32_t height, uint64_t timeout,
                                     unsigned char **bytes, size_t *length) {
    if (!bytes || !length) return 3;
    *bytes = nullptr;
    *length = 0;
    try {
        auto capture = std::make_shared<Capture>();
        probe(capture->desktop);
        if (!capture->desktop.capture) return 1;
        if (!capture->done || display != 1 || width != capture->desktop.width || height != capture->desktop.height) return 3;
        if (!timeout) return 2;
        auto parameter = std::make_unique<std::shared_ptr<Capture>>(capture);
        if (runtime().capturing.exchange(true)) return 4;
        HANDLE thread = CreateThread(nullptr, 0, capture_thread, parameter.get(), 0, nullptr);
        if (!thread) { runtime().capturing.store(false); return 3; }
        parameter.release();
        CloseHandle(thread);
        DWORD wait = timeout >= INFINITE ? INFINITE - 1 : static_cast<DWORD>(timeout);
        if (WaitForSingleObject(capture->done, wait) != WAIT_OBJECT_0) return 2;
        if (capture->result) return capture->result;
        *bytes = static_cast<unsigned char *>(std::malloc(capture->bytes.size()));
        if (!*bytes) return 3;
        *length = capture->bytes.size();
        std::memcpy(*bytes, capture->bytes.data(), *length);
        return 0;
    } catch (...) { return 3; }
}

static INPUT key_input(WORD key, bool up, bool unicode = false) {
    INPUT input{};
    input.type = INPUT_KEYBOARD;
    input.ki.wVk = unicode ? 0 : key;
    input.ki.wScan = unicode ? key : static_cast<WORD>(MapVirtualKeyW(key, MAPVK_VK_TO_VSC));
    input.ki.dwFlags = (up ? KEYEVENTF_KEYUP : 0) | (unicode ? KEYEVENTF_UNICODE : 0);
    if (!unicode && (key == VK_RMENU || key == VK_RCONTROL || key == VK_LWIN || key == VK_RWIN || key == VK_INSERT || key == VK_DELETE
        || key == VK_HOME || key == VK_END || key == VK_PRIOR || key == VK_NEXT || key == VK_LEFT || key == VK_RIGHT || key == VK_UP || key == VK_DOWN))
        input.ki.dwFlags |= KEYEVENTF_EXTENDEDKEY;
    input.ki.dwExtraInfo = runtime().marker;
    return input;
}

static INPUT mouse_input(DWORD flags, DWORD data = 0) {
    INPUT input{};
    input.type = INPUT_MOUSE;
    input.mi.dwFlags = flags;
    input.mi.mouseData = data;
    input.mi.dwExtraInfo = runtime().marker;
    return input;
}

static void modifiers(std::vector<INPUT> &events, uint64_t flags, bool up) {
    WORD keys[] = {VK_LSHIFT, VK_LCONTROL, VK_LMENU, VK_LWIN};
    for (int step = 0; step < 4; ++step) {
        int i = up ? 3 - step : step;
        if (flags & (1ULL << (17 + i))) events.push_back(key_input(keys[i], up));
    }
}

static bool pointer_input(double x, double y, INPUT &input) {
    BuckyDesktop desktop{};
    probe(desktop);
    if (!desktop.input || !std::isfinite(x) || !std::isfinite(y) || x < desktop.x || y < desktop.y
        || x >= desktop.x + desktop.width || y >= desktop.y + desktop.height) return false;
    POINT point{static_cast<LONG>(std::floor(x)), static_cast<LONG>(std::floor(y))};
    HWND target = WindowFromPoint(point);
    DWORD pid = 0;
    GetWindowThreadProcessId(target, &pid);
    if (!target || !process_allowed(pid)) return false;
    LONG left = GetSystemMetrics(SM_XVIRTUALSCREEN), top = GetSystemMetrics(SM_YVIRTUALSCREEN);
    LONG width = GetSystemMetrics(SM_CXVIRTUALSCREEN), height = GetSystemMetrics(SM_CYVIRTUALSCREEN);
    if (width <= 0 || height <= 0) return false;
    input = mouse_input(MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK);
    input.mi.dx = static_cast<LONG>((static_cast<int64_t>(point.x - left) * 65536 + 32768) / width);
    input.mi.dy = static_cast<LONG>((static_cast<int64_t>(point.y - top) * 65536 + 32768) / height);
    return true;
}

static void remember(const INPUT &input, std::vector<INPUT> &pending) {
    if (input.type == INPUT_KEYBOARD) {
        for (auto it = pending.begin(); it != pending.end(); ++it) {
            if (it->type == INPUT_KEYBOARD && it->ki.wVk == input.ki.wVk && it->ki.wScan == input.ki.wScan) {
                if (input.ki.dwFlags & KEYEVENTF_KEYUP) pending.erase(it);
                return;
            }
        }
        if (!(input.ki.dwFlags & KEYEVENTF_KEYUP)) { INPUT up = input; up.ki.dwFlags |= KEYEVENTF_KEYUP; pending.push_back(up); }
    } else if (input.type == INPUT_MOUSE) {
        DWORD downs[] = {MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_MIDDLEDOWN};
        DWORD ups[] = {MOUSEEVENTF_LEFTUP, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_MIDDLEUP};
        for (int i = 0; i < 3; ++i) {
            if (input.mi.dwFlags & downs[i]) pending.push_back(mouse_input(ups[i]));
            if (input.mi.dwFlags & ups[i]) {
                for (auto it = pending.begin(); it != pending.end(); ++it) if (it->type == INPUT_MOUSE && it->mi.dwFlags == ups[i]) { pending.erase(it); break; }
            }
        }
    }
}

static int dispatch(std::vector<INPUT> &events) {
    auto &r = runtime();
    std::lock_guard<std::mutex> lock(r.pending_mutex);
    if (!r.pending.empty()) return 1;
    BuckyDesktop desktop{};
    probe(desktop);
    if (!desktop.input) return 1;
    r.pending.reserve(events.size());
    if (dispatch_guard && !dispatch_guard(dispatch_guard_context, &desktop)) return 1;
    UINT sent = SendInput(static_cast<UINT>(events.size()), events.data(), sizeof(INPUT));
    for (UINT i = 0; i < sent; ++i) remember(events[i], r.pending);
    return sent == events.size() ? 0 : sent == 0 ? 1 : 2;
}

extern "C" int bucky_desktop_click(double x, double y, uint32_t button, uint32_t count, uint64_t flags) {
    try {
        DpiScope dpi;
        if (!dpi.previous || button > 2 || count == 0 || count > 3 || flags & ~((1ULL << 21) - (1ULL << 17))) return 1;
        INPUT move{};
        if (!pointer_input(x, y, move)) return 1;
        std::vector<INPUT> events{move};
        modifiers(events, flags, false);
        DWORD down[] = {MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_MIDDLEDOWN};
        DWORD up[] = {MOUSEEVENTF_LEFTUP, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_MIDDLEUP};
        events.push_back(mouse_input(down[button]));
        events.push_back(mouse_input(up[button]));
        modifiers(events, flags, true);
        return dispatch(events);
    } catch (...) { return 2; }
}

extern "C" int bucky_desktop_key(uint16_t key, uint64_t flags, const uint16_t *text, size_t length) {
    try {
        if (key >= 256 || length > 2 || (length && !text) || flags & ~((1ULL << 21) - (1ULL << 17))) return 1;
        std::vector<INPUT> events;
        modifiers(events, flags, false);
        if (length) {
            for (size_t i = 0; i < length; ++i) { events.push_back(key_input(text[i], false, true)); events.push_back(key_input(text[i], true, true)); }
        } else {
            if (!key) return 1;
            events.push_back(key_input(key, false));
            events.push_back(key_input(key, true));
        }
        modifiers(events, flags, true);
        return dispatch(events);
    } catch (...) { return 2; }
}

extern "C" int bucky_desktop_keycode(uint16_t character) {
    DWORD thread = GetWindowThreadProcessId(GetForegroundWindow(), nullptr);
    SHORT key = VkKeyScanExW(character, GetKeyboardLayout(thread));
    return key != -1 && HIBYTE(key) == 0 ? LOBYTE(key) : -1;
}

extern "C" int bucky_desktop_scroll(double x, double y, int32_t dx, int32_t dy, uint32_t pixel) {
    try {
        DpiScope dpi;
        if (!dpi.previous || pixel) return 1;
        UINT lines = 0, characters = 0;
        if (dy && (!SystemParametersInfoW(SPI_GETWHEELSCROLLLINES, 0, &lines, 0) || !lines || lines == WHEEL_PAGESCROLL)) return 1;
        if (dx && (!SystemParametersInfoW(SPI_GETWHEELSCROLLCHARS, 0, &characters, 0) || !characters || characters == WHEEL_PAGESCROLL)) return 1;
        int64_t horizontal = dx ? std::llround(static_cast<double>(dx) * WHEEL_DELTA / characters) : 0;
        int64_t vertical = dy ? std::llround(-static_cast<double>(dy) * WHEEL_DELTA / lines) : 0;
        if ((dx && !horizontal) || (dy && !vertical) || horizontal > INT32_MAX || horizontal < INT32_MIN || vertical > INT32_MAX || vertical < INT32_MIN) return 1;
        INPUT move{};
        if (!pointer_input(x, y, move)) return 1;
        std::vector<INPUT> events{move};
        if (dx) events.push_back(mouse_input(MOUSEEVENTF_HWHEEL, static_cast<DWORD>(horizontal)));
        if (dy) events.push_back(mouse_input(MOUSEEVENTF_WHEEL, static_cast<DWORD>(vertical)));
        return dispatch(events);
    } catch (...) { return 2; }
}

extern "C" int bucky_desktop_finish() {
    try {
        auto &r = runtime();
        std::lock_guard<std::mutex> lock(r.pending_mutex);
        if (r.pending.empty()) return 0;
        BuckyDesktop desktop{};
        probe(desktop);
        if (!desktop.input) return 1;
        auto releases = r.pending;
        UINT sent = SendInput(static_cast<UINT>(releases.size()), releases.data(), sizeof(INPUT));
        for (UINT i = 0; i < sent; ++i) remember(releases[i], r.pending);
        return sent == releases.size() ? 0 : 1;
    } catch (...) { return 1; }
}

extern "C" int bucky_desktop_release(uint16_t cleanup) {
    if (cleanup == UINT16_MAX) return 0;
    if (cleanup >= 260) return 1;
    try {
        auto &r = runtime();
        std::lock_guard<std::mutex> lock(r.pending_mutex);
        BuckyDesktop desktop{};
        probe(desktop);
        if (!desktop.input || desktop.physical_input_down) return 1;
        auto releases = r.pending;
        if (releases.empty()) {
            if (cleanup == 259) return 1;
            if (cleanup < 256) releases.push_back(key_input(cleanup, true));
            else if (cleanup < 259) {
                DWORD ups[] = {MOUSEEVENTF_LEFTUP, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_MIDDLEUP};
                releases.push_back(mouse_input(ups[cleanup - 256]));
            }
            WORD modifiers_to_release[] = {VK_LSHIFT, VK_RSHIFT, VK_LCONTROL, VK_RCONTROL, VK_LMENU, VK_RMENU, VK_LWIN, VK_RWIN};
            for (auto key : modifiers_to_release) if (GetAsyncKeyState(key) & 0x8000) releases.push_back(key_input(key, true));
        }
        if (releases.empty()) return 0;
        UINT sent = SendInput(static_cast<UINT>(releases.size()), releases.data(), sizeof(INPUT));
        for (UINT i = 0; i < sent; ++i) remember(releases[i], r.pending);
        return sent == releases.size() ? 0 : 1;
    } catch (...) { return 1; }
}

// UI Automation: every IUIAutomation call runs on one MTA worker thread.
// Callers queue a job and wait with a deadline; a job that has not started
// when its caller gives up is skipped, so a late action never runs.

enum : uint32_t {
    PAT_INVOKE = 1u << 0, PAT_VALUE = 1u << 1, PAT_TOGGLE = 1u << 2, PAT_SELECTION_ITEM = 1u << 3,
    PAT_EXPAND_COLLAPSE = 1u << 4, PAT_SCROLL_ITEM = 1u << 5, PAT_FOCUSABLE = 1u << 6,
};
enum : uint32_t {
    ST_DISABLED = 1u << 0, ST_FOCUSED = 1u << 1, ST_READONLY = 1u << 2, ST_SELECTED = 1u << 3,
    ST_CHECKED = 1u << 4, ST_MIXED = 1u << 5, ST_EXPANDED = 1u << 6, ST_COLLAPSED = 1u << 7,
    ST_PARTIAL = 1u << 8, ST_DIALOG = 1u << 9,
};
enum : int { UR_OK = 0, UR_STALE = 1, UR_TIMEOUT = 2, UR_FAILED = 3, UR_UNAVAILABLE = 4, UR_BUSY = 5, UR_DISABLED = 6, UR_DENIED = 7, UR_NOT_STARTED = 8 };
enum : int { JOB_QUEUED = 0, JOB_RUNNING = 1, JOB_ABANDONED = 2 };

static const uint32_t UIA_MAX_VISITED = 3000;
static const uint32_t UIA_MAX_POPUPS = 4;
static const size_t UIA_MAX_NAME = 512;
static const size_t UIA_MAX_VALUE = 20000;
static const size_t UIA_MAX_SNAPSHOTS = 96;

struct UiaJob {
    std::function<int()> run;
    std::atomic<int> state{JOB_QUEUED};
    int result = UR_FAILED;
    std::string json;
    HANDLE done = CreateEventW(nullptr, TRUE, FALSE, nullptr);
    ~UiaJob() { if (done) CloseHandle(done); }
};

struct UiaNode {
    IUIAutomationElement *element = nullptr;
    HWND root = nullptr;
    DWORD root_pid = 0, pid = 0;
    std::vector<int> id;
};

struct UiaSnapshot { std::vector<UiaNode> nodes; };

struct Uia {
    std::once_flag start;
    std::mutex mutex;
    std::condition_variable wake;
    std::deque<std::shared_ptr<UiaJob>> jobs;
    bool stop = false;
    HRESULT status = E_PENDING;
    HANDLE ready = CreateEventW(nullptr, TRUE, FALSE, nullptr);
    HANDLE thread = nullptr;
    IUIAutomation2 *automation = nullptr;
    IUIAutomationCondition *view = nullptr;
    IUIAutomationCacheRequest *element_cache = nullptr;
    IUIAutomationCacheRequest *children_cache = nullptr;
    std::map<uint64_t, UiaSnapshot> snapshots;
    uint64_t next_snapshot = 1;
};

static Uia &uia() { static Uia *value = new Uia; return *value; }

static void release_snapshot(UiaSnapshot &snapshot) {
    for (auto &node : snapshot.nodes) if (node.element) { node.element->Release(); node.element = nullptr; }
}

static HRESULT make_cache(Uia &u, TreeScope scope, IUIAutomationCacheRequest **out) {
    IUIAutomationCacheRequest *request = nullptr;
    HRESULT hr = u.automation->CreateCacheRequest(&request);
    if (FAILED(hr)) return hr;
    const PROPERTYID required[] = {
        UIA_RuntimeIdPropertyId, UIA_ControlTypePropertyId, UIA_NamePropertyId, UIA_AutomationIdPropertyId,
        UIA_ClassNamePropertyId, UIA_BoundingRectanglePropertyId, UIA_IsOffscreenPropertyId, UIA_IsEnabledPropertyId,
        UIA_HasKeyboardFocusPropertyId, UIA_IsKeyboardFocusablePropertyId, UIA_ProcessIdPropertyId,
        UIA_IsInvokePatternAvailablePropertyId, UIA_IsValuePatternAvailablePropertyId, UIA_IsTogglePatternAvailablePropertyId,
        UIA_IsSelectionItemPatternAvailablePropertyId, UIA_IsExpandCollapsePatternAvailablePropertyId,
        UIA_IsScrollItemPatternAvailablePropertyId, UIA_ValueValuePropertyId, UIA_ValueIsReadOnlyPropertyId,
        UIA_ToggleToggleStatePropertyId, UIA_SelectionItemIsSelectedPropertyId, UIA_ExpandCollapseExpandCollapseStatePropertyId,
    };
    for (auto id : required) {
        hr = request->AddProperty(id);
        if (FAILED(hr)) break;
    }
    if (SUCCEEDED(hr)) {
        request->AddProperty(UIA_AriaRolePropertyId);
        request->AddProperty(UIA_HeadingLevelPropertyId);
        request->AddProperty(UIA_IsDialogPropertyId);
        hr = request->put_TreeScope(scope);
    }
    if (SUCCEEDED(hr)) hr = request->put_TreeFilter(u.view);
    if (FAILED(hr)) { request->Release(); return hr; }
    *out = request;
    return S_OK;
}

static DWORD WINAPI uia_thread(void *) {
    auto &u = uia();
    DpiScope dpi;
    HRESULT hr = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
    bool com = SUCCEEDED(hr);
    if (com) hr = CoCreateInstance(__uuidof(CUIAutomation8), nullptr, CLSCTX_INPROC_SERVER, __uuidof(IUIAutomation2), reinterpret_cast<void **>(&u.automation));
    if (SUCCEEDED(hr)) hr = u.automation->put_ConnectionTimeout(1000);
    if (SUCCEEDED(hr)) hr = u.automation->put_TransactionTimeout(1500);
    if (SUCCEEDED(hr)) hr = u.automation->get_ControlViewCondition(&u.view);
    if (SUCCEEDED(hr)) hr = make_cache(u, TreeScope_Element, &u.element_cache);
    if (SUCCEEDED(hr)) hr = make_cache(u, TreeScope_Children, &u.children_cache);
    if (SUCCEEDED(hr) && !dpi.previous) hr = E_FAIL;
    {
        std::lock_guard<std::mutex> lock(u.mutex);
        u.status = hr;
    }
    SetEvent(u.ready);
    for (;;) {
        std::shared_ptr<UiaJob> job;
        {
            std::unique_lock<std::mutex> lock(u.mutex);
            u.wake.wait(lock, [&u] { return u.stop || !u.jobs.empty(); });
            if (u.stop) break;
            job = u.jobs.front();
            u.jobs.pop_front();
        }
        int expected = JOB_QUEUED;
        if (job->state.compare_exchange_strong(expected, JOB_RUNNING)) {
            try { job->result = SUCCEEDED(hr) ? job->run() : UR_UNAVAILABLE; } catch (...) { job->result = UR_FAILED; }
        }
        SetEvent(job->done);
    }
    for (auto &entry : u.snapshots) release_snapshot(entry.second);
    u.snapshots.clear();
    if (u.children_cache) u.children_cache->Release();
    if (u.element_cache) u.element_cache->Release();
    if (u.view) u.view->Release();
    if (u.automation) u.automation->Release();
    if (com) CoUninitialize();
    return 0;
}

static void uia_shutdown() {
    auto &u = uia();
    {
        std::lock_guard<std::mutex> lock(u.mutex);
        u.stop = true;
    }
    u.wake.notify_all();
    if (u.thread) WaitForSingleObject(u.thread, 300);
}

static bool uia_ready() {
    auto &u = uia();
    std::call_once(u.start, [&u] {
        if (!u.ready) return;
        u.thread = CreateThread(nullptr, 0, uia_thread, nullptr, 0, nullptr);
        if (!u.thread) { SetEvent(u.ready); return; }
        std::atexit(uia_shutdown);
    });
    if (!u.ready || WaitForSingleObject(u.ready, 2000) != WAIT_OBJECT_0) return false;
    std::lock_guard<std::mutex> lock(u.mutex);
    return u.thread && SUCCEEDED(u.status) && !u.stop;
}

static int submit(const std::shared_ptr<UiaJob> &job, uint32_t timeout_ms, bool wait_result) {
    auto &u = uia();
    if (!uia_ready()) return UR_UNAVAILABLE;
    if (!job->done) return UR_FAILED;
    {
        std::lock_guard<std::mutex> lock(u.mutex);
        if (u.stop) return UR_UNAVAILABLE;
        if (u.jobs.size() >= 16) return UR_BUSY;
        u.jobs.push_back(job);
    }
    u.wake.notify_one();
    if (!wait_result) return UR_OK;
    ULONGLONG until = GetTickCount64() + timeout_ms;
    for (;;) {
        ULONGLONG now = GetTickCount64();
        DWORD remaining = now >= until ? 0 : static_cast<DWORD>(until - now);
        DWORD waited = MsgWaitForMultipleObjects(1, &job->done, FALSE, remaining, QS_ALLINPUT);
        if (waited == WAIT_OBJECT_0) return job->result;
        if (waited == WAIT_OBJECT_0 + 1) {
            MSG message;
            while (PeekMessageW(&message, nullptr, 0, 0, PM_REMOVE)) {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            continue;
        }
        int expected = JOB_QUEUED;
        return job->state.compare_exchange_strong(expected, JOB_ABANDONED) ? UR_NOT_STARTED : UR_TIMEOUT;
    }
}

static int uia_error(HRESULT hr) {
    if (hr == UIA_E_ELEMENTNOTAVAILABLE) return UR_STALE;
    if (hr == UIA_E_TIMEOUT) return UR_TIMEOUT;
    if (hr == UIA_E_ELEMENTNOTENABLED) return UR_DISABLED;
    if (hr == E_ACCESSDENIED) return UR_DENIED;
    return UR_FAILED;
}

static std::wstring clip(const wchar_t *text, size_t length, size_t limit) {
    if (!text) return {};
    if (length > limit) {
        length = limit;
        if (length && text[length - 1] >= 0xd800 && text[length - 1] <= 0xdbff) --length;
    }
    return std::wstring(text, length);
}

static std::wstring take_bstr(BSTR text, size_t limit) {
    std::wstring result = clip(text, text ? SysStringLen(text) : 0, limit);
    SysFreeString(text);
    return result;
}

static void json_string(std::string &out, const std::wstring &text) {
    std::string utf8;
    if (!text.empty()) {
        int size = WideCharToMultiByte(CP_UTF8, 0, text.data(), static_cast<int>(text.size()), nullptr, 0, nullptr, nullptr);
        if (size > 0) {
            utf8.resize(static_cast<size_t>(size));
            WideCharToMultiByte(CP_UTF8, 0, text.data(), static_cast<int>(text.size()), utf8.data(), size, nullptr, nullptr);
        }
    }
    out += '"';
    for (char c : utf8) {
        auto byte = static_cast<unsigned char>(c);
        if (c == '"' || c == '\\') { out += '\\'; out += c; }
        else if (byte < 0x20) { char escaped[8]; std::snprintf(escaped, sizeof(escaped), "\\u%04x", byte); out += escaped; }
        else out += c;
    }
    out += '"';
}

static void json_rect(std::string &out, LONG left, LONG top, LONG right, LONG bottom) {
    out += '[' + std::to_string(left) + ',' + std::to_string(top) + ',' + std::to_string(right > left ? right - left : 0)
        + ',' + std::to_string(bottom > top ? bottom - top : 0) + ']';
}

static bool cached_bool(IUIAutomationElement *element, PROPERTYID id) {
    VARIANT value;
    VariantInit(&value);
    bool result = SUCCEEDED(element->GetCachedPropertyValueEx(id, TRUE, &value)) && value.vt == VT_BOOL && value.boolVal == VARIANT_TRUE;
    VariantClear(&value);
    return result;
}

static int cached_int(IUIAutomationElement *element, PROPERTYID id, int fallback) {
    VARIANT value;
    VariantInit(&value);
    int result = SUCCEEDED(element->GetCachedPropertyValueEx(id, TRUE, &value)) && value.vt == VT_I4 ? value.lVal : fallback;
    VariantClear(&value);
    return result;
}

static std::wstring cached_text(IUIAutomationElement *element, PROPERTYID id, size_t limit) {
    VARIANT value;
    VariantInit(&value);
    std::wstring result;
    if (SUCCEEDED(element->GetCachedPropertyValueEx(id, TRUE, &value)) && value.vt == VT_BSTR) result = clip(value.bstrVal, SysStringLen(value.bstrVal), limit);
    VariantClear(&value);
    return result;
}

static std::vector<int> cached_runtime_id(IUIAutomationElement *element) {
    VARIANT value;
    VariantInit(&value);
    std::vector<int> result;
    if (SUCCEEDED(element->GetCachedPropertyValue(UIA_RuntimeIdPropertyId, &value)) && value.vt == (VT_ARRAY | VT_I4) && value.parray) {
        LONG lower = 0, upper = -1;
        SafeArrayGetLBound(value.parray, 1, &lower);
        SafeArrayGetUBound(value.parray, 1, &upper);
        for (LONG i = lower; i <= upper && result.size() < 64; ++i) {
            int item = 0;
            if (SUCCEEDED(SafeArrayGetElement(value.parray, &i, &item))) result.push_back(item);
        }
    }
    VariantClear(&value);
    return result;
}

struct UiaFacts {
    CONTROLTYPEID type = 0;
    std::wstring name;
    uint32_t patterns = 0, states = 0;
    bool offscreen = false;
    DWORD pid = 0;
    RECT bounds{};
};

static UiaFacts read_facts(IUIAutomationElement *element) {
    UiaFacts facts;
    element->get_CachedControlType(&facts.type);
    BSTR name = nullptr;
    if (SUCCEEDED(element->get_CachedName(&name))) facts.name = take_bstr(name, UIA_MAX_NAME);
    BOOL flag = FALSE;
    if (SUCCEEDED(element->get_CachedIsOffscreen(&flag))) facts.offscreen = flag != FALSE;
    flag = TRUE;
    if (SUCCEEDED(element->get_CachedIsEnabled(&flag)) && !flag) facts.states |= ST_DISABLED;
    flag = FALSE;
    if (SUCCEEDED(element->get_CachedHasKeyboardFocus(&flag)) && flag) facts.states |= ST_FOCUSED;
    flag = FALSE;
    if (SUCCEEDED(element->get_CachedIsKeyboardFocusable(&flag)) && flag) facts.patterns |= PAT_FOCUSABLE;
    int pid = 0;
    if (SUCCEEDED(element->get_CachedProcessId(&pid))) facts.pid = static_cast<DWORD>(pid);
    element->get_CachedBoundingRectangle(&facts.bounds);
    if (cached_bool(element, UIA_IsInvokePatternAvailablePropertyId)) facts.patterns |= PAT_INVOKE;
    if (cached_bool(element, UIA_IsScrollItemPatternAvailablePropertyId)) facts.patterns |= PAT_SCROLL_ITEM;
    if (cached_bool(element, UIA_IsValuePatternAvailablePropertyId)) {
        facts.patterns |= PAT_VALUE;
        if (cached_bool(element, UIA_ValueIsReadOnlyPropertyId)) facts.states |= ST_READONLY;
    }
    if (cached_bool(element, UIA_IsTogglePatternAvailablePropertyId)) {
        facts.patterns |= PAT_TOGGLE;
        int toggle = cached_int(element, UIA_ToggleToggleStatePropertyId, ToggleState_Off);
        if (toggle == ToggleState_On) facts.states |= ST_CHECKED;
        else if (toggle == ToggleState_Indeterminate) facts.states |= ST_MIXED;
    }
    if (cached_bool(element, UIA_IsSelectionItemPatternAvailablePropertyId)) {
        facts.patterns |= PAT_SELECTION_ITEM;
        if (cached_bool(element, UIA_SelectionItemIsSelectedPropertyId)) facts.states |= ST_SELECTED;
    }
    if (cached_bool(element, UIA_IsExpandCollapsePatternAvailablePropertyId)) {
        int state = cached_int(element, UIA_ExpandCollapseExpandCollapseStatePropertyId, ExpandCollapseState_LeafNode);
        if (state != ExpandCollapseState_LeafNode) facts.patterns |= PAT_EXPAND_COLLAPSE;
        if (state == ExpandCollapseState_Expanded) facts.states |= ST_EXPANDED;
        else if (state == ExpandCollapseState_Collapsed) facts.states |= ST_COLLAPSED;
        else if (state == ExpandCollapseState_PartiallyExpanded) facts.states |= ST_PARTIAL;
    }
    if (cached_bool(element, UIA_IsDialogPropertyId)) facts.states |= ST_DIALOG;
    return facts;
}

static void node_json(std::string &out, IUIAutomationElement *element, const UiaFacts &facts, int parent, HWND root) {
    out += "{\"p\":" + std::to_string(parent) + ",\"ct\":" + std::to_string(facts.type) + ",\"n\":";
    json_string(out, facts.name);
    BSTR text = nullptr;
    out += ",\"aid\":";
    json_string(out, SUCCEEDED(element->get_CachedAutomationId(&text)) ? take_bstr(text, 128) : std::wstring());
    text = nullptr;
    out += ",\"cls\":";
    json_string(out, SUCCEEDED(element->get_CachedClassName(&text)) ? take_bstr(text, 128) : std::wstring());
    out += ",\"aria\":";
    json_string(out, cached_text(element, UIA_AriaRolePropertyId, 64));
    int heading = cached_int(element, UIA_HeadingLevelPropertyId, HeadingLevel_None);
    out += ",\"hl\":" + std::to_string(heading > HeadingLevel_None && heading <= HeadingLevel9 ? heading - HeadingLevel_None : 0);
    out += ",\"pat\":" + std::to_string(facts.patterns) + ",\"st\":" + std::to_string(facts.states) + ",\"b\":";
    json_rect(out, facts.bounds.left, facts.bounds.top, facts.bounds.right, facts.bounds.bottom);
    if (facts.patterns & PAT_VALUE) {
        out += ",\"v\":";
        json_string(out, cached_text(element, UIA_ValueValuePropertyId, 1024));
    }
    out += ",\"pid\":" + std::to_string(facts.pid) + ",\"hwnd\":" + std::to_string(reinterpret_cast<uintptr_t>(root)) + '}';
}

static bool container_type(CONTROLTYPEID type) {
    switch (type) {
        case UIA_PaneControlTypeId: case UIA_GroupControlTypeId: case UIA_CustomControlTypeId: case UIA_TextControlTypeId:
        case UIA_ImageControlTypeId: case UIA_CalendarControlTypeId: case UIA_ProgressBarControlTypeId: case UIA_ScrollBarControlTypeId:
        case UIA_SpinnerControlTypeId: case UIA_StatusBarControlTypeId: case UIA_ToolTipControlTypeId: case UIA_ThumbControlTypeId:
        case UIA_TitleBarControlTypeId: case UIA_SeparatorControlTypeId: case UIA_SemanticZoomControlTypeId: case UIA_HeaderControlTypeId:
            return true;
        default:
            return false;
    }
}

static bool leaf_type(CONTROLTYPEID type) {
    return type == UIA_TextControlTypeId || type == UIA_ImageControlTypeId || type == UIA_SeparatorControlTypeId
        || type == UIA_ThumbControlTypeId || type == UIA_ScrollBarControlTypeId || type == UIA_ProgressBarControlTypeId;
}

static RECT primary_rect() {
    MONITORINFO info{};
    info.cbSize = sizeof(info);
    POINT origin{0, 0};
    if (!GetMonitorInfoW(MonitorFromPoint(origin, MONITOR_DEFAULTTOPRIMARY), &info)) return RECT{};
    return info.rcMonitor;
}

static bool shown_on(HWND window, const RECT &display, RECT &rect) {
    DWORD cloaked = 0;
    if (!IsWindowVisible(window) || IsIconic(window) || !GetWindowRect(window, &rect)) return false;
    if (SUCCEEDED(DwmGetWindowAttribute(window, DWMWA_CLOAKED, &cloaked, sizeof(cloaked))) && cloaked) return false;
    RECT overlap{};
    return rect.right > rect.left && rect.bottom > rect.top && IntersectRect(&overlap, &rect, &display);
}

static std::wstring process_name(DWORD pid) {
    HANDLE process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid);
    if (!process) return {};
    wchar_t path[MAX_PATH]{};
    DWORD length = MAX_PATH;
    std::wstring result;
    if (QueryFullProcessImageNameW(process, 0, path, &length)) {
        std::wstring full(path, length);
        size_t slash = full.find_last_of(L"\\/");
        result = slash == std::wstring::npos ? full : full.substr(slash + 1);
    }
    CloseHandle(process);
    return result;
}

struct WindowSearch {
    RECT display{};
    HWND foreground = nullptr;
    std::string json;
    uint32_t count = 0;
    HWND root = nullptr;
    DWORD root_pid = 0;
    std::vector<HWND> popups;
};

static BOOL CALLBACK collect_window(HWND window, LPARAM context) {
    auto &search = *reinterpret_cast<WindowSearch *>(context);
    RECT rect{};
    if (search.count >= 32 || !shown_on(window, search.display, rect)) return TRUE;
    wchar_t title[256]{};
    int length = InternalGetWindowText(window, title, 256);
    bool foreground = window == search.foreground;
    if ((length <= 0 || (GetWindowLongPtrW(window, GWL_EXSTYLE) & WS_EX_TOOLWINDOW)) && !foreground) return TRUE;
    DWORD pid = 0;
    GetWindowThreadProcessId(window, &pid);
    wchar_t cls[128]{};
    int cls_length = GetClassNameW(window, cls, 128);
    if (search.count) search.json += ',';
    search.json += "{\"hwnd\":" + std::to_string(reinterpret_cast<uintptr_t>(window)) + ",\"pid\":" + std::to_string(pid) + ",\"title\":";
    json_string(search.json, std::wstring(title, length > 0 ? static_cast<size_t>(length) : 0));
    search.json += ",\"process\":";
    json_string(search.json, process_name(pid));
    search.json += ",\"class\":";
    json_string(search.json, std::wstring(cls, cls_length > 0 ? static_cast<size_t>(cls_length) : 0));
    search.json += ",\"bounds\":";
    json_rect(search.json, rect.left, rect.top, rect.right, rect.bottom);
    search.json += std::string(",\"foreground\":") + (foreground ? "true" : "false") + ",\"accessible\":" + (process_allowed(pid) ? "true" : "false") + '}';
    ++search.count;
    return TRUE;
}

static BOOL CALLBACK collect_popup(HWND window, LPARAM context) {
    auto &search = *reinterpret_cast<WindowSearch *>(context);
    RECT rect{};
    if (window == search.root || search.popups.size() >= UIA_MAX_POPUPS || !shown_on(window, search.display, rect)) return TRUE;
    DWORD pid = 0;
    GetWindowThreadProcessId(window, &pid);
    if (pid != search.root_pid) return TRUE;
    wchar_t cls[16]{};
    GetClassNameW(window, cls, 16);
    bool owned = std::wcscmp(cls, L"#32768") == 0;
    HWND owner = GetWindow(window, GW_OWNER);
    for (int depth = 0; owner && !owned && depth < 16; ++depth) {
        owned = owner == search.root;
        owner = GetWindow(owner, GW_OWNER);
    }
    if (owned) search.popups.push_back(window);
    return TRUE;
}

static bool window_matches(HWND window, DWORD pid) {
    DWORD owner = 0;
    return window && IsWindow(window) && GetWindowThreadProcessId(window, &owner) && owner == pid;
}

static int snapshot_job(Uia &u, HWND requested, DWORD requested_pid, uint32_t max_nodes, uint32_t budget_ms, std::string &json) {
    ULONGLONG deadline = GetTickCount64() + budget_ms;
    if (requested && !window_matches(requested, requested_pid)) return UR_STALE;
    WindowSearch search;
    search.display = primary_rect();
    search.foreground = GetForegroundWindow();
    EnumWindows(collect_window, reinterpret_cast<LPARAM>(&search));
    search.root = requested ? requested : GetAncestor(search.foreground, GA_ROOT);
    if (search.root) GetWindowThreadProcessId(search.root, &search.root_pid);
    json = "{\"root\":" + std::to_string(reinterpret_cast<uintptr_t>(search.root)) + ",\"root_pid\":" + std::to_string(search.root_pid)
        + ",\"windows\":[" + search.json + "],\"nodes\":[";
    UiaSnapshot snapshot;
    const char *reason = nullptr;
    bool truncated = false;
    uint32_t visited = 0, counted = 0;
    if (!search.root) reason = "no_window";
    else if (!process_allowed(search.root_pid)) reason = "window_inaccessible";
    else {
        std::vector<HWND> roots{search.root};
        EnumWindows(collect_popup, reinterpret_cast<LPARAM>(&search));
        roots.insert(roots.end(), search.popups.begin(), search.popups.end());
        // A visit owns one reference to the element it expands; `out` is the
        // returned node its children hang under. Offscreen nodes are not
        // returned; one without geometry (a WinUI Popup window) is a logical
        // container whose children may be on screen, so it is still expanded.
        struct Visit { IUIAutomationElement *element; int out; int label; uint32_t depth; HWND root; DWORD root_pid; };
        std::deque<Visit> queue;
        std::vector<std::wstring> labels;
        std::set<std::vector<int>> seen;
        auto add = [&](IUIAutomationElement *element, int parent, int label, HWND root, DWORD root_pid, uint32_t depth, bool is_root) {
            ++visited;
            UiaFacts facts = read_facts(element);
            std::vector<int> id = cached_runtime_id(element);
            if (!id.empty() && !seen.insert(id).second) { element->Release(); return; }
            if (facts.offscreen && !is_root) {
                bool geometry = facts.bounds.right > facts.bounds.left && facts.bounds.bottom > facts.bounds.top;
                if (geometry || leaf_type(facts.type)) element->Release();
                else queue.push_back(Visit{element, parent, label, depth, root, root_pid});
                return;
            }
            uint32_t actions = facts.patterns & ~PAT_SCROLL_ITEM;
            bool redundant = (facts.type == UIA_TextControlTypeId || facts.type == UIA_ImageControlTypeId) && !actions && !facts.name.empty()
                && label >= 0 && labels[static_cast<size_t>(label)].find(facts.name) != std::wstring::npos;
            bool counts = is_root || (!redundant && !(facts.name.empty() && !actions && container_type(facts.type)));
            if (snapshot.nodes.size()) json += ',';
            node_json(json, element, facts, parent, root);
            UiaNode node;
            node.element = element;
            node.root = root;
            node.root_pid = root_pid;
            node.pid = facts.pid;
            node.id = std::move(id);
            snapshot.nodes.push_back(std::move(node));
            int next_label = label;
            if (counts) {
                ++counted;
                labels.push_back(facts.name);
                next_label = static_cast<int>(labels.size() - 1);
            }
            if (!leaf_type(facts.type)) {
                element->AddRef();
                queue.push_back(Visit{element, static_cast<int>(snapshot.nodes.size() - 1), next_label, depth, root, root_pid});
            }
        };
        for (auto root : roots) {
            IUIAutomationElement *element = nullptr;
            HRESULT hr = u.automation->ElementFromHandleBuildCache(root, u.element_cache, &element);
            if (SUCCEEDED(hr) && element) add(element, -1, -1, root, search.root_pid, 0, true);
            else if (root == search.root) reason = uia_error(hr) == UR_TIMEOUT ? "backend_timeout" : "backend_error";
        }
        while (!queue.empty()) {
            if (GetTickCount64() >= deadline) { reason = "timeout"; break; }
            if (counted >= max_nodes) { reason = "max_nodes"; break; }
            if (visited >= UIA_MAX_VISITED) { reason = "max_visited"; break; }
            Visit visit = queue.front();
            queue.pop_front();
            IUIAutomationElement *updated = nullptr;
            HRESULT hr = visit.depth < 256 ? visit.element->BuildUpdatedCache(u.children_cache, &updated) : E_ABORT;
            visit.element->Release();
            if (FAILED(hr) || !updated) {
                if (hr == UIA_E_TIMEOUT) reason = "backend_timeout";
                continue;
            }
            IUIAutomationElementArray *children = nullptr;
            int length = 0;
            if (SUCCEEDED(updated->GetCachedChildren(&children)) && children && SUCCEEDED(children->get_Length(&length))) {
                for (int i = 0; i < length; ++i) {
                    if (visited >= UIA_MAX_VISITED) { reason = "max_visited"; truncated = true; break; }
                    IUIAutomationElement *child = nullptr;
                    if (SUCCEEDED(children->GetElement(i, &child)) && child)
                        add(child, visit.out, visit.label, visit.root, visit.root_pid, visit.depth + 1, false);
                }
            }
            if (children) children->Release();
            updated->Release();
        }
        truncated = truncated || !queue.empty() || (reason && std::strcmp(reason, "backend_timeout") == 0);
        for (auto &visit : queue) visit.element->Release();
    }
    json += "],\"visited\":" + std::to_string(visited) + ",\"truncated\":" + (truncated ? "true" : "false") + ",\"reason\":";
    json_string(json, reason ? std::wstring(reason, reason + std::strlen(reason)) : std::wstring());
    uint64_t id = u.next_snapshot++;
    json += ",\"snapshot\":" + std::to_string(id) + '}';
    while (u.snapshots.size() >= UIA_MAX_SNAPSHOTS) {
        release_snapshot(u.snapshots.begin()->second);
        u.snapshots.erase(u.snapshots.begin());
    }
    u.snapshots.emplace(id, std::move(snapshot));
    return UR_OK;
}

static int refresh(Uia &u, UiaNode &node, IUIAutomationElement **fresh) {
    *fresh = nullptr;
    if (!node.element || !window_matches(node.root, node.root_pid)) return UR_STALE;
    HRESULT hr = node.element->BuildUpdatedCache(u.element_cache, fresh);
    if (FAILED(hr) || !*fresh) return FAILED(hr) ? uia_error(hr) : UR_STALE;
    int pid = 0;
    if (cached_runtime_id(*fresh) != node.id || FAILED((*fresh)->get_CachedProcessId(&pid)) || static_cast<DWORD>(pid) != node.pid) {
        (*fresh)->Release();
        *fresh = nullptr;
        return UR_STALE;
    }
    return UR_OK;
}

static int element_job(Uia &u, uint64_t snapshot_id, uint32_t index, uint32_t op, const std::wstring &text, std::string &json) {
    auto found = u.snapshots.find(snapshot_id);
    if (found == u.snapshots.end() || index >= found->second.nodes.size()) return UR_STALE;
    UiaNode &node = found->second.nodes[index];
    IUIAutomationElement *fresh = nullptr;
    int status = refresh(u, node, &fresh);
    if (status != UR_OK) return status;
    UiaFacts facts = read_facts(fresh);
    HRESULT hr = S_OK;
    json = "{";
    if (op == 0) {
        POINT point{(facts.bounds.left + facts.bounds.right) / 2, (facts.bounds.top + facts.bounds.bottom) / 2};
        BOOL clickable = FALSE;
        POINT offered{};
        if (SUCCEEDED(fresh->GetClickablePoint(&offered, &clickable)) && clickable) point = offered;
        POINT center{(facts.bounds.left + facts.bounds.right) / 2, (facts.bounds.top + facts.bounds.bottom) / 2};
        HWND at = WindowFromPoint(point), at_center = WindowFromPoint(center);
        bool obscured = !at || GetAncestor(at, GA_ROOT) != node.root;
        bool center_obscured = !at_center || GetAncestor(at_center, GA_ROOT) != node.root;
        json += "\"off\":" + std::string(facts.offscreen ? "true" : "false") + ",\"st\":" + std::to_string(facts.states)
            + ",\"pat\":" + std::to_string(facts.patterns) + ",\"b\":";
        json_rect(json, facts.bounds.left, facts.bounds.top, facts.bounds.right, facts.bounds.bottom);
        json += ",\"pt\":[" + std::to_string(point.x) + ',' + std::to_string(point.y) + "],\"obscured\":" + (obscured ? "true" : "false")
            + ",\"center_obscured\":" + (center_obscured ? "true" : "false");
    } else if (facts.states & ST_DISABLED) {
        status = UR_DISABLED;
    } else if (op == 1) {
        hr = fresh->SetFocus();
        bool focused = false;
        for (int attempt = 0; SUCCEEDED(hr) && !focused && attempt < 10; ++attempt) {
            IUIAutomationElement *after = nullptr;
            if (SUCCEEDED(node.element->BuildUpdatedCache(u.element_cache, &after)) && after) {
                BOOL flag = FALSE;
                focused = SUCCEEDED(after->get_CachedHasKeyboardFocus(&flag)) && flag;
                after->Release();
            }
            if (!focused) Sleep(20);
        }
        json += std::string("\"focused\":") + (focused ? "true" : "false");
    } else if (op == 2) {
        IUIAutomationInvokePattern *pattern = nullptr;
        hr = fresh->GetCurrentPatternAs(UIA_InvokePatternId, __uuidof(IUIAutomationInvokePattern), reinterpret_cast<void **>(&pattern));
        if (SUCCEEDED(hr) && !pattern) hr = E_NOINTERFACE;
        if (SUCCEEDED(hr)) hr = pattern->Invoke();
        if (pattern) pattern->Release();
        if (SUCCEEDED(hr)) {
            IUIAutomationElement *after = nullptr;
            bool gone = refresh(u, node, &after) != UR_OK;
            json += std::string("\"gone\":") + (gone ? "true" : "false");
            if (after) {
                UiaFacts current = read_facts(after);
                json += ",\"st\":" + std::to_string(current.states);
                after->Release();
            }
        }
    } else if (op == 3) {
        IUIAutomationValuePattern *pattern = nullptr;
        hr = fresh->GetCurrentPatternAs(UIA_ValuePatternId, __uuidof(IUIAutomationValuePattern), reinterpret_cast<void **>(&pattern));
        if (SUCCEEDED(hr) && !pattern) hr = E_NOINTERFACE;
        BSTR value = nullptr;
        if (SUCCEEDED(hr)) {
            value = SysAllocStringLen(text.data(), static_cast<UINT>(text.size()));
            hr = value ? pattern->SetValue(value) : E_OUTOFMEMORY;
            SysFreeString(value);
        }
        if (SUCCEEDED(hr)) {
            BSTR current = nullptr;
            if (SUCCEEDED(pattern->get_CurrentValue(&current)) && current) {
                json += "\"v\":";
                json_string(json, take_bstr(current, UIA_MAX_VALUE));
            } else SysFreeString(current);
        }
        if (pattern) pattern->Release();
    } else {
        status = UR_FAILED;
    }
    fresh->Release();
    json += '}';
    if (status == UR_OK && FAILED(hr)) status = uia_error(hr);
    return status;
}

static int take_json(const std::shared_ptr<UiaJob> &job, int status, char **json, size_t *length) {
    if (status != UR_OK) return status;
    *json = static_cast<char *>(std::malloc(job->json.size() + 1));
    if (!*json) return UR_FAILED;
    std::memcpy(*json, job->json.data(), job->json.size());
    (*json)[job->json.size()] = 0;
    *length = job->json.size();
    return UR_OK;
}

extern "C" int bucky_semantic_status() {
    try { return initialize() && uia_ready() ? UR_OK : UR_UNAVAILABLE; } catch (...) { return UR_UNAVAILABLE; }
}

extern "C" int bucky_semantic_snapshot(uint64_t root, uint32_t root_pid, uint32_t max_nodes, uint32_t budget_ms, char **json, size_t *length) {
    if (!json || !length) return UR_FAILED;
    *json = nullptr;
    *length = 0;
    try {
        if (!initialize()) return UR_UNAVAILABLE;
        auto job = std::make_shared<UiaJob>();
        UiaJob *raw = job.get();
        HWND window = reinterpret_cast<HWND>(static_cast<uintptr_t>(root));
        job->run = [raw, window, root_pid, max_nodes, budget_ms] {
            return snapshot_job(uia(), window, root_pid, max_nodes, budget_ms, raw->json);
        };
        return take_json(job, submit(job, budget_ms + 2500, true), json, length);
    } catch (...) { return UR_FAILED; }
}

extern "C" int bucky_semantic_element(uint64_t snapshot, uint32_t index, uint32_t op, const uint16_t *text, size_t text_length,
                                 uint32_t timeout_ms, char **json, size_t *length) {
    if (!json || !length || (text_length && !text) || op > 3) return UR_FAILED;
    *json = nullptr;
    *length = 0;
    try {
        auto job = std::make_shared<UiaJob>();
        UiaJob *raw = job.get();
        std::wstring value = text_length ? std::wstring(reinterpret_cast<const wchar_t *>(text), text_length) : std::wstring();
        job->run = [raw, snapshot, index, op, value] { return element_job(uia(), snapshot, index, op, value, raw->json); };
        return take_json(job, submit(job, timeout_ms, true), json, length);
    } catch (...) { return UR_FAILED; }
}

extern "C" void bucky_semantic_retain(uint64_t snapshot, const uint32_t *indices, size_t count) {
    try {
        std::vector<uint32_t> keep(indices, indices + (indices ? count : 0));
        auto job = std::make_shared<UiaJob>();
        job->run = [snapshot, keep] {
            auto found = uia().snapshots.find(snapshot);
            if (found == uia().snapshots.end()) return UR_OK;
            std::vector<bool> kept(found->second.nodes.size(), false);
            for (auto index : keep) if (index < kept.size()) kept[index] = true;
            for (size_t i = 0; i < kept.size(); ++i) {
                auto &node = found->second.nodes[i];
                if (!kept[i] && node.element) { node.element->Release(); node.element = nullptr; }
            }
            return UR_OK;
        };
        submit(job, 0, false);
    } catch (...) {}
}

extern "C" void bucky_semantic_release(uint64_t snapshot) {
    try {
        auto job = std::make_shared<UiaJob>();
        job->run = [snapshot] {
            auto found = uia().snapshots.find(snapshot);
            if (found != uia().snapshots.end()) {
                release_snapshot(found->second);
                uia().snapshots.erase(found);
            }
            return UR_OK;
        };
        submit(job, 0, false);
    } catch (...) {}
}

#ifdef BUCKY_DESKTOP_SELF_TEST
#include <cstdio>

static WNDPROC test_edit_proc = nullptr;
static unsigned test_clicks = 0, test_vertical = 0, test_horizontal = 0;
static LRESULT CALLBACK test_edit(HWND window, UINT message, WPARAM wparam, LPARAM lparam) {
    if (message == WM_LBUTTONDOWN) ++test_clicks;
    if (message == WM_MOUSEWHEEL) ++test_vertical;
    if (message == WM_MOUSEHWHEEL) ++test_horizontal;
    return CallWindowProcW(test_edit_proc, window, message, wparam, lparam);
}

static void pump_test(DWORD duration) {
    ULONGLONG until = GetTickCount64() + duration;
    do {
        MSG message;
        while (PeekMessageW(&message, nullptr, 0, 0, PM_REMOVE)) {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        Sleep(1);
    } while (GetTickCount64() < until);
}

static bool test_text(HWND window, const wchar_t *text) {
    for (size_t i = 0; text[i]; ++i) {
        if (GetForegroundWindow() != window) return false;
        size_t length = text[i] >= 0xd800 && text[i] <= 0xdbff && text[i + 1] >= 0xdc00 && text[i + 1] <= 0xdfff ? 2 : 1;
        if (bucky_desktop_key(0, 0, reinterpret_cast<const uint16_t *>(text + i), length) || bucky_desktop_finish()) return false;
        if (length == 2) ++i;
        pump_test(5);
    }
    return true;
}

static int reject_test_input(const void *context, const BuckyDesktop *) {
    static_cast<std::atomic<unsigned> *>(const_cast<void *>(context))->fetch_add(1);
    return 0;
}

static unsigned test_invoked = 0;
static LRESULT CALLBACK test_uia_proc(HWND window, UINT message, WPARAM wparam, LPARAM lparam) {
    if (message == WM_COMMAND && LOWORD(wparam) == 101 && HIWORD(wparam) == BN_CLICKED) ++test_invoked;
    return DefWindowProcW(window, message, wparam, lparam);
}

static LRESULT CALLBACK test_hung_proc(HWND window, UINT message, WPARAM wparam, LPARAM lparam) {
    if (message == WM_APP) Sleep(8000);
    return DefWindowProcW(window, message, wparam, lparam);
}

struct HungWindow { HWND window = nullptr; HANDLE created = CreateEventW(nullptr, TRUE, FALSE, nullptr); int x = 0, y = 0; };

static DWORD WINAPI test_hung_thread(void *parameter) {
    auto &hung = *static_cast<HungWindow *>(parameter);
    DpiScope dpi;
    WNDCLASSW cls{};
    cls.lpfnWndProc = test_hung_proc;
    cls.hInstance = GetModuleHandleW(nullptr);
    cls.lpszClassName = L"BuckyUiaHungSelfTest";
    RegisterClassW(&cls);
    hung.window = CreateWindowExW(0, cls.lpszClassName, L"BuckyOS UIA hung self-test", WS_OVERLAPPEDWINDOW | WS_VISIBLE,
        hung.x, hung.y, 300, 200, nullptr, nullptr, cls.hInstance, nullptr);
    SetEvent(hung.created);
    MSG message;
    while (GetMessageW(&message, nullptr, 0, 0) > 0) DispatchMessageW(&message);
    return 0;
}

static int find_test_node(const std::string &tree, const char *needle) {
    size_t nodes = tree.find("\"nodes\":[");
    size_t at = tree.find(needle, nodes == std::string::npos ? 0 : nodes);
    if (nodes == std::string::npos || at == std::string::npos) return -1;
    int index = -1;
    for (size_t pos = tree.find("{\"p\":", nodes); pos != std::string::npos && pos < at; pos = tree.find("{\"p\":", pos + 1)) ++index;
    return index;
}

static int test_uia_call(const std::function<int(char **, size_t *)> &call, std::string &out) {
    char *json = nullptr;
    size_t length = 0;
    int code = call(&json, &length);
    out = code == 0 && json ? std::string(json, length) : std::string();
    bucky_desktop_free(json);
    return code;
}

static int uia_self_test(const BuckyDesktop &desktop) {
    if (bucky_semantic_status() != 0) { std::puts("UI Automation is unavailable"); return 1; }
    DpiScope dpi;
    WNDCLASSW cls{};
    cls.lpfnWndProc = test_uia_proc;
    cls.hInstance = GetModuleHandleW(nullptr);
    cls.lpszClassName = L"BuckyUiaSelfTest";
    RegisterClassW(&cls);
    HWND window = CreateWindowExW(0, cls.lpszClassName, L"BuckyOS UIA self-test", WS_OVERLAPPEDWINDOW,
        static_cast<int>(desktop.x) + 96, static_cast<int>(desktop.y) + 96, 480, 260, nullptr, nullptr, cls.hInstance, nullptr);
    if (!window) return 1;
    HWND button = CreateWindowExW(0, L"BUTTON", L"Bucky Invoke", WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON, 12, 12, 160, 32, window,
        reinterpret_cast<HMENU>(static_cast<INT_PTR>(101)), cls.hInstance, nullptr);
    HWND edit = CreateWindowExW(0, L"EDIT", L"", WS_CHILD | WS_VISIBLE | WS_BORDER | ES_AUTOHSCROLL, 12, 56, 300, 28, window,
        reinterpret_cast<HMENU>(static_cast<INT_PTR>(102)), cls.hInstance, nullptr);
    HWND check = CreateWindowExW(0, L"BUTTON", L"Bucky Check", WS_CHILD | WS_VISIBLE | BS_AUTOCHECKBOX, 12, 96, 160, 28, window,
        reinterpret_cast<HMENU>(static_cast<INT_PTR>(103)), cls.hInstance, nullptr);
    if (!button || !edit || !check) { DestroyWindow(window); return 1; }
    ShowWindow(window, SW_SHOWNOACTIVATE);
    pump_test(200);
    std::string tree, reply;
    ULONGLONG started = GetTickCount64();
    int code = test_uia_call([&](char **json, size_t *length) { return bucky_semantic_snapshot(reinterpret_cast<uintptr_t>(window), GetCurrentProcessId(), 400, 3000, json, length); }, tree);
    std::printf("snapshot code=%d bytes=%zu elapsed=%llums\n", code, tree.size(), static_cast<unsigned long long>(GetTickCount64() - started));
    size_t id_at = tree.rfind("\"snapshot\":");
    uint64_t snapshot = id_at == std::string::npos ? 0 : std::strtoull(tree.c_str() + id_at + 11, nullptr, 10);
    int button_node = find_test_node(tree, "\"n\":\"Bucky Invoke\"");
    int check_node = find_test_node(tree, "\"n\":\"Bucky Check\"");
    int edit_node = find_test_node(tree, "\"ct\":50004");
    bool passed = code == 0 && snapshot && button_node > 0 && check_node > 0 && edit_node > 0
        && tree.find("\"ct\":50002") != std::string::npos && tree.find("\"ct\":50032") != std::string::npos;
    std::printf("tree: button=%d edit=%d checkbox=%d\n", button_node, edit_node, check_node);
    code = test_uia_call([&](char **json, size_t *length) { return bucky_semantic_element(snapshot, static_cast<uint32_t>(button_node), 0, nullptr, 0, 3000, json, length); }, reply);
    passed = passed && code == 0 && reply.find("\"pt\":[") != std::string::npos && reply.find("\"off\":false") != std::string::npos;
    code = test_uia_call([&](char **json, size_t *length) { return bucky_semantic_element(snapshot, static_cast<uint32_t>(button_node), 2, nullptr, 0, 3000, json, length); }, reply);
    pump_test(100);
    std::printf("invoke code=%d clicked=%u\n", code, test_invoked);
    passed = passed && code == 0 && test_invoked == 1;
    const wchar_t *value = L"uia 中文 value";
    code = test_uia_call([&](char **json, size_t *length) { return bucky_semantic_element(snapshot, static_cast<uint32_t>(edit_node), 3, reinterpret_cast<const uint16_t *>(value),
        std::wcslen(value), 3000, json, length); }, reply);
    pump_test(50);
    wchar_t text[64]{};
    GetWindowTextW(edit, text, 64);
    std::printf("set_value code=%d applied=%d\n", code, std::wcscmp(text, value) == 0);
    passed = passed && code == 0 && std::wcscmp(text, value) == 0 && reply.find("\"v\":") != std::string::npos;
    DestroyWindow(button);
    pump_test(50);
    code = test_uia_call([&](char **json, size_t *length) { return bucky_semantic_element(snapshot, static_cast<uint32_t>(button_node), 2, nullptr, 0, 3000, json, length); }, reply);
    pump_test(50);
    std::printf("destroyed element code=%d clicked=%u\n", code, test_invoked);
    passed = passed && code == 1 && test_invoked == 1;
    bucky_semantic_release(snapshot);
    HungWindow hung;
    hung.x = static_cast<int>(desktop.x) + 160;
    hung.y = static_cast<int>(desktop.y) + 160;
    HANDLE thread = CreateThread(nullptr, 0, test_hung_thread, &hung, 0, nullptr);
    if (!thread || WaitForSingleObject(hung.created, 3000) != WAIT_OBJECT_0 || !hung.window) { DestroyWindow(window); return 1; }
    CloseHandle(thread);
    pump_test(100);
    PostMessageW(hung.window, WM_APP, 0, 0);
    Sleep(100);
    started = GetTickCount64();
    code = test_uia_call([&](char **json, size_t *length) { return bucky_semantic_snapshot(reinterpret_cast<uintptr_t>(hung.window), GetCurrentProcessId(), 400, 3000, json, length); }, tree);
    ULONGLONG elapsed = GetTickCount64() - started;
    size_t reason = tree.find("\"reason\":");
    std::printf("hung window snapshot code=%d elapsed=%llums %s\n", code, static_cast<unsigned long long>(elapsed),
        reason == std::string::npos ? "" : tree.substr(reason, 40).c_str());
    passed = passed && elapsed < 6000 && (code == 0 || code == 2 || code == 8);
    std::printf("uia self-test: %s\n", passed ? "passed" : "failed");
    DestroyWindow(window);
    return passed ? 0 : 1;
}

int main(int argc, char **argv) {
    std::atomic<uint64_t> heartbeat{1000};
    std::atomic<bool> healthy{true};
    if (advance_monitor_heartbeat(heartbeat, healthy, 1050, 250) || !healthy.load()) return 1;
    if (!advance_monitor_heartbeat(heartbeat, healthy, 1300, 250) || healthy.load()) return 1;
    if (advance_monitor_heartbeat(heartbeat, healthy, 1310, 250) || healthy.load()) return 1;
    heartbeat.store(1000);
    healthy.store(true);
    if (!advance_monitor_heartbeat(heartbeat, healthy, 1040, 40) || healthy.load()) return 1;
    std::puts("monitor heartbeat gaps fail closed and stay disabled");
    std::vector<INPUT> pending;
    remember(key_input(VK_LCONTROL, false), pending);
    remember(key_input(0x4e2d, false, true), pending);
    remember(mouse_input(MOUSEEVENTF_LEFTDOWN), pending);
    if (pending.size() != 3 || !(pending[0].ki.dwFlags & KEYEVENTF_KEYUP)
        || !(pending[1].ki.dwFlags & KEYEVENTF_UNICODE) || pending[2].mi.dwFlags != MOUSEEVENTF_LEFTUP) return 1;
    remember(key_input(0x4e2d, true, true), pending);
    remember(mouse_input(MOUSEEVENTF_LEFTUP), pending);
    if (pending.size() != 1 || pending[0].ki.wVk != VK_LCONTROL) return 1;
    remember(key_input(VK_LCONTROL, true), pending);
    if (!pending.empty()) return 1;
    BuckyDesktop desktop{};
    bucky_desktop_probe(&desktop);
    std::printf("session=%u ready=%u capture=%u input=%u locked=%u display=%u size=%ux%u focus=%llu physical=%u\n",
        desktop.session, desktop.ready, desktop.capture, desktop.input, desktop.locked, desktop.display,
        desktop.width, desktop.height, static_cast<unsigned long long>(desktop.focus), desktop.physical_input_down);
    if (desktop.input && !desktop.physical_input_down) {
        std::atomic<unsigned> guard_calls{0};
        bucky_desktop_set_guard(reject_test_input, &guard_calls);
        int rejected = bucky_desktop_key('A', 0, nullptr, 0);
        bucky_desktop_set_guard(nullptr, nullptr);
        BuckyDesktop after_guard{};
        bucky_desktop_probe(&after_guard);
        if (rejected != 1 || guard_calls.load() != 1 || desktop.events != after_guard.events) return 1;
        std::puts("pre-dispatch cancellation guard passed without input events");
    }
    if (bucky_desktop_click(-1e12, -1e12, 0, 1, 0) != 1 || bucky_desktop_scroll(0, 0, 0, 1, 1) != 1
        || bucky_desktop_key(256, 0, nullptr, 0) != 1 || bucky_desktop_finish() != 0) return 1;
    if (!desktop.capture) { std::puts("interactive desktop unavailable; capture/input tests skipped"); return 2; }
    unsigned char *png = nullptr;
    size_t length = 0;
    int captured = bucky_desktop_capture(1, desktop.width, desktop.height, 5000, &png, &length);
    bool valid = captured == 0 && length > 24 && std::memcmp(png, "\x89PNG\r\n\x1a\n", 8) == 0;
    bucky_desktop_free(png);
    if (!valid) { std::printf("capture failed: code=%d bytes=%zu\n", captured, length); return 1; }
    std::printf("capture PNG and invalid-input rejection passed: %zu bytes\n", length);
    if (argc == 2 && std::strcmp(argv[1], "--uia") == 0) return uia_self_test(desktop);
    if (argc != 2 || std::strcmp(argv[1], "--input") != 0) return 0;
    DpiScope dpi;
    HWND previous = GetForegroundWindow();
    WNDCLASSW cls{};
    cls.lpfnWndProc = DefWindowProcW;
    cls.hInstance = GetModuleHandleW(nullptr);
    cls.lpszClassName = L"BuckyDesktopSelfTest";
    RegisterClassW(&cls);
    HWND window = CreateWindowExW(0, cls.lpszClassName, L"BuckyOS ComputerUse native self-test", WS_OVERLAPPEDWINDOW,
        static_cast<int>(desktop.x) + 64, static_cast<int>(desktop.y) + 64, 520, 300, nullptr, nullptr, GetModuleHandleW(nullptr), nullptr);
    if (!window) return 1;
    HWND edit = CreateWindowExW(0, L"EDIT", L"", WS_CHILD | WS_VISIBLE | WS_BORDER | ES_MULTILINE | ES_AUTOVSCROLL | WS_VSCROLL,
        12, 12, 470, 220, window, nullptr, GetModuleHandleW(nullptr), nullptr);
    if (!edit) { DestroyWindow(window); return 1; }
    test_edit_proc = reinterpret_cast<WNDPROC>(SetWindowLongPtrW(edit, GWLP_WNDPROC, reinterpret_cast<LONG_PTR>(test_edit)));
    ShowWindow(window, SW_SHOWNORMAL);
    SetForegroundWindow(window);
    SetFocus(edit);
    pump_test(150);
    if (GetForegroundWindow() != window) { DestroyWindow(window); std::puts("could not acquire self-test window focus; input tests skipped"); return 2; }
    BuckyDesktop before{};
    bucky_desktop_probe(&before);
    RECT rect{};
    GetWindowRect(edit, &rect);
    double x = rect.left + 30.0, y = rect.top + 20.0;
    bool passed = before.input && before.focus && before.focus_on_display && !before.physical_input_down;
    passed = passed && bucky_desktop_click(x, y, 0, 1, 0) == 0 && bucky_desktop_finish() == 0;
    pump_test(40);
    passed = passed && test_clicks == 1 && test_text(window, L"BuckyOS \u4e2d\u6587 \xd83d\xde80");
    wchar_t text[256]{};
    GetWindowTextW(edit, text, 256);
    passed = passed && std::wcscmp(text, L"BuckyOS \u4e2d\u6587 \xd83d\xde80") == 0;
    if (passed && GetForegroundWindow() == window) passed = bucky_desktop_key('A', 1ULL << 18, nullptr, 0) == 0 && bucky_desktop_finish() == 0;
    else passed = false;
    pump_test(40);
    passed = passed && test_text(window, L"replacement");
    GetWindowTextW(edit, text, 256);
    passed = passed && std::wcscmp(text, L"replacement") == 0;
    if (passed && GetForegroundWindow() == window) passed = bucky_desktop_scroll(x, y, 1, 1, 0) == 0 && bucky_desktop_finish() == 0;
    else passed = false;
    pump_test(80);
    BuckyDesktop after{};
    bucky_desktop_probe(&after);
    passed = passed && test_vertical > 0 && test_horizontal > 0 && after.events > before.events
        && after.hardware_events == before.hardware_events && !after.physical_input_down;
    std::printf("input self-test: %s (click=%u vertical=%u horizontal=%u own_events=%llu human_events=%llu)\n", passed ? "passed" : "failed",
        test_clicks, test_vertical, test_horizontal, static_cast<unsigned long long>(after.events - before.events),
        static_cast<unsigned long long>(after.hardware_events - before.hardware_events));
    if (GetForegroundWindow() == window && previous) SetForegroundWindow(previous);
    DestroyWindow(window);
    return passed ? 0 : 1;
}
#endif
