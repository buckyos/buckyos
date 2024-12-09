#include <windows.h>

extern "C" void entry();

int WINAPI wWinMain(HINSTANCE hInstance, HINSTANCE, LPWSTR, int nShowCmd) {
	entry();
	return 0;
}
