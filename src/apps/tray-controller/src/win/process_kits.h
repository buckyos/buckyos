#pragma once
#include <set>
#include <map>
#include <string>
#include <windows.h>

bool find_process_by_name(const std::set<std::wstring>& all_process, std::map<std::wstring, DWORD>& exist_process, std::set<std::wstring>& not_exist_process);
BOOL kill_process_by_id(DWORD processId);
void execute_cmd_hidden(LPCWSTR command);

extern LPCWSTR buckyos_process[5];