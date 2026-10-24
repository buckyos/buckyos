#define NOMINMAX
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <wtsapi32.h>
#include <shellscalingapi.h>
#include <objidl.h>
#include <gdiplus.h>
#include <sddl.h>
#include <atomic>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <cmath>
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
    if (!desktop.input || desktop.physical_input_down) return 1;
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
        if (!desktop.input || desktop.physical_input_down) return 1;
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
