#ifndef _SYSTEM_STATE_H_
#define _SYSTEM_STATE_H_

#include "ffi_extern.h"
#include <windows.h>

class SystemState
{
public:
	SystemState(void (*on_status_changed_callback)(BuckyStatus new_status, BuckyStatus old_status, void* userdata), void* userdata, HWND hwnd);
	~SystemState();

	void scan();
    void stop();

    BuckyStatus status();

private:
    static void CALLBACK timer_proc(HWND, UINT, UINT_PTR idEvent, DWORD);
    static void on_status_query_callback(bool is_success, BuckyStatus status, void* userdata);

private:
    HWND m_hwnd;
    bool m_is_unstable;
    BuckyStatus m_status;
    void (*m_on_status_changed)(BuckyStatus new_status, BuckyStatus old_status, void* userdata);
    void* m_userdata;
    UINT_PTR m_timerId;
    ULONGLONG m_last_query_tick_count;
};

#endif