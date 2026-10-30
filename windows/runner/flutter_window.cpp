#include "flutter_window.h"

#include <optional>
#include <algorithm>
#include <dwmapi.h>
#include <commctrl.h>
#include <cwchar>

#include "flutter/generated_plugin_registrant.h"
#include "desktop_multi_window/desktop_multi_window_plugin.h"
#include <flutter/method_channel.h>
#include <flutter/standard_method_codec.h>

#include "capture_bridge.h"
#include "ocr_bridge.h"

void TraceStartup(const char* stage);

namespace {
BOOL CALLBACK HideInputToolbar(HWND window, LPARAM context) {
  if (!IsWindowVisible(window)) return TRUE;
  wchar_t class_name[64]{};
  if (!GetClassNameW(window, class_name, 64) ||
      std::wcscmp(class_name, L"SoPY_Status") != 0) return TRUE;
  if (ShowWindow(window, SW_HIDE)) {
    reinterpret_cast<std::vector<HWND>*>(context)->push_back(window);
  }
  return TRUE;
}

void RestoreInputToolbars(std::vector<HWND>& windows) {
  for (HWND window : windows) {
    if (IsWindow(window)) ShowWindow(window, SW_SHOWNOACTIVATE);
  }
  windows.clear();
}

int Number(const flutter::EncodableMap& args, const char* key) {
  const auto& value = args.at(flutter::EncodableValue(key));
  return std::holds_alternative<int64_t>(value)
      ? static_cast<int>(std::get<int64_t>(value))
      : std::get<int32_t>(value);
}

RECT VisibleRect(HWND window) {
  RECT rect{};
  if (DwmGetWindowAttribute(window, DWMWA_EXTENDED_FRAME_BOUNDS,
                            &rect, sizeof(rect)) != S_OK ||
      rect.right <= rect.left || rect.bottom <= rect.top) {
    GetWindowRect(window, &rect);
  }
  return rect;
}

struct SnapSearch { POINT point; HWND overlay; HWND found = nullptr; };
BOOL CALLBACK FindSnapWindow(HWND window, LPARAM context) {
  auto& search = *reinterpret_cast<SnapSearch*>(context);
  if (window == search.overlay || !IsWindowVisible(window)) return TRUE;
  const auto style = GetWindowLongPtr(window, GWL_EXSTYLE);
  if (style & (WS_EX_TRANSPARENT | WS_EX_NOACTIVATE)) return TRUE;
  BOOL cloaked = FALSE;
  if (DwmGetWindowAttribute(window, DWMWA_CLOAKED, &cloaked,
                            sizeof(cloaked)) == S_OK && cloaked) return TRUE;
  const RECT rect = VisibleRect(window);
  if (rect.right - rect.left < 12 || rect.bottom - rect.top < 12 ||
      !PtInRect(&rect, search.point)) return TRUE;
  search.found = window;
  return FALSE;
}

struct RegionSearch { POINT point; RECT best; int64_t area; };
BOOL CALLBACK FindSnapRegion(HWND window, LPARAM context) {
  auto& search = *reinterpret_cast<RegionSearch*>(context);
  if (!IsWindowVisible(window)) return TRUE;
  RECT rect{};
  GetWindowRect(window, &rect);
  const int width = rect.right - rect.left;
  const int height = rect.bottom - rect.top;
  const int64_t area = static_cast<int64_t>(width) * height;
  if (width >= 120 && height >= 32 && area >= 30000 &&
      area < search.area * 95 / 100 && PtInRect(&rect, search.point)) {
    search.best = rect;
    search.area = area;
  }
  return TRUE;
}

flutter::EncodableValue SnapAt(const flutter::EncodableMap& args, HWND overlay) {
  SnapSearch search{{Number(args, "pixelX"), Number(args, "pixelY")}, overlay};
  EnumWindows(FindSnapWindow, reinterpret_cast<LPARAM>(&search));
  if (!search.found) return flutter::EncodableValue();
  RECT rect = VisibleRect(search.found);
  RegionSearch region{search.point, rect,
                      static_cast<int64_t>(rect.right - rect.left) *
                          (rect.bottom - rect.top)};
  EnumChildWindows(search.found, FindSnapRegion,
                   reinterpret_cast<LPARAM>(&region));
  rect = region.best;
  const auto& scale_value = args.at(flutter::EncodableValue("scale"));
  const double scale = std::get<double>(scale_value);
  const int origin_x = Number(args, "displayPixelX");
  const int origin_y = Number(args, "displayPixelY");
  const double left = std::max(0.0, (rect.left - origin_x) / scale);
  const double top = std::max(0.0, (rect.top - origin_y) / scale);
  const double right = std::min(static_cast<double>(Number(args, "displayWidth")),
                                (rect.right - origin_x) / scale);
  const double bottom = std::min(static_cast<double>(Number(args, "displayHeight")),
                                 (rect.bottom - origin_y) / scale);
  if (right <= left || bottom <= top) return flutter::EncodableValue();
  return flutter::EncodableValue(flutter::EncodableMap{
      {flutter::EncodableValue("x"), flutter::EncodableValue(left)},
      {flutter::EncodableValue("y"), flutter::EncodableValue(top)},
      {flutter::EncodableValue("width"), flutter::EncodableValue(right - left)},
      {flutter::EncodableValue("height"), flutter::EncodableValue(bottom - top)}});
}
}  // namespace

FlutterWindow::FlutterWindow(const flutter::DartProject& project)
    : project_(project) {}

FlutterWindow::~FlutterWindow() {
  RestoreInputToolbars(hidden_input_toolbars_);
}

bool FlutterWindow::OnCreate() {
  TraceStartup("on-create-entry");
  if (!Win32Window::OnCreate()) {
    return false;
  }
  TraceStartup("win32-window-created");

  // The root window hosts only the 56 px floating ball. An overlapped window
  // has a system minimum width even when Flutter hides its title bar.
  SetWindowLongPtr(GetHandle(), GWL_STYLE, WS_POPUP | WS_CLIPCHILDREN);
  SetWindowPos(GetHandle(), nullptr, 0, 0, 0, 0,
               SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED);

  RECT frame = GetClientArea();

  // The size here must match the window dimensions to avoid unnecessary surface
  // creation / destruction in the startup path.
  flutter_controller_ = std::make_unique<flutter::FlutterViewController>(
      frame.right - frame.left, frame.bottom - frame.top, project_);
  TraceStartup("flutter-controller-created");
  // Ensure that basic setup of the controller was successful.
  if (!flutter_controller_->engine() || !flutter_controller_->view()) {
    return false;
  }
  RegisterPlugins(flutter_controller_->engine());
  TraceStartup("plugins-registered");
  capture_channel_ = std::make_unique<flutter::MethodChannel<flutter::EncodableValue>>(
      flutter_controller_->engine()->messenger(), "snappin/capture",
      &flutter::StandardMethodCodec::GetInstance());
  capture_channel_->SetMethodCallHandler(
      [this](const auto& call, auto result) {
        try {
          if (call.method_name() == "captureDisplays") {
            result->Success(flutter::EncodableValue(CaptureDisplays()));
          } else if (call.method_name() == "windowSnap") {
            result->Success(SnapAt(std::get<flutter::EncodableMap>(*call.arguments()), GetHandle()));
          } else if (call.method_name() == "captureRegion") {
            if (suppress_input_toolbar_) {
              EnumWindows(HideInputToolbar,
                          reinterpret_cast<LPARAM>(&hidden_input_toolbars_));
            }
            const auto& args = std::get<flutter::EncodableMap>(*call.arguments());
            const auto number = [&](const char* key) {
              const auto& value = args.at(flutter::EncodableValue(key));
              if (std::holds_alternative<int64_t>(value)) {
                return static_cast<int>(std::get<int64_t>(value));
              }
              return std::get<int32_t>(value);
            };
            result->Success(flutter::EncodableValue(CaptureRegion(
                number("x"), number("y"), number("width"), number("height"))));
          } else if (call.method_name() == "suppressInputToolbar") {
            suppress_input_toolbar_ = std::get<bool>(*call.arguments());
            if (suppress_input_toolbar_) {
              EnumWindows(HideInputToolbar,
                          reinterpret_cast<LPARAM>(&hidden_input_toolbars_));
            } else {
              RestoreInputToolbars(hidden_input_toolbars_);
            }
            result->Success(flutter::EncodableValue(
                static_cast<int>(hidden_input_toolbars_.size())));
          } else if (call.method_name() == "setLongEscape") {
            const bool enabled = std::get<bool>(*call.arguments());
            if (enabled && !long_escape_registered_) {
              long_escape_registered_ =
                  RegisterHotKey(GetHandle(), 0x534e, MOD_NOREPEAT, VK_ESCAPE) != 0;
              if (!long_escape_registered_) {
                result->Error("hotkey_failed", "Unable to register Escape");
                return;
              }
            } else if (!enabled && long_escape_registered_) {
              UnregisterHotKey(GetHandle(), 0x534e);
              long_escape_registered_ = false;
            }
            result->Success();
          } else if (call.method_name() == "scrollAt") {
            const auto& args = std::get<flutter::EncodableMap>(*call.arguments());
            const auto number = [&](const char* key) {
              const auto& value = args.at(flutter::EncodableValue(key));
              return std::holds_alternative<int64_t>(value)
                  ? static_cast<int>(std::get<int64_t>(value))
                  : std::get<int32_t>(value);
            };
            POINT old_cursor{};
            GetCursorPos(&old_cursor);
            SetCursorPos(number("x"), number("y"));
            INPUT input{};
            input.type = INPUT_MOUSE;
            input.mi.dwFlags = MOUSEEVENTF_WHEEL;
            input.mi.mouseData = static_cast<DWORD>(-WHEEL_DELTA * number("lines"));
            const auto sent = SendInput(1, &input, sizeof(INPUT));
            SetCursorPos(old_cursor.x, old_cursor.y);
            if (sent != 1) {
              result->Error("scroll_failed", "Unable to inject wheel input");
              return;
            }
            result->Success();
          } else {
            result->NotImplemented();
          }
        } catch (const std::exception& error) {
          result->Error("capture_failed", error.what());
        }
      });
  ocr_channel_ = std::make_unique<flutter::MethodChannel<flutter::EncodableValue>>(
      flutter_controller_->engine()->messenger(), "snappin/ocr",
      &flutter::StandardMethodCodec::GetInstance());
  ocr_channel_->SetMethodCallHandler(
      [](const auto& call, auto result) {
        if (call.method_name() != "recognizeText") {
          result->NotImplemented();
          return;
        }
        try {
          const auto& bytes = std::get<std::vector<uint8_t>>(*call.arguments());
          result->Success(flutter::EncodableValue(RecognizeText(bytes)));
        } catch (const std::exception& error) {
          result->Error("ocr_failed", error.what());
        }
      });
  DesktopMultiWindowSetWindowCreatedCallback([](void* controller) {
    auto* view = reinterpret_cast<flutter::FlutterViewController*>(controller);
    RegisterPlugins(view->engine());
    flutter::MethodChannel<flutter::EncodableValue> channel(
        view->engine()->messenger(), "snappin/capture",
        &flutter::StandardMethodCodec::GetInstance());
    const HWND overlay = GetAncestor(view->view()->GetNativeWindow(), GA_ROOT);
    channel.SetMethodCallHandler([overlay](const auto& call, auto result) {
      if (call.method_name() == "windowSnap") {
        result->Success(SnapAt(std::get<flutter::EncodableMap>(*call.arguments()), overlay));
      } else {
        result->NotImplemented();
      }
    });
  });
  TraceStartup("channels-ready");
  const HWND flutter_view = flutter_controller_->view()->GetNativeWindow();
  SetChildContent(flutter_view);
  INITCOMMONCONTROLSEX controls{sizeof(controls), ICC_WIN95_CLASSES};
  InitCommonControlsEx(&controls);
  tooltip_ = CreateWindowExW(
      WS_EX_TOPMOST, TOOLTIPS_CLASSW, nullptr,
      WS_POPUP | TTS_NOPREFIX | TTS_ALWAYSTIP,
      CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT,
      GetHandle(), nullptr, GetModuleHandle(nullptr), nullptr);
  if (tooltip_) {
    static wchar_t tooltip_text[] =
        L"\u5355\u51FB\u622A\u56FE \u00B7 \u53F3\u952E\u83DC\u5355 \u00B7 \u62D6\u52A8\u79FB\u52A8";
    TOOLINFOW tool{};
    tool.cbSize = sizeof(tool);
    tool.uFlags = TTF_IDISHWND | TTF_SUBCLASS;
    tool.hwnd = GetHandle();
    tool.uId = reinterpret_cast<UINT_PTR>(flutter_view);
    tool.lpszText = tooltip_text;
    SendMessageW(tooltip_, TTM_ADDTOOLW, 0,
                 reinterpret_cast<LPARAM>(&tool));
  }
  TraceStartup("child-content-set");

  flutter_controller_->engine()->SetNextFrameCallback([&]() {
    this->Show();
  });

  // Flutter can complete the first frame before the "show window" callback is
  // registered. The following call ensures a frame is pending to ensure the
  // window is shown. It is a no-op if the first frame hasn't completed yet.
  flutter_controller_->ForceRedraw();
  TraceStartup("redraw-requested");

  return true;
}

void FlutterWindow::OnDestroy() {
  if (tooltip_) {
    DestroyWindow(tooltip_);
    tooltip_ = nullptr;
  }
  if (long_escape_registered_) {
    UnregisterHotKey(GetHandle(), 0x534e);
    long_escape_registered_ = false;
  }
  ocr_channel_ = nullptr;
  capture_channel_ = nullptr;
  if (flutter_controller_) {
    flutter_controller_ = nullptr;
  }

  Win32Window::OnDestroy();
}

LRESULT
FlutterWindow::MessageHandler(HWND hwnd, UINT const message,
                              WPARAM const wparam,
                              LPARAM const lparam) noexcept {
  if (message == WM_HOTKEY && wparam == 0x534e && capture_channel_) {
    capture_channel_->InvokeMethod("longEscape", nullptr);
    return 0;
  }
  // Give Flutter, including plugins, an opportunity to handle window messages.
  if (flutter_controller_) {
    std::optional<LRESULT> result =
        flutter_controller_->HandleTopLevelWindowProc(hwnd, message, wparam,
                                                      lparam);
    if (result) {
      return *result;
    }
  }

  switch (message) {
    case WM_FONTCHANGE:
      flutter_controller_->engine()->ReloadSystemFonts();
      break;
  }

  return Win32Window::MessageHandler(hwnd, message, wparam, lparam);
}
