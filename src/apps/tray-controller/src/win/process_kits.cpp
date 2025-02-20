#include "process_kits.h"
#include <tlhelp32.h>

LPCWSTR buckyos_process[5] = { L"node_daemon", L"scheduler", L"verify_hub", L"system_config", L"cyfs_gateway" };

bool find_process_by_name(const std::set<std::wstring>& all_process, std::map<std::wstring, DWORD>& exist_process, std::set<std::wstring>& not_exist_process) {
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
            exist_process.insert_or_assign(name, pe32.th32ProcessID);

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

BOOL kill_process_by_id(DWORD processId) {
    HANDLE hProcess = OpenProcess(PROCESS_TERMINATE, FALSE, processId);
    if (hProcess == NULL) {
        printf("Failed to open process with ID %lu.\n", processId);
        return FALSE;
    }

    BOOL result = TerminateProcess(hProcess, 0);
    if (!result) {
        printf("Failed to terminate process with ID %lu.\n", processId);
    } else {
        printf("Successfully terminated process with ID %lu.\n", processId);
    }

    CloseHandle(hProcess);
    return result;
}

void execute_cmd_hidden(LPCWSTR command) {
    STARTUPINFO si;
    PROCESS_INFORMATION pi;

    // 初始化STARTUPINFO结构体
    ZeroMemory(&si, sizeof(si));
    si.cb = sizeof(si);
    si.dwFlags = STARTF_USESHOWWINDOW;
    si.wShowWindow = SW_HIDE; // 隐藏窗口

    // 初始化PROCESS_INFORMATION结构体
    ZeroMemory(&pi, sizeof(pi));

    // 创建进程
    if (!CreateProcessW(
            NULL,
            (LPWSTR)command,
            NULL,
            NULL,
            FALSE,
            0,
            NULL,
            NULL,
            &si,
            &pi
        )) {
    } else {
        CloseHandle(pi.hProcess);
        CloseHandle(pi.hThread);
    }
}
