#include <flutter/dart_project.h>
#include <flutter/flutter_view_controller.h>
#include <windows.h>
#include <cstdio>
#include <string>

#include "flutter_window.h"
#include "utils.h"

void TraceStartup(const char* stage) {
  static const ULONGLONG started = GetTickCount64();
  wchar_t app_data[MAX_PATH]{};
  if (!GetEnvironmentVariableW(L"APPDATA", app_data, MAX_PATH)) return;
  std::wstring directory = std::wstring(app_data) + L"\\com.snappin.flutter";
  CreateDirectoryW(directory.c_str(), nullptr);
  directory += L"\\logs";
  CreateDirectoryW(directory.c_str(), nullptr);
  SYSTEMTIME now{};
  GetLocalTime(&now);
  wchar_t filename[64]{};
  swprintf_s(filename, L"\\snappin-%04u-%02u-%02u.log", now.wYear,
             now.wMonth, now.wDay);
  const std::wstring path = directory + filename;
  HANDLE file = CreateFileW(path.c_str(), FILE_APPEND_DATA,
                            FILE_SHARE_READ | FILE_SHARE_WRITE, nullptr,
                            OPEN_ALWAYS, FILE_ATTRIBUTE_NORMAL, nullptr);
  if (file == INVALID_HANDLE_VALUE) return;
  char line[200]{};
  const int length = sprintf_s(
      line, "%04u-%02u-%02uT%02u:%02u:%02u.%03u [native-startup] pid=%lu %s elapsed=%llums\r\n",
      now.wYear, now.wMonth, now.wDay, now.wHour, now.wMinute, now.wSecond,
      now.wMilliseconds, GetCurrentProcessId(), stage,
      GetTickCount64() - started);
  DWORD written = 0;
  WriteFile(file, line, length, &written, nullptr);
  CloseHandle(file);
}

int APIENTRY wWinMain(_In_ HINSTANCE instance, _In_opt_ HINSTANCE prev,
                      _In_ wchar_t *command_line, _In_ int show_command) {
  TraceStartup("entry");
  wchar_t smoke_mode[32]{};
  const bool runtime_smoke = GetEnvironmentVariableW(L"SNAPPIN_RUNTIME_SMOKE", smoke_mode, 32) > 0;
  const auto mutex_name = runtime_smoke
      ? L"Local\\SnapPinFlutterRuntimeSmoke" : L"Local\\SnapPinFlutterSingleInstance";
  HANDLE instance_mutex = CreateMutexW(nullptr, TRUE, mutex_name);
  if (!instance_mutex) {
    return EXIT_FAILURE;
  }
  if (GetLastError() == ERROR_ALREADY_EXISTS) {
    CloseHandle(instance_mutex);
    return EXIT_SUCCESS;
  }

  // Attach to console when present (e.g., 'flutter run') or create a
  // new console when running with a debugger.
  if (!::AttachConsole(ATTACH_PARENT_PROCESS) && ::IsDebuggerPresent()) {
    CreateAndAttachConsole();
  }

  // Initialize COM, so that it is available for use in the library and/or
  // plugins.
  ::CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
  TraceStartup("com-ready");

  flutter::DartProject project(L"data");
  // On Windows this reduced engine startup from roughly 4 seconds to 0.6 seconds.
  project.set_impeller_switch(flutter::ImpellerSwitch::Disabled);
  TraceStartup("dart-project");

  std::vector<std::string> command_line_arguments =
      GetCommandLineArguments();

  project.set_dart_entrypoint_arguments(std::move(command_line_arguments));
  TraceStartup("arguments-ready");

  FlutterWindow window(project);
  Win32Window::Point origin(10, 10);
  Win32Window::Size size(64, 64);
  if (!window.Create(L"snappin_flutter", origin, size)) {
    ReleaseMutex(instance_mutex);
    CloseHandle(instance_mutex);
    return EXIT_FAILURE;
  }
  TraceStartup("window-created");
  window.SetQuitOnClose(true);

  ::MSG msg;
  while (::GetMessage(&msg, nullptr, 0, 0)) {
    ::TranslateMessage(&msg);
    ::DispatchMessage(&msg);
  }

  ::CoUninitialize();
  ReleaseMutex(instance_mutex);
  CloseHandle(instance_mutex);
  return EXIT_SUCCESS;
}
