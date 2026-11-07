#include "capture_bridge.h"

#include <windows.h>
#include <shellscalingapi.h>
#include <gdiplus.h>
#include <dwmapi.h>
#include <objidl.h>

#include <algorithm>
#include <atomic>
#include <cmath>
#include <filesystem>
#include <stdexcept>
#include <string>

namespace {

std::atomic<uint64_t> capture_id{0};
std::vector<HWND> long_guide_windows;
LRESULT CALLBACK GuideWindowProc(HWND window, UINT message, WPARAM wparam, LPARAM lparam) {
  if (message == WM_NCHITTEST) return HTTRANSPARENT;
  if (message == WM_MOUSEACTIVATE) return MA_NOACTIVATE;
  return DefWindowProcW(window, message, wparam, lparam);
}


std::string Utf8(const std::wstring& value) {
  const int length = WideCharToMultiByte(CP_UTF8, 0, value.c_str(), -1,
                                         nullptr, 0, nullptr, nullptr);
  std::string result(length, '\0');
  WideCharToMultiByte(CP_UTF8, 0, value.c_str(), -1, result.data(), length,
                      nullptr, nullptr);
  result.pop_back();
  return result;
}

struct CaptureContext {
  flutter::EncodableList displays;
  CLSID png_encoder = {0x557cf406, 0x1a04, 0x11d3,
                                {0x9a, 0x73, 0x00, 0x00, 0xf8, 0x1e, 0xf3, 0x2e}};
};

BOOL CALLBACK CaptureMonitor(HMONITOR monitor, HDC, LPRECT, LPARAM data) {
  auto* context = reinterpret_cast<CaptureContext*>(data);
  MONITORINFOEXW info{};
  info.cbSize = sizeof(info);
  if (!GetMonitorInfoW(monitor, &info)) return TRUE;

  const int width = info.rcMonitor.right - info.rcMonitor.left;
  const int height = info.rcMonitor.bottom - info.rcMonitor.top;
  if (width <= 0 || height <= 0) return TRUE;

  HDC screen = GetDC(nullptr);
  HDC memory = CreateCompatibleDC(screen);
  HBITMAP bitmap = CreateCompatibleBitmap(screen, width, height);
  HGDIOBJ previous = SelectObject(memory, bitmap);
  const BOOL copied = BitBlt(memory, 0, 0, width, height, screen,
                            info.rcMonitor.left, info.rcMonitor.top,
                            SRCCOPY | CAPTUREBLT);
  SelectObject(memory, previous);
  DeleteDC(memory);
  ReleaseDC(nullptr, screen);
  if (!copied) {
    DeleteObject(bitmap);
    return TRUE;
  }

  const auto directory = std::filesystem::temp_directory_path() / L"snappin-flutter";
  std::filesystem::create_directories(directory);
  const auto path = directory /
      (L"capture-" + std::to_wstring(GetCurrentProcessId()) + L"-" +
       std::to_wstring(++capture_id) + L".png");
  Gdiplus::Status status;
  {
    Gdiplus::Bitmap image(bitmap, nullptr);
    status = image.Save(path.c_str(), &context->png_encoder);
  }
  DeleteObject(bitmap);
  if (status != Gdiplus::Ok) return TRUE;

  UINT dpi_x = 96;
  UINT dpi_y = 96;
  if (GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &dpi_x, &dpi_y) != S_OK) {
    dpi_x = 96;
  }
  const double scale = static_cast<double>(dpi_x) / 96.0;
  context->displays.push_back(flutter::EncodableMap{
      {flutter::EncodableValue("path"), flutter::EncodableValue(Utf8(path.wstring()))},
      {flutter::EncodableValue("x"), flutter::EncodableValue(static_cast<double>(info.rcMonitor.left) / scale)},
      {flutter::EncodableValue("y"), flutter::EncodableValue(static_cast<double>(info.rcMonitor.top) / scale)},
      {flutter::EncodableValue("width"), flutter::EncodableValue(static_cast<double>(width) / scale)},
      {flutter::EncodableValue("height"), flutter::EncodableValue(static_cast<double>(height) / scale)},
      {flutter::EncodableValue("pixelWidth"), flutter::EncodableValue(width)},
      {flutter::EncodableValue("pixelHeight"), flutter::EncodableValue(height)},
      {flutter::EncodableValue("pixelX"), flutter::EncodableValue(info.rcMonitor.left)},
      {flutter::EncodableValue("pixelY"), flutter::EncodableValue(info.rcMonitor.top)},
      {flutter::EncodableValue("scale"), flutter::EncodableValue(scale)},
  });
  return TRUE;
}

}  // namespace

flutter::EncodableList CaptureDisplays() {
  Gdiplus::GdiplusStartupInput input;
  ULONG_PTR token = 0;
  if (Gdiplus::GdiplusStartup(&token, &input, nullptr) != Gdiplus::Ok) {
    throw std::runtime_error("GDI+ initialization failed");
  }
  CaptureContext context;
  EnumDisplayMonitors(nullptr, nullptr, CaptureMonitor,
                      reinterpret_cast<LPARAM>(&context));
  Gdiplus::GdiplusShutdown(token);
  if (context.displays.empty()) {
    throw std::runtime_error("No displays were captured");
  }
  return context.displays;
}

std::vector<uint8_t> CaptureRegion(int x, int y, int width, int height) {
  if (width <= 0 || height <= 0 || width > 16384 || height > 16384) {
    throw std::runtime_error("Invalid capture region");
  }
  Gdiplus::GdiplusStartupInput input;
  ULONG_PTR token = 0;
  if (Gdiplus::GdiplusStartup(&token, &input, nullptr) != Gdiplus::Ok) {
    throw std::runtime_error("GDI+ initialization failed");
  }
  HDC screen = GetDC(nullptr);
  HDC memory = CreateCompatibleDC(screen);
  HBITMAP bitmap = CreateCompatibleBitmap(screen, width, height);
  HGDIOBJ previous = SelectObject(memory, bitmap);
  const BOOL copied = BitBlt(memory, 0, 0, width, height, screen, x, y,
                            SRCCOPY | CAPTUREBLT);
  SelectObject(memory, previous);
  DeleteDC(memory);
  ReleaseDC(nullptr, screen);
  if (!copied) {
    DeleteObject(bitmap);
    Gdiplus::GdiplusShutdown(token);
    throw std::runtime_error("BitBlt failed");
  }

  IStream* stream = nullptr;
  if (CreateStreamOnHGlobal(nullptr, TRUE, &stream) != S_OK) {
    DeleteObject(bitmap);
    Gdiplus::GdiplusShutdown(token);
    throw std::runtime_error("CreateStreamOnHGlobal failed");
  }
  const CLSID encoder = {0x557cf406, 0x1a04, 0x11d3,
                         {0x9a, 0x73, 0x00, 0x00, 0xf8, 0x1e, 0xf3, 0x2e}};
  Gdiplus::Status status;
  {
    Gdiplus::Bitmap image(bitmap, nullptr);
    status = image.Save(stream, &encoder);
  }
  DeleteObject(bitmap);
  if (status != Gdiplus::Ok) {
    stream->Release();
    Gdiplus::GdiplusShutdown(token);
    throw std::runtime_error("PNG encoding failed");
  }
  STATSTG stat{};
  if (stream->Stat(&stat, STATFLAG_NONAME) != S_OK) {
    stream->Release();
    Gdiplus::GdiplusShutdown(token);
    throw std::runtime_error("PNG stream stat failed");
  }
  HGLOBAL global = nullptr;
  if (GetHGlobalFromStream(stream, &global) != S_OK || !global) {
    stream->Release();
    Gdiplus::GdiplusShutdown(token);
    throw std::runtime_error("PNG stream buffer unavailable");
  }
  const auto* data = static_cast<const uint8_t*>(GlobalLock(global));
  if (!data) {
    stream->Release();
    Gdiplus::GdiplusShutdown(token);
    throw std::runtime_error("PNG stream lock failed");
  }
  const size_t length = static_cast<size_t>(stat.cbSize.QuadPart);
  std::vector<uint8_t> bytes(data, data + length);
  GlobalUnlock(global);
  stream->Release();
  Gdiplus::GdiplusShutdown(token);
  return bytes;
}

void HideLongCaptureGuide() {
  for (auto window : long_guide_windows) {
    if (IsWindow(window)) DestroyWindow(window);
  }
  long_guide_windows.clear();
}

flutter::EncodableMap ShowLongCaptureGuide(const flutter::EncodableMap& args) {
  HideLongCaptureGuide();
  const auto integer = [&](const char* key) {
    const auto& value = args.at(flutter::EncodableValue(key));
    return std::holds_alternative<int64_t>(value)
        ? static_cast<int>(std::get<int64_t>(value)) : std::get<int32_t>(value);
  };
  const int x = integer("x") - 4, y = integer("y") - 4;
  const int width = integer("width") + 8, height = integer("height") + 8;
  if (width <= 8 || height <= 8) throw std::runtime_error("Invalid long capture guide bounds");
  double scale = 1;
  const auto logical = args.find(flutter::EncodableValue("logicalWidth"));
  if (logical != args.end() && std::holds_alternative<double>(logical->second) &&
      std::get<double>(logical->second) > 0) {
    scale = width / std::get<double>(logical->second);
  }
  const int t = std::max(1, static_cast<int>(std::round(2 * scale)));
  static const wchar_t* class_name = L"SnapPinLongCaptureGuide";
  static const ATOM atom = [] {
    WNDCLASSW cls{};
    cls.lpfnWndProc = GuideWindowProc;
    cls.hInstance = GetModuleHandleW(nullptr);
    cls.hbrBackground = CreateSolidBrush(RGB(22, 136, 240));
    cls.lpszClassName = class_name;
    return RegisterClassW(&cls);
  }();
  if (!atom) throw std::runtime_error("Cannot register long capture guide class");
  const RECT strips[] = {{x, y, x+width, y+t}, {x, y+height-t, x+width, y+height},
                         {x, y, x+t, y+height}, {x+width-t, y, x+width, y+height}};
  bool excluded = true;
  try {
    for (const auto& r : strips) {
      HWND window = CreateWindowExW(WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE |
          WS_EX_LAYERED | WS_EX_TRANSPARENT, class_name, L"SnapPin long capture guide", WS_POPUP,
          r.left, r.top, r.right-r.left, r.bottom-r.top, nullptr, nullptr, GetModuleHandleW(nullptr), nullptr);
      if (!window) throw std::runtime_error("Cannot create long capture guide strip");
      long_guide_windows.push_back(window);
      if (!SetLayeredWindowAttributes(window, 0, 255, LWA_ALPHA)) {
        throw std::runtime_error("Cannot configure transparent-input guide");
      }
      excluded = (SetWindowDisplayAffinity(window, 0x11) != 0) && excluded;
      ShowWindow(window, SW_SHOWNOACTIVATE);
      UpdateWindow(window);
    }
    DwmFlush();
    return {{flutter::EncodableValue("visible"), flutter::EncodableValue(true)},
            {flutter::EncodableValue("segments"), flutter::EncodableValue(4)},
            {flutter::EncodableValue("mouseTransparent"), flutter::EncodableValue(true)},
            {flutter::EncodableValue("excluded"), flutter::EncodableValue(excluded)}};
  } catch (...) {
    HideLongCaptureGuide();
    throw;
  }
}

// Own test windows only: verify the production GDI path after hiding a reused
// capture surface. Never send input or change the user's browser window.
flutter::EncodableList CaptureRepeatedRegionSmoke() {
  wchar_t mode[32]{};
  if (!GetEnvironmentVariableW(L"SNAPPIN_RUNTIME_SMOKE", mode, 32)) {
    throw std::runtime_error("Capture regression requires runtime smoke mode");
  }
  const auto instance = GetModuleHandleW(nullptr);
  HBRUSH brush = CreateSolidBrush(RGB(37, 157, 211));
  WNDCLASSW fixture_class{};
  fixture_class.lpfnWndProc = DefWindowProcW;
  fixture_class.hInstance = instance;
  fixture_class.hbrBackground = brush;
  fixture_class.lpszClassName = L"SnapPinRepeatedCaptureFixture";
  const auto fixture_atom = RegisterClassW(&fixture_class);
  WNDCLASSW cover_class = fixture_class;
  cover_class.hbrBackground = static_cast<HBRUSH>(GetStockObject(BLACK_BRUSH));
  cover_class.lpszClassName = L"SnapPinRepeatedCaptureSurface";
  const auto cover_atom = RegisterClassW(&cover_class);
  RECT work{};
  SystemParametersInfoW(SPI_GETWORKAREA, 0, &work, 0);
  const int x = work.left + 40, y = work.top + 40;
  HWND fixture = nullptr, cover = nullptr;
  const auto cleanup = [&]() {
    HideLongCaptureGuide();
    if (cover) DestroyWindow(cover);
    if (fixture) DestroyWindow(fixture);
    if (cover_atom) UnregisterClassW(cover_class.lpszClassName, instance);
    if (fixture_atom) UnregisterClassW(fixture_class.lpszClassName, instance);
    DeleteObject(brush);
  };
  try {
    if (!fixture_atom || !cover_atom) throw std::runtime_error("Cannot register capture fixtures");
    fixture = CreateWindowExW(WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
        fixture_class.lpszClassName, L"SnapPin capture regression", WS_POPUP,
        x, y, 260, 320, nullptr, nullptr, instance, nullptr);
    cover = CreateWindowExW(WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW |
        WS_EX_LAYERED | WS_EX_TRANSPARENT, cover_class.lpszClassName,
        L"SnapPin capture surface regression", WS_POPUP,
        x, y, 260, 320, nullptr, nullptr, instance, nullptr);
    if (!fixture || !cover) throw std::runtime_error("Cannot create capture fixtures");
    SetLayeredWindowAttributes(cover, 0, 255, LWA_ALPHA);
    SetWindowDisplayAffinity(cover, 0x11);
    ShowWindow(fixture, SW_SHOWNOACTIVATE);
    UpdateWindow(fixture);
    flutter::EncodableList frames;
    for (int cycle = 0; cycle < 2; ++cycle) {
      ShowWindow(cover, SW_SHOWNOACTIVATE);
      UpdateWindow(cover);
      DwmFlush();
      ShowWindow(cover, SW_HIDE);
      DwmFlush();
      const auto guide_status = ShowLongCaptureGuide({
          {flutter::EncodableValue("x"), flutter::EncodableValue(x + 34)},
          {flutter::EncodableValue("y"), flutter::EncodableValue(y + 34)},
          {flutter::EncodableValue("width"), flutter::EncodableValue(192)},
          {flutter::EncodableValue("height"), flutter::EncodableValue(252)}});
      if (!std::get<bool>(guide_status.at(flutter::EncodableValue("excluded")))) {
        throw std::runtime_error("Guide capture exclusion unavailable in smoke test");
      }
      for (auto guide : long_guide_windows) {
        if (!IsWindowVisible(guide) || GetActiveWindow() == guide ||
            !(GetWindowLongPtr(guide, GWL_EXSTYLE) & WS_EX_TRANSPARENT)) {
          throw std::runtime_error("Guide is invisible, active or intercepts input");
        }
      }
      if (WindowFromPoint(POINT{x + 31, y + 160}) != fixture) {
        throw std::runtime_error("Guide does not pass pointer hit testing to document");
      }
      Sleep(30);
      frames.push_back(flutter::EncodableValue(CaptureRegion(x, y, 260, 320)));
      HideLongCaptureGuide();
      if (!long_guide_windows.empty()) throw std::runtime_error("Guide cleanup failed");
    }
    cleanup();
    return frames;
  } catch (...) {
    cleanup();
    throw;
  }
}
