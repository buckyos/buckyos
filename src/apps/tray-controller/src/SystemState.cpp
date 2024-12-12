#include "SystemState.h"
#include <set>
#include <string>
#include "process_kits.h"

void query_buckyos_status(void (*callback)(bool is_success, BuckyStatus status, void* userdata), void* userdata);

SystemState::SystemState(void (*on_status_changed_callback)(BuckyStatus new_status, BuckyStatus old_status, void* userdata), void* userdata, HWND hwnd) {
    this->m_hwnd = hwnd;
    this->m_is_unstable = true;
    this->m_status = BuckyStatus::NotInstall;
    this->m_on_status_changed = on_status_changed_callback;
    this->m_userdata = userdata;
    this->m_timerId = 0;
    this->m_last_query_tick_count = 0;
}

SystemState::~SystemState() {
    if (this->m_timerId != 0) {
        KillTimer(NULL, this->m_timerId);
    }
}

void SystemState::scan() {
    this->m_is_unstable = true;
    this->m_status = BuckyStatus::NotInstall;
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

BuckyStatus SystemState::status() {
	return this->m_status;
}


void CALLBACK SystemState::timer_proc(HWND, UINT, UINT_PTR idEvent, DWORD) {
    SystemState* self = (SystemState*)idEvent;

    DWORD interval = 3000;
    if (self->m_is_unstable) {
        interval = 1000;
    }

    ULONGLONG tick_count = GetTickCount64();
    if (tick_count - self->m_last_query_tick_count < interval) {
        return;
    }

    self->m_last_query_tick_count = tick_count;
    query_buckyos_status(SystemState::on_status_query_callback, (void*)self);
}

void SystemState::on_status_query_callback(bool is_success, BuckyStatus status, void* userdata) {
    SystemState* self = (SystemState*)userdata;

    BuckyStatus old_status = self->m_status;
    BuckyStatus new_status = status;
    if (is_success) {
        self->m_is_unstable = false;
    }
    else {
        self->m_is_unstable = true;
        new_status = (BuckyStatus)((old_status + 1) % 4);
    }

    if (new_status != old_status) {
        self->m_status = new_status;
        self->m_on_status_changed(new_status, old_status, self->m_userdata);
    }
}

void query_buckyos_status(void (*callback)(bool is_success, BuckyStatus status, void* userdata), void* userdata) {
    std::set<std::wstring> all_process_set;
    for (int i = 0; i < sizeof(buckyos_process) / sizeof(buckyos_process[0]); i++) {
        all_process_set.insert(buckyos_process[i]);
    }

    std::map<std::wstring, DWORD> exist_process_map;
    std::set<std::wstring> not_exist_process_set;
    if (!find_process_by_name(all_process_set, exist_process_map, not_exist_process_set)) {
        callback(false, BuckyStatus::Failed, userdata);
        return;
    }

    if (exist_process_map.size() > 0) {
        if (exist_process_map.size() > 0) {
            callback(true, BuckyStatus::Failed, userdata);
        }
        else {
            // TODO: maybe not install
            callback(true, BuckyStatus::Stopped, userdata);
        }
    }
    else {
        callback(true, BuckyStatus::Running, userdata);
    }
}
