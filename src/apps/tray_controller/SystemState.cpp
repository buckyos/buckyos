#include "SystemState.h"
#include <set>
#include <string>
#include <tlhelp32.h>

void query_buckyos_status(void (*callback)(bool is_success, SystemState::Status status, void* userdata), void* userdata);
bool find_process_by_name(const std::set<std::wstring>& all_process, std::set<std::wstring>& exist_process, std::set<std::wstring>& not_exist_process);

SystemState::SystemState(HWND hwnd) {
    this->m_hwnd = hwnd;
    this->m_is_unstable = true;
    this->m_status = Status::NotInstall;
    this->m_on_status_changed = NULL;
    this->m_userdata = NULL;
    this->m_timerId = 0;
    this->m_last_query_tick_count = 0;
}

SystemState::~SystemState() {
    if (this->m_timerId != 0) {
        KillTimer(NULL, this->m_timerId);
    }
}

void SystemState::scan(void (*on_status_changed_callback)(Status new_status, Status old_status, void* userdata), void* userdata) {
    this->m_on_status_changed = on_status_changed_callback;
    this->m_userdata = userdata;
    this->m_is_unstable = true;
    this->m_status = Status::NotInstall;
    this->m_last_query_tick_count = 0;

    if (this->m_timerId != 0) {
        KillTimer(NULL, this->m_timerId);
        this->m_timerId = 0;
    }

    this->m_timerId = SetTimer(this->m_hwnd, (UINT_PTR)this, 500, SystemState::timer_proc);
}

void SystemState::stop() {
    if (this->m_timerId != 0) {
        KillTimer(NULL, this->m_timerId);
    }
}

SystemState::Status SystemState::status() {
	return this->m_status;
}


void CALLBACK SystemState::timer_proc(HWND, UINT, UINT_PTR idEvent, DWORD) {
    SystemState* self = (SystemState*)idEvent;

    DWORD interval = 3000;
    if (self->m_is_unstable) {
        interval = 1000;
    }

    DWORD tick_count = GetTickCount();
    if (tick_count - self->m_last_query_tick_count < interval) {
        return;
    }

    self->m_last_query_tick_count = tick_count;
    query_buckyos_status(SystemState::on_status_query_callback, (void*)self);
}

void SystemState::on_status_query_callback(bool is_success, SystemState::Status status, void* userdata) {
    SystemState* self = (SystemState*)userdata;

    SystemState::Status old_status = self->m_status;
    SystemState::Status new_status = status;
    if (is_success) {
        self->m_is_unstable = false;
    }
    else {
        self->m_is_unstable = true;
        new_status = (SystemState::Status)((old_status + 1) % 4);
    }

    if (new_status != old_status) {
        self->m_status = new_status;
        self->m_on_status_changed(new_status, old_status, self->m_userdata);
    }
}

void query_buckyos_status(void (*callback)(bool is_success, SystemState::Status status, void* userdata), void* userdata) {
    LPCWSTR all_process[] = { L"node_daemon", L"scheduler", L"verify_hub", L"system_config", L"cyfs_gateway" };
    std::set<std::wstring> all_process_set;
    for (int i = 0; i < sizeof(all_process) / sizeof(all_process[0]); i++) {
        all_process_set.insert(all_process[i]);
    }

    std::set<std::wstring> exist_process_set;
    std::set<std::wstring> not_exist_process_set;
    if (!find_process_by_name(all_process_set, exist_process_set, not_exist_process_set)) {
        callback(false, SystemState::Status::Failed, userdata);
        return;
    }

    if (not_exist_process_set.size() > 0) {
        if (exist_process_set.size() > 0) {
            callback(true, SystemState::Status::Failed, userdata);
        }
        else {
            // TODO: maybe not install
            callback(true, SystemState::Status::Stopped, userdata);
        }
    }
    else {
        callback(true, SystemState::Status::Running, userdata);
    }
}

bool find_process_by_name(const std::set<std::wstring>& all_process, std::set<std::wstring>& exist_process, std::set<std::wstring>& not_exist_process) {
    HANDLE hProcessSnap;
    PROCESSENTRY32 pe32;

    exist_process.clear();
    not_exist_process.clear();

    if (all_process.size() == 0) {
        return true;
    }

    std::set<std::wstring> all_process_with_low_case;
    for (std::set<std::wstring>::iterator it = all_process.begin(); it != all_process.end(); ++it) {
        std::wstring name;
        for (std::wstring::const_iterator name_it = it->begin(); name_it != it->end(); ++name_it) {
            name.push_back(std::tolower(*name_it));
        }
        all_process_with_low_case.insert(name);
    }

    // Take a snapshot of all processes in the system
    hProcessSnap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
    if (hProcessSnap == INVALID_HANDLE_VALUE) {
        return false;
    }

    pe32.dwSize = sizeof(PROCESSENTRY32);

    // Retrieve information about the first process
    if (!Process32First(hProcessSnap, &pe32)) {
        CloseHandle(hProcessSnap);
        return false;
    }

    // Loop through all processes
    do {
        std::wstring name;
        for (LPWSTR ch = pe32.szExeFile; *ch != L'\0'; ch++) {
            name.push_back(std::tolower(*ch));
        }
        if (all_process_with_low_case.find(name) != all_process_with_low_case.end()) {
            exist_process.insert(name);

            if (exist_process.size() == all_process_with_low_case.size()) {
                break;
            }
        }
    } while (Process32Next(hProcessSnap, &pe32));

    CloseHandle(hProcessSnap);

    for (std::set<std::wstring>::iterator it = all_process_with_low_case.begin(); it != all_process_with_low_case.end(); ++it) {
        if (exist_process.find(*it) == exist_process.end()) {
            not_exist_process.insert(*it);
        }
    }
    return true;
}
