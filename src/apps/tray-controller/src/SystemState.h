#ifndef _SYSTEM_STATE_H_
#define _SYSTEM_STATE_H_

#include <windows.h>

class SystemState
{
public:
    enum Status {
        Running,
        Stopped,
        NotInstall,
        Failed,
    };

public:
	SystemState(HWND hwnd);
	~SystemState();

	void scan(void (*on_status_changed_callback)(Status new_status, Status old_status, void* userdata), void* userdata);
    void stop();

    Status status();

private:
    static void CALLBACK timer_proc(HWND, UINT, UINT_PTR idEvent, DWORD);
    static void on_status_query_callback(bool is_success, Status status, void* userdata);

private:
    HWND m_hwnd;
    bool m_is_unstable;
    Status m_status;
    void (*m_on_status_changed)(Status new_status, Status old_status, void* userdata);
    void* m_userdata;
    UINT_PTR m_timerId;
    DWORD m_last_query_tick_count;
};

#endif