#include "capture_bridge.h"

#include <windows.h>
#include <shellscalingapi.h>
#include <gdiplus.h>
#include <objidl.h>

#include <atomic>
#include <filesystem>
#include <stdexcept>
#include <string>

namespace {

std::atomic<uint64_t> capture_id{0};

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
