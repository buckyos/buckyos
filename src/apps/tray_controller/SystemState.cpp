#include "SystemState.h"

void query_buckyos_status(void (*callback)(bool is_success, SystemState::Status status, void* userdata), void* userdata);

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
    callback(false, SystemState::Status::Running, userdata);
}